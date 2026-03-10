//! CDDSの動作確認向けCLIツール
//!
//! Phase 1では `ls` に以下を実装する。
//! - Builtin readerを使ったTopic検出
//! - Topic単位のpub/sub集約
//! - 内部トピック(DCPS*)除外
//! - QoS詳細表示

use cyclonedds_rs::dds_builtin::{BuiltinDataReader, BuiltinSamples, Publications, Subscriptions};
use cyclonedds_rs::untyped::Untyped;
use cyclonedds_rs::{
    DDSError, DdsParticipant, DdsQos, DdsReader, DdsSubscriber, DdsTopic, SampleBuffer,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Display;
use std::io::{self, Write};
use std::process;
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_DOMAIN_ID: u32 = 0;
const DEFAULT_SCAN_MS: u64 = 1_000;
const DEFAULT_INTERVAL_MS: u64 = 1_000;
const TOP_READ_BUFFER_SIZE: usize = 1024;
const TOP_LOOP_SLEEP_MS: u64 = 50;
const TOP_STATUS_SKIP_PREFIX: &str = "skip:";

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliConfig {
    domain_id: u32,
    command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Ls(LsArgs),
    Top(TopArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LsArgs {
    scan_ms: u64,
    include_internal: bool,
}

impl Default for LsArgs {
    fn default() -> Self {
        Self {
            scan_ms: DEFAULT_SCAN_MS,
            include_internal: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TopArgs {
    scan_ms: u64,
    interval_ms: u64,
    include_internal: bool,
}

impl Default for TopArgs {
    fn default() -> Self {
        Self {
            scan_ms: DEFAULT_SCAN_MS,
            interval_ms: DEFAULT_INTERVAL_MS,
            include_internal: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseError {
    Invalid(String),
    Help(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointKind {
    Publication,
    Subscription,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EndpointState {
    kind: EndpointKind,
    topic_name: String,
    type_name: String,
    qos_detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TopicKey {
    topic_name: String,
    type_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TopicSummary {
    pub_count: usize,
    sub_count: usize,
    pub_qos_details: BTreeSet<String>,
    sub_qos_details: BTreeSet<String>,
}

struct TopReader {
    reader: DdsReader<Untyped>,
    _topic: DdsTopic<Untyped>,
    buffer: SampleBuffer<Untyped>,
}

struct TopTopicState {
    pub_count: usize,
    sub_count: usize,
    msg_total: u64,
    bytes_total: u64,
    msg_rate: f64,
    bytes_rate: f64,
    last_msg_total: u64,
    last_bytes_total: u64,
    reader: Option<TopReader>,
    create_error: Option<String>,
    read_error: Option<String>,
}

impl TopTopicState {
    fn new(pub_count: usize, sub_count: usize) -> Self {
        Self {
            pub_count,
            sub_count,
            msg_total: 0,
            bytes_total: 0,
            msg_rate: 0.0,
            bytes_rate: 0.0,
            last_msg_total: 0,
            last_bytes_total: 0,
            reader: None,
            create_error: None,
            read_error: None,
        }
    }
}

fn main() {
    let args = std::env::args().skip(1);
    let cli = match parse_cli(args) {
        Ok(cli) => cli,
        Err(ParseError::Help(help)) => {
            println!("{help}");
            return;
        }
        Err(ParseError::Invalid(msg)) => {
            eprintln!("引数エラー: {msg}\n\n{}", main_help());
            process::exit(2);
        }
    };

    if let Err(msg) = run(cli) {
        eprintln!("実行エラー: {msg}");
        process::exit(1);
    }
}

fn run(cli: CliConfig) -> Result<(), String> {
    match cli.command {
        Command::Ls(args) => run_ls(cli.domain_id, args),
        Command::Top(args) => run_top(cli.domain_id, args),
    }
}

fn run_ls(domain_id: u32, args: LsArgs) -> Result<(), String> {
    if args.scan_ms == 0 {
        return Err("--scan-ms は1以上を指定してください".to_string());
    }

    let endpoint_states = scan_endpoint_states(domain_id, args.scan_ms)?;
    let topic_summaries = summarize_topics(&endpoint_states, args.include_internal);

    println!(
        "domain={}, scan-ms={}ms, include-internal={}, endpoints={}, topics={}",
        domain_id,
        args.scan_ms,
        args.include_internal,
        endpoint_states.len(),
        topic_summaries.len()
    );

    if topic_summaries.is_empty() {
        println!("対象Topicは見つかりませんでした。");
        return Ok(());
    }

    let headers = ["topic", "type", "pub", "sub", "qos(detail)"];
    let mut rows = Vec::with_capacity(topic_summaries.len());
    for (topic_key, summary) in topic_summaries {
        rows.push(vec![
            topic_key.topic_name,
            topic_key.type_name,
            summary.pub_count.to_string(),
            summary.sub_count.to_string(),
            format_qos_cell(&summary),
        ]);
    }
    println!("{}", render_table(&headers, &rows));
    Ok(())
}

fn scan_endpoint_states(
    domain_id: u32,
    scan_ms: u64,
) -> Result<HashMap<Uuid, EndpointState>, String> {
    let participant = DdsParticipant::create(Some(domain_id), None, None)
        .map_err(|err| explain_participant_create_error(domain_id, err))?;

    let publication_reader = BuiltinDataReader::<Publications>::create(&participant, None)
        .map_err(|err| explain_builtin_reader_create_error("Publications", err))?;
    let subscription_reader = BuiltinDataReader::<Subscriptions>::create(&participant, None)
        .map_err(|err| explain_builtin_reader_create_error("Subscriptions", err))?;

    let mut publication_samples = BuiltinSamples::<Publications>::new(256);
    let mut subscription_samples = BuiltinSamples::<Subscriptions>::new(256);

    let mut endpoint_states: HashMap<Uuid, EndpointState> = HashMap::new();
    let scan_window = Duration::from_millis(scan_ms);
    let started = Instant::now();

    loop {
        collect_publications(
            &publication_reader,
            &mut publication_samples,
            &mut endpoint_states,
        )?;
        collect_subscriptions(
            &subscription_reader,
            &mut subscription_samples,
            &mut endpoint_states,
        )?;

        if started.elapsed() >= scan_window {
            break;
        }

        let remaining = scan_window.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }

    Ok(endpoint_states)
}

fn collect_publications(
    reader: &BuiltinDataReader<Publications>,
    samples: &mut BuiltinSamples<Publications>,
    endpoint_states: &mut HashMap<Uuid, EndpointState>,
) -> Result<(), String> {
    match reader.take_now(samples) {
        Ok(_) => {
            for sample in samples.iter() {
                apply_endpoint_event(
                    sample.guid(),
                    EndpointKind::Publication,
                    sample.is_alive(),
                    sample
                        .name()
                        .map(|name| name.to_string_lossy().into_owned()),
                    sample
                        .type_name()
                        .map(|type_name| type_name.to_string_lossy().into_owned()),
                    sample.qos().map(|qos| format_qos_detail(&qos)),
                    endpoint_states,
                );
            }
            Ok(())
        }
        Err(DDSError::NoData) => Ok(()),
        Err(err) => Err(explain_builtin_take_error("Publications", err)),
    }
}

fn collect_subscriptions(
    reader: &BuiltinDataReader<Subscriptions>,
    samples: &mut BuiltinSamples<Subscriptions>,
    endpoint_states: &mut HashMap<Uuid, EndpointState>,
) -> Result<(), String> {
    match reader.take_now(samples) {
        Ok(_) => {
            for sample in samples.iter() {
                apply_endpoint_event(
                    sample.guid(),
                    EndpointKind::Subscription,
                    sample.is_alive(),
                    sample
                        .name()
                        .map(|name| name.to_string_lossy().into_owned()),
                    sample
                        .type_name()
                        .map(|type_name| type_name.to_string_lossy().into_owned()),
                    sample.qos().map(|qos| format_qos_detail(&qos)),
                    endpoint_states,
                );
            }
            Ok(())
        }
        Err(DDSError::NoData) => Ok(()),
        Err(err) => Err(explain_builtin_take_error("Subscriptions", err)),
    }
}

fn apply_endpoint_event(
    guid: Uuid,
    kind: EndpointKind,
    is_alive: bool,
    topic_name: Option<String>,
    type_name: Option<String>,
    qos_detail: Option<String>,
    endpoint_states: &mut HashMap<Uuid, EndpointState>,
) {
    if !is_alive {
        endpoint_states.remove(&guid);
        return;
    }

    endpoint_states.insert(
        guid,
        EndpointState {
            kind,
            topic_name: topic_name.unwrap_or_else(|| "<unknown-topic>".to_string()),
            type_name: type_name.unwrap_or_else(|| "<unknown-type>".to_string()),
            qos_detail: qos_detail.unwrap_or_else(|| "qos-unavailable".to_string()),
        },
    );
}

fn summarize_topics(
    endpoint_states: &HashMap<Uuid, EndpointState>,
    include_internal: bool,
) -> BTreeMap<TopicKey, TopicSummary> {
    let mut summaries = BTreeMap::<TopicKey, TopicSummary>::new();

    for endpoint in endpoint_states.values() {
        if !include_internal && is_internal_topic(&endpoint.topic_name) {
            continue;
        }

        let key = TopicKey {
            topic_name: endpoint.topic_name.clone(),
            type_name: endpoint.type_name.clone(),
        };
        let summary = summaries.entry(key).or_default();
        match endpoint.kind {
            EndpointKind::Publication => {
                summary.pub_count += 1;
                summary.pub_qos_details.insert(endpoint.qos_detail.clone());
            }
            EndpointKind::Subscription => {
                summary.sub_count += 1;
                summary.sub_qos_details.insert(endpoint.qos_detail.clone());
            }
        }
    }

    summaries
}

fn is_internal_topic(topic_name: &str) -> bool {
    topic_name.starts_with("DCPS")
}

fn format_qos_cell(summary: &TopicSummary) -> String {
    let pub_side = format_qos_side("pub", &summary.pub_qos_details);
    let sub_side = format_qos_side("sub", &summary.sub_qos_details);
    format!("{pub_side}\n{sub_side}")
}

fn format_qos_side(label: &str, values: &BTreeSet<String>) -> String {
    match values.len() {
        0 => format!("{label}: -"),
        1 => {
            let value = values.iter().next().expect("set length already checked");
            format!("{label}:\n{}", indent_lines(value, "  "))
        }
        _ => {
            let mut out = format!("{label}[{}]:", values.len());
            for (index, value) in values.iter().enumerate() {
                out.push('\n');
                out.push_str(&format!("  profile#{}:", index + 1));
                out.push('\n');
                out.push_str(&indent_lines(value, "    "));
            }
            out
        }
    }
}

fn format_qos_detail(qos: &DdsQos) -> String {
    let durability = qos.durability();
    let (history_kind, history_depth) = qos.history();
    let (reliability_kind, reliability_duration) = qos.reliability();
    let lifespan = qos.lifespan();
    let deadline = qos.deadline();
    let (liveliness_kind, lease_duration) = qos.liveliness();

    format!(
        "dur={durability:?}\nhist={history_kind:?}({history_depth})\nrel={reliability_kind:?}({})\nlifespan={}\ndeadline={}\nliveliness={liveliness_kind:?}({})",
        format_duration(reliability_duration),
        format_duration(lifespan),
        format_duration(deadline),
        format_duration(lease_duration)
    )
}

fn indent_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_duration(duration: Duration) -> String {
    if duration.is_zero() {
        return "0s".to_string();
    }

    if duration.as_secs() > 0 {
        return format!("{:.3}s", duration.as_secs_f64());
    }
    if duration.as_millis() > 0 {
        return format!("{}ms", duration.as_millis());
    }
    if duration.as_micros() > 0 {
        return format!("{}us", duration.as_micros());
    }
    format!("{}ns", duration.as_nanos())
}

fn run_top(domain_id: u32, args: TopArgs) -> Result<(), String> {
    if args.scan_ms == 0 {
        return Err("--scan-ms は1以上を指定してください".to_string());
    }
    if args.interval_ms == 0 {
        return Err("--interval-ms は1以上を指定してください".to_string());
    }

    let participant = DdsParticipant::create(Some(domain_id), None, None)
        .map_err(|err| explain_participant_create_error(domain_id, err))?;
    let subscriber =
        DdsSubscriber::create(&participant, None, None).map_err(explain_subscriber_create_error)?;

    let scan_interval = Duration::from_millis(args.scan_ms);
    let render_interval = Duration::from_millis(args.interval_ms);
    let mut tracked_topics: BTreeMap<TopicKey, TopTopicState> = BTreeMap::new();

    let mut last_scan = Instant::now()
        .checked_sub(scan_interval)
        .unwrap_or_else(Instant::now);
    let mut last_render = Instant::now()
        .checked_sub(render_interval)
        .unwrap_or_else(Instant::now);
    let mut last_rate_update = Instant::now();

    loop {
        let now = Instant::now();

        if now.duration_since(last_scan) >= scan_interval {
            let endpoint_states = scan_endpoint_states(domain_id, args.scan_ms)?;
            let summaries = summarize_topics(&endpoint_states, args.include_internal);
            sync_top_topics(&mut tracked_topics, &summaries, &participant, &subscriber);
            last_scan = now;
        }

        poll_top_readers(&mut tracked_topics);

        let now = Instant::now();
        if now.duration_since(last_render) >= render_interval {
            let elapsed = now.duration_since(last_rate_update);
            update_top_rates(&mut tracked_topics, elapsed);
            render_top_table(domain_id, &args, &tracked_topics);

            last_rate_update = now;
            last_render = now;
        }

        thread::sleep(Duration::from_millis(TOP_LOOP_SLEEP_MS));
    }
}

fn sync_top_topics(
    tracked_topics: &mut BTreeMap<TopicKey, TopTopicState>,
    summaries: &BTreeMap<TopicKey, TopicSummary>,
    participant: &DdsParticipant,
    subscriber: &DdsSubscriber,
) {
    tracked_topics.retain(|topic_key, _| summaries.contains_key(topic_key));

    for (topic_key, summary) in summaries {
        let state = tracked_topics
            .entry(topic_key.clone())
            .or_insert_with(|| TopTopicState::new(summary.pub_count, summary.sub_count));

        state.pub_count = summary.pub_count;
        state.sub_count = summary.sub_count;

        if state.reader.is_none() && state.create_error.is_none() {
            if is_internal_topic(&topic_key.topic_name) {
                state.create_error =
                    Some("skip: 内部トピックはtopで受信統計を取得できません".to_string());
                continue;
            }

            match create_top_reader(participant, subscriber, topic_key) {
                Ok(reader) => {
                    state.reader = Some(reader);
                }
                Err(err) => {
                    eprintln!(
                        "warning: reader作成失敗 topic='{}' type='{}': {}",
                        topic_key.topic_name, topic_key.type_name, err
                    );
                    state.create_error = Some(err);
                }
            }
        }
    }
}

fn create_top_reader(
    participant: &DdsParticipant,
    subscriber: &DdsSubscriber,
    topic_key: &TopicKey,
) -> Result<TopReader, String> {
    let topic = DdsTopic::<Untyped>::create_untyped(
        participant,
        &topic_key.topic_name,
        &topic_key.type_name,
        None,
        None,
    )
    .map_err(|err| explain_untyped_topic_error(topic_key, err))?;

    let reader = DdsReader::<Untyped>::create(subscriber, topic.clone(), None, None)
        .map_err(|err| explain_untyped_reader_error(topic_key, err))?;

    Ok(TopReader {
        reader,
        _topic: topic,
        buffer: SampleBuffer::<Untyped>::new(TOP_READ_BUFFER_SIZE),
    })
}

fn poll_top_readers(tracked_topics: &mut BTreeMap<TopicKey, TopTopicState>) {
    for (topic_key, state) in tracked_topics.iter_mut() {
        let Some(reader) = state.reader.as_mut() else {
            continue;
        };

        match reader.reader.takecdr_now(&mut reader.buffer) {
            Ok(_) => {
                let mut msg_delta = 0u64;
                let mut bytes_delta = 0u64;
                for sample in reader.buffer.iter_sample() {
                    msg_delta += 1;
                    bytes_delta += sample.cdr().map_or(0, |cdr| cdr.len() as u64);
                }
                state.msg_total += msg_delta;
                state.bytes_total += bytes_delta;
                reader.buffer.clear();
            }
            Err(DDSError::NoData) => {}
            Err(err) => {
                let err_text = err.to_string();
                if state.read_error.as_deref() != Some(err_text.as_str()) {
                    eprintln!(
                        "warning: 読み取り失敗 topic='{}' type='{}': {}",
                        topic_key.topic_name, topic_key.type_name, err_text
                    );
                }
                state.read_error = Some(err_text);
            }
        }
    }
}

fn update_top_rates(tracked_topics: &mut BTreeMap<TopicKey, TopTopicState>, elapsed: Duration) {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return;
    }

    for state in tracked_topics.values_mut() {
        let msg_delta = state.msg_total.saturating_sub(state.last_msg_total);
        let bytes_delta = state.bytes_total.saturating_sub(state.last_bytes_total);

        state.msg_rate = msg_delta as f64 / seconds;
        state.bytes_rate = bytes_delta as f64 / seconds;

        state.last_msg_total = state.msg_total;
        state.last_bytes_total = state.bytes_total;
    }
}

fn render_top_table(
    domain_id: u32,
    args: &TopArgs,
    tracked_topics: &BTreeMap<TopicKey, TopTopicState>,
) {
    print!("\x1B[2J\x1B[H");
    println!(
        "cdds-cli top domain={} interval={}ms scan={}ms include-internal={}",
        domain_id, args.interval_ms, args.scan_ms, args.include_internal
    );
    println!("Ctrl+C で終了");

    if tracked_topics.is_empty() {
        println!("\n監視対象Topicはありません。");
        let _ = io::stdout().flush();
        return;
    }

    let headers = [
        "topic", "type", "pub", "sub", "msgs", "bytes", "msg/s", "bytes/s", "status",
    ];
    let mut rows = Vec::with_capacity(tracked_topics.len());
    for (topic_key, state) in tracked_topics {
        let status = format_top_status(state);

        rows.push(vec![
            topic_key.topic_name.clone(),
            topic_key.type_name.clone(),
            state.pub_count.to_string(),
            state.sub_count.to_string(),
            state.msg_total.to_string(),
            format_bytes(state.bytes_total),
            format!("{:.1}", state.msg_rate),
            format_bytes_rate(state.bytes_rate),
            status,
        ]);
    }
    println!("\n{}", render_table(&headers, &rows));
    let _ = io::stdout().flush();
}

fn format_bytes_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec <= 0.0 {
        return "0 B/s".to_string();
    }
    format!("{}/s", format_bytes(bytes_per_sec.round() as u64))
}

fn truncate_text(text: &str, max_len: usize) -> String {
    let count = text.chars().count();
    if count <= max_len {
        return text.to_string();
    }

    if max_len <= 3 {
        return text.chars().take(max_len).collect();
    }
    let mut out = text.chars().take(max_len - 3).collect::<String>();
    out.push_str("...");
    out
}

fn format_top_status(state: &TopTopicState) -> String {
    if let Some(err) = &state.create_error {
        if let Some(reason) = err.strip_prefix(TOP_STATUS_SKIP_PREFIX) {
            return format!("skip: {}", truncate_text(reason.trim(), 48));
        }
        return format!("reader-ng: {}", truncate_text(err, 48));
    }

    if let Some(err) = &state.read_error {
        return format!("read-ng: {}", truncate_text(err, 48));
    }

    "ok".to_string()
}

fn explain_participant_create_error(domain_id: u32, err: DDSError) -> String {
    let hint = match err {
        DDSError::BadParameter => "domain IDまたはCycloneDDS設定(cyclonedds.xml)を確認してください",
        DDSError::NotAllowedBySecurity => "Security設定によりDomain参加が拒否されました",
        DDSError::OutOfResources => "OSリソース不足の可能性があります",
        _ => "CycloneDDSの起動状態と設定を確認してください",
    };

    format!(
        "DomainParticipant作成に失敗しました (domain={}): {} ({})",
        domain_id, err, hint
    )
}

fn explain_subscriber_create_error(err: DDSError) -> String {
    let hint = match err {
        DDSError::PreconditionNotMet => "DomainParticipantの初期化状態を確認してください",
        DDSError::NotAllowedBySecurity => "Security設定によりSubscriber作成が拒否されました",
        _ => "CycloneDDS設定を確認してください",
    };

    format!("Subscriber作成に失敗しました: {} ({})", err, hint)
}

fn explain_builtin_reader_create_error(reader_kind: &str, err: DDSError) -> String {
    let hint = match err {
        DDSError::Unsupported => "使用中のCycloneDDSがBuiltin readerをサポートしていません",
        DDSError::NotAllowedBySecurity => "Security設定によりBuiltin reader作成が拒否されました",
        _ => "Domain参加直後のタイミングや設定を確認してください",
    };

    format!(
        "{} reader作成に失敗しました: {} ({})",
        reader_kind, err, hint
    )
}

fn explain_builtin_take_error(reader_kind: &str, err: DDSError) -> String {
    let hint = match err {
        DDSError::IllegalOperation => "reader状態が不正の可能性があります",
        DDSError::Timeout => "scan-msを大きくして再試行してください",
        _ => "CycloneDDSの通信状態を確認してください",
    };

    format!("{} 読み取りに失敗しました: {} ({})", reader_kind, err, hint)
}

fn explain_untyped_topic_error(topic_key: &TopicKey, err: DDSError) -> String {
    let hint = match err {
        DDSError::BadParameter => "型情報が一致しないか、このTopicはtop集計に非対応です",
        DDSError::Unsupported => "この型はCDR直接購読に対応していません",
        DDSError::NotAllowedBySecurity => "Security設定によりTopic購読が拒否されました",
        _ => "Topic/type情報の整合性を確認してください",
    };

    format!(
        "untyped topic作成に失敗 (topic='{}', type='{}'): {} ({})",
        topic_key.topic_name, topic_key.type_name, err, hint
    )
}

fn explain_untyped_reader_error(topic_key: &TopicKey, err: DDSError) -> String {
    let hint = match err {
        DDSError::PreconditionNotMet => "topicまたはsubscriberの初期化状態を確認してください",
        DDSError::OutOfResources => "reader数が多すぎる可能性があります",
        DDSError::NotAllowedBySecurity => "Security設定によりreader作成が拒否されました",
        _ => "QoS互換性や型情報の整合性を確認してください",
    };

    format!(
        "reader作成に失敗 (topic='{}', type='{}'): {} ({})",
        topic_key.topic_name, topic_key.type_name, err, hint
    )
}

fn parse_cli<I, S>(args: I) -> Result<CliConfig, ParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut domain_id = DEFAULT_DOMAIN_ID;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(ParseError::Help(main_help())),
            "--domain" => {
                domain_id = parse_u32_value("--domain", args.next())?;
            }
            "ls" => {
                let sub_args = parse_ls_args(args.collect())?;
                return Ok(CliConfig {
                    domain_id,
                    command: Command::Ls(sub_args),
                });
            }
            "top" => {
                let sub_args = parse_top_args(args.collect())?;
                return Ok(CliConfig {
                    domain_id,
                    command: Command::Top(sub_args),
                });
            }
            _ => {
                return Err(ParseError::Invalid(format!("不明な引数です: {arg}")));
            }
        }
    }

    Err(ParseError::Help(main_help()))
}

fn parse_ls_args(args: Vec<String>) -> Result<LsArgs, ParseError> {
    let mut options = LsArgs::default();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(ParseError::Help(ls_help())),
            "--scan-ms" => {
                options.scan_ms = parse_u64_value("--scan-ms", iter.next())?;
            }
            "--include-internal" => {
                options.include_internal = true;
            }
            _ => {
                return Err(ParseError::Invalid(format!("ls の不明な引数です: {arg}")));
            }
        }
    }

    Ok(options)
}

fn parse_top_args(args: Vec<String>) -> Result<TopArgs, ParseError> {
    let mut options = TopArgs::default();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(ParseError::Help(top_help())),
            "--scan-ms" => {
                options.scan_ms = parse_u64_value("--scan-ms", iter.next())?;
            }
            "--interval-ms" => {
                options.interval_ms = parse_u64_value("--interval-ms", iter.next())?;
            }
            "--include-internal" => {
                options.include_internal = true;
            }
            _ => {
                return Err(ParseError::Invalid(format!("top の不明な引数です: {arg}")));
            }
        }
    }

    Ok(options)
}

fn parse_u32_value(flag: &str, maybe_value: Option<String>) -> Result<u32, ParseError> {
    parse_value(flag, maybe_value)
}

fn parse_u64_value(flag: &str, maybe_value: Option<String>) -> Result<u64, ParseError> {
    parse_value(flag, maybe_value)
}

fn parse_value<T>(flag: &str, maybe_value: Option<String>) -> Result<T, ParseError>
where
    T: std::str::FromStr,
    T::Err: Display,
{
    let value = maybe_value.ok_or_else(|| ParseError::Invalid(format!("{flag} の値が必要です")))?;
    value
        .parse::<T>()
        .map_err(|e| ParseError::Invalid(format!("{flag} の値が不正です: {value} ({e})")))
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit_idx = 0usize;

    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{bytes} {}", UNITS[unit_idx])
    } else {
        format!("{value:.1} {}", UNITS[unit_idx])
    }
}

fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }

    let mut widths: Vec<usize> = headers.iter().map(|header| cell_width(header)).collect();
    for row in rows {
        for (index, width) in widths.iter_mut().enumerate() {
            let cell = row.get(index).map_or("", String::as_str);
            *width = (*width).max(cell_width(cell));
        }
    }

    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.extend(render_multiline_row(
        headers.iter().map(|header| header.to_string()).collect(),
        &widths,
    ));
    lines.push(
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-"),
    );

    for row in rows {
        let normalized_row = (0..headers.len())
            .map(|i| row.get(i).cloned().unwrap_or_default())
            .collect::<Vec<_>>();
        lines.extend(render_multiline_row(normalized_row, &widths));
    }

    lines.join("\n")
}

fn render_multiline_row(cells: Vec<String>, widths: &[usize]) -> Vec<String> {
    let split_cells = cells
        .into_iter()
        .map(|cell| {
            cell.split('\n')
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .collect::<Vec<_>>();

    let row_height = split_cells
        .iter()
        .map(|lines| lines.len())
        .max()
        .unwrap_or(1);
    let mut out = Vec::with_capacity(row_height);

    for line_index in 0..row_height {
        let line = split_cells
            .iter()
            .zip(widths.iter())
            .map(|(cell_lines, width)| {
                let cell = cell_lines
                    .get(line_index)
                    .map_or("", std::string::String::as_str);
                format!("{cell:<width$}", width = *width)
            })
            .collect::<Vec<_>>()
            .join(" | ");
        out.push(line);
    }

    out
}

fn cell_width(cell: &str) -> usize {
    cell.lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

fn main_help() -> String {
    format!(
        "cdds-cli: CycloneDDS動作確認CLI\n\nusage:\n  cdds-cli [--domain <u32>] <command> [options]\n\ncommands:\n  ls     Topic一覧とQoSを表示\n  top    Topicごとの通信統計を継続表示\n\noptions:\n  --domain <u32>    対象Domain ID (default: {DEFAULT_DOMAIN_ID})\n  -h, --help        ヘルプを表示\n\nsubcommand help:\n  cdds-cli ls --help\n  cdds-cli top --help"
    )
}

fn ls_help() -> String {
    format!(
        "usage:\n  cdds-cli [--domain <u32>] ls [--scan-ms <u64>] [--include-internal]\n\noptions:\n  --scan-ms <u64>         検索ウィンドウ[ms] (default: {DEFAULT_SCAN_MS})\n  --include-internal      内部トピック(DCPS*)を含める\n  -h, --help              ヘルプを表示"
    )
}

fn top_help() -> String {
    format!(
		"usage:\n  cdds-cli [--domain <u32>] top [--interval-ms <u64>] [--scan-ms <u64>] [--include-internal]\n\noptions:\n  --interval-ms <u64>     更新間隔[ms] (default: {DEFAULT_INTERVAL_MS})\n  --scan-ms <u64>         再探索間隔[ms] (default: {DEFAULT_SCAN_MS})\n  --include-internal      内部トピック(DCPS*)を含める\n  -h, --help              ヘルプを表示"
	)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_default_values() {
        let cli = parse_cli(["ls"]).expect("parse failed");
        assert_eq!(
            cli,
            CliConfig {
                domain_id: DEFAULT_DOMAIN_ID,
                command: Command::Ls(LsArgs::default()),
            }
        );
    }

    #[test]
    fn parse_top_with_all_flags() {
        let cli = parse_cli([
            "--domain",
            "7",
            "top",
            "--interval-ms",
            "500",
            "--scan-ms",
            "1200",
            "--include-internal",
        ])
        .expect("parse failed");

        assert_eq!(
            cli,
            CliConfig {
                domain_id: 7,
                command: Command::Top(TopArgs {
                    scan_ms: 1200,
                    interval_ms: 500,
                    include_internal: true,
                }),
            }
        );
    }

    #[test]
    fn format_bytes_works() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1_024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
    }

    #[test]
    fn table_render_has_header_separator_and_rows() {
        let table = render_table(
            &["name", "count"],
            &[vec!["topic/a".to_string(), "12".to_string()]],
        );
        assert!(table.contains("name"));
        assert!(table.contains("count"));
        assert!(table.contains("topic/a"));
        assert!(table.contains("-+-"));
    }

    #[test]
    fn table_render_supports_multiline_cells() {
        let table = render_table(
            &["topic", "qos"],
            &[vec![
                "/demo/topic".to_string(),
                "pub:\n  dur=VOLATILE\nsub:\n  dur=TRANSIENT_LOCAL".to_string(),
            ]],
        );

        assert!(table.contains("/demo/topic"));
        assert!(table.contains("pub:"));
        assert!(table.contains("dur=VOLATILE"));
        assert!(table.contains("sub:"));
    }

    #[test]
    fn internal_topic_filter_works() {
        assert!(is_internal_topic("DCPSPublication"));
        assert!(!is_internal_topic("/my/topic"));
    }

    #[test]
    fn summarize_topics_respects_internal_flag() {
        let mut states = HashMap::new();
        states.insert(
            Uuid::from_u128(1),
            EndpointState {
                kind: EndpointKind::Publication,
                topic_name: "DCPSPublication".to_string(),
                type_name: "BuiltinType".to_string(),
                qos_detail: "q1".to_string(),
            },
        );
        states.insert(
            Uuid::from_u128(2),
            EndpointState {
                kind: EndpointKind::Subscription,
                topic_name: "/demo/chatter".to_string(),
                type_name: "std_msgs/String".to_string(),
                qos_detail: "q2".to_string(),
            },
        );

        let filtered = summarize_topics(&states, false);
        assert_eq!(filtered.len(), 1);
        assert!(filtered.contains_key(&TopicKey {
            topic_name: "/demo/chatter".to_string(),
            type_name: "std_msgs/String".to_string(),
        }));

        let included = summarize_topics(&states, true);
        assert_eq!(included.len(), 2);
    }

    #[test]
    fn qos_cell_formats_multiple_values() {
        let mut summary = TopicSummary::default();
        summary.pub_qos_details.insert("a".to_string());
        summary.pub_qos_details.insert("b".to_string());
        summary.sub_qos_details.insert("x".to_string());

        let qos_cell = format_qos_cell(&summary);
        assert!(qos_cell.contains("pub[2]:"));
        assert!(qos_cell.contains("profile#1"));
        assert!(qos_cell.contains("sub:"));
    }

    #[test]
    fn format_bytes_rate_works() {
        assert_eq!(format_bytes_rate(0.0), "0 B/s");
        assert_eq!(format_bytes_rate(1024.0), "1.0 KiB/s");
    }

    #[test]
    fn truncate_text_works() {
        assert_eq!(truncate_text("abc", 8), "abc");
        assert_eq!(truncate_text("abcdefghij", 6), "abc...");
    }

    #[test]
    fn update_top_rates_uses_delta() {
        let mut tracked = BTreeMap::new();
        tracked.insert(
            TopicKey {
                topic_name: "/demo/topic".to_string(),
                type_name: "demo/type".to_string(),
            },
            TopTopicState {
                pub_count: 1,
                sub_count: 1,
                msg_total: 140,
                bytes_total: 2_200,
                msg_rate: 0.0,
                bytes_rate: 0.0,
                last_msg_total: 100,
                last_bytes_total: 1_000,
                reader: None,
                create_error: None,
                read_error: None,
            },
        );

        update_top_rates(&mut tracked, Duration::from_secs(2));
        let state = tracked
            .values()
            .next()
            .expect("state should exist after update");

        assert!((state.msg_rate - 20.0).abs() < 1e-9);
        assert!((state.bytes_rate - 600.0).abs() < 1e-9);
        assert_eq!(state.last_msg_total, 140);
        assert_eq!(state.last_bytes_total, 2_200);
    }

    #[test]
    fn top_status_formats_skip_reason() {
        let state = TopTopicState {
            pub_count: 0,
            sub_count: 0,
            msg_total: 0,
            bytes_total: 0,
            msg_rate: 0.0,
            bytes_rate: 0.0,
            last_msg_total: 0,
            last_bytes_total: 0,
            reader: None,
            create_error: Some(format!("{} internal", TOP_STATUS_SKIP_PREFIX)),
            read_error: None,
        };

        assert_eq!(format_top_status(&state), "skip: internal");
    }

    #[test]
    fn participant_error_message_contains_hint() {
        let message = explain_participant_create_error(0, DDSError::BadParameter);
        assert!(message.contains("DomainParticipant作成に失敗"));
        assert!(message.contains("domain ID"));
    }
}
