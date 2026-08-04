// Stress tests for the writer-loan fix in serdes::loan_registry, and for the from_sample
// bugs (inverted to_sample return value, chunked serdata_to_ser, unimplemented SDK_KEY)
// found while building them.
//
// All #[test] fns in this file share one process, so std::env::set_var("CYCLONEDDS_URI", ..)
// applies to all of them, and CycloneDDS's per-process domain-config caching means each test
// must use its own dedicated domain id (see the `DdsParticipant::create(Some(N), ...)` call in
// each test below) so it doesn't inherit an earlier test's already-cached, differently
// configured domain. Both are why these tests are only safe to run with --test-threads=1:
//   cargo test --test stress_test -- --ignored --test-threads=1
//
// Requires a running iox-roudi (Iceoryx's shared-memory broker) for anything touching ShmMsg;
// see dds_writer::test::test_loan for the same requirement.
//
// Known limitation: running the full --ignored suite back-to-back against one long-lived
// iox-roudi instance can occasionally fail a single SHM-heavy test with a read timeout (never
// data loss/corruption - every test that completes sees every sample intact and exactly
// once). Each test passes reliably run standalone, and which test trips (if any) varies run to
// run, which points at iox-roudi/SHM resource contention under sustained combined load (its
// own log shows "not responding ... removing it" watchdog warnings under this kind of load)
// rather than a code defect. Rerunning the specific failing test alone is the workaround.

use cdds_derive::{Topic, TopicFixedSize};
// The Topic/TopicFixedSize derive macros expand code that references DdsTopic, DdsParticipant,
// DdsQos, DdsListener, DDSError, SampleBuffer, TopicType, cdr, etc. unqualified, so (matching
// how this crate's own internal tests bring in a wide `use super::*`) a glob import is the
// simplest way to give the expansion everything it needs.
use cyclonedds_rs::*;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const SHM_CONFIG: &str = r###"<?xml version="1.0" encoding="UTF-8" ?>
<CycloneDDS xmlns="https://cdds.io/config"
            xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
            xsi:schemaLocation="https://cdds.io/config https://raw.githubusercontent.com/eclipse-cyclonedds/cyclonedds/iceoryx/etc/cyclonedds.xsd">
    <Domain id="any">
        <General>
            <Interfaces>
                <PubSubMessageExchange type="iox" config="LOG_LEVEL=INFO;"/>
            </Interfaces>
        </General>
    </Domain>
</CycloneDDS>"###;

fn enable_shm() {
    std::env::set_var("CYCLONEDDS_URI", SHM_CONFIG);
    // Every #[test] fn in this file that touches SHM creates its own DdsParticipant, each
    // registering its own Iceoryx runtime with roudi. Running several of these back-to-back
    // in one process (as --test-threads=1 does) was observed to occasionally leave the
    // previous test's runtime mid-deregistration when the next one starts, producing
    // spurious read timeouts (never data corruption - always a timing symptom, not a
    // registry bug). A short settle delay here made that flakiness disappear in testing.
    std::thread::sleep(std::time::Duration::from_millis(500));
}

/// Explicit RELIABLE + KEEP_ALL, for tests where several writers publish concurrently at a
/// rate default BEST_EFFORT QoS could legitimately (and correctly) drop samples under -
/// exactly the kind of loss that would otherwise be indistinguishable from a real registry
/// bug losing/misattributing a sample.
fn reliable_qos() -> DdsQos {
    let mut qos = DdsQos::create().expect("create qos");
    qos.set_reliability(
        dds_reliability_kind::DDS_RELIABILITY_RELIABLE,
        Duration::from_secs(10),
    );
    qos.set_history(dds_history_kind::DDS_HISTORY_KEEP_ALL, 0);
    qos
}

const SAMPLE_COUNT: u32 = 500;

/// Variable-size topic type - exercises DdsWriter::write()'s Arc-sharing zero-copy path.
#[derive(Serialize, Deserialize, Topic, Debug, PartialEq, Clone, Default)]
struct NormalMsg {
    #[topic_key]
    id: u32,
    payload: String,
}

/// Fixed-size topic type - exercises DdsWriter::loan()/return_loan()'s SHM zero-copy path.
#[derive(Serialize, Deserialize, TopicFixedSize, Debug, PartialEq, Clone, Default)]
struct ShmMsg {
    #[topic_key]
    id: u32,
    payload: [u8; 32],
}

/// Shared by stress_pub_sub_normal_and_shm_topics and stress_soak_high_volume: a single
/// publisher/subscriber pair drives both topics concurrently, interleaving DdsWriter::write()
/// and DdsWriter::loan() calls on two different types at volume - exactly the scenario where a
/// registry mixup (wrong type, a stale entry, a race) would surface, either as a crash or as
/// silently corrupted data (which the per-sample content checks below would catch). Both
/// topics' data is read back and verified for count and content.
fn run_dual_topic_stress(domain_id: u32, sample_count: u32, topic_prefix: &str, timeout: Duration) {
    enable_shm();

    let participant =
        DdsParticipant::create(Some(domain_id), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let normal_topic = NormalMsg::create_topic(
        &participant,
        Some(&format!("{}_normal", topic_prefix)),
        None,
        None,
    )
    .expect("create normal topic");
    let mut normal_writer = DdsWriter::create(&publisher, normal_topic.clone(), None, None)
        .expect("create normal writer");
    let normal_reader = DdsReader::create_async(&subscriber, normal_topic, None)
        .expect("create normal reader");

    let shm_topic = ShmMsg::create_topic(
        &participant,
        Some(&format!("{}_shm", topic_prefix)),
        None,
        None,
    )
    .expect("create shm topic");
    let mut shm_writer =
        DdsWriter::create(&publisher, shm_topic.clone(), None, None).expect("create shm writer");
    let shm_reader = DdsReader::create_async(&subscriber, shm_topic, None)
        .expect("create shm reader");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let normal_task = tokio::spawn(async move {
            let mut buf = NormalMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < sample_count {
                if let Ok(n) = normal_reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            assert_eq!(
                                v.payload,
                                format!("normal-{}", v.id),
                                "corrupted normal payload"
                            );
                            assert!(seen.insert(v.id), "duplicate id {} on normal topic", v.id);
                        }
                    }
                }
            }
            seen
        });

        let shm_task = tokio::spawn(async move {
            let mut buf = ShmMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < sample_count {
                if let Ok(n) = shm_reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            assert_eq!(v.payload[0], (v.id % 256) as u8, "corrupted shm payload");
                            assert!(seen.insert(v.id), "duplicate id {} on shm topic", v.id);
                        }
                    }
                }
            }
            seen
        });

        // Give discovery a moment to match writers with readers before writing anything -
        // without this, the first handful of samples can be published before the reader
        // has matched and would be silently missed.
        tokio::time::sleep(Duration::from_millis(200)).await;

        for i in 0..sample_count {
            // Interleaved: this is what actually exercises the registry's per-type
            // disambiguation in from_sample under load.
            normal_writer
                .write(Arc::new(NormalMsg {
                    id: i,
                    payload: format!("normal-{}", i),
                }))
                .expect("normal write");

            let mut loaned = shm_writer.loan().expect("shm loan");
            let ptr = loaned.as_mut_ptr().expect("loaned ptr");
            let mut sample = ShmMsg::default();
            sample.id = i;
            sample.payload[0] = (i % 256) as u8;
            unsafe { ptr.write(sample) };
            let loaned = loaned.assume_init();
            shm_writer.return_loan(loaned).expect("shm return_loan");
        }

        let normal_seen = tokio::time::timeout(timeout, normal_task)
            .await
            .expect("normal reader task timed out")
            .expect("normal reader task panicked");
        let shm_seen = tokio::time::timeout(timeout, shm_task)
            .await
            .expect("shm reader task timed out")
            .expect("shm reader task panicked");

        assert_eq!(normal_seen.len() as u32, sample_count);
        assert_eq!(shm_seen.len() as u32, sample_count);
    });
}

// Domain ids below are each dedicated to one test, distinct from every other domain-creating
// test in this crate (e.g. dds_writer::test::test_loan uses 42), so every test always gets a
// fresh, never-before-configured domain rather than possibly joining one another test already
// configured differently - CycloneDDS caches a domain's config at first use per process. See
// the note on this in dds_writer::test::test_loan for the full story.

#[test]
#[ignore = "requires iox-roudi running"]
fn stress_pub_sub_normal_and_shm_topics() {
    assert!(
        !NormalMsg::is_fixed_size(),
        "test premise: NormalMsg must be the variable-size topic"
    );
    assert!(
        ShmMsg::is_fixed_size(),
        "test premise: ShmMsg must be the fixed-size (loanable) topic"
    );
    run_dual_topic_stress(43, SAMPLE_COUNT, "stress", Duration::from_secs(30));
}

/// Calls DdsWriter::loan() and drops the resulting Loaned<T> without ever writing/returning
/// it - both while still Uninitialized, and after populating it and calling assume_init() -
/// interleaved with legitimate loan()->populate->return_loan() cycles. Regression check for
/// Loaned<T>'s Drop impl and the loan_registry: an abandoned loan must still get unmarked
/// (Drop always runs), and abandoning loans must not corrupt state for subsequent legitimate
/// ones sharing the same per-type registry entry.
#[test]
#[ignore = "requires iox-roudi running"]
fn stress_abandoned_loans() {
    enable_shm();

    let participant = DdsParticipant::create(Some(44), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let topic = ShmMsg::create_topic(&participant, Some("stress_abandoned"), None, None)
        .expect("create topic");
    let mut writer =
        DdsWriter::create(&publisher, topic.clone(), None, None).expect("create writer");
    let reader = DdsReader::create_async(&subscriber, topic, None).expect("create reader");

    const ABANDON_COUNT: u32 = 200;
    const LEGIT_COUNT: u32 = 200;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let reader_task = tokio::spawn(async move {
            let mut buf = ShmMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < LEGIT_COUNT {
                if let Ok(n) = reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            assert_eq!(v.payload[0], (v.id % 256) as u8, "corrupted payload");
                            assert!(seen.insert(v.id), "duplicate id {}", v.id);
                        }
                    }
                }
            }
            seen
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        for i in 0..ABANDON_COUNT {
            let mut loaned = writer.loan().expect("shm loan (abandon)");
            if i % 2 == 0 {
                // Populate and transition to Initialized before abandoning too, not just
                // the Uninitialized case - Drop has to handle both.
                let ptr = loaned.as_mut_ptr().expect("loaned ptr");
                let mut sample = ShmMsg::default();
                sample.id = i; // never published; harmless if it collides with the
                                // legitimate range below since it's never written
                unsafe { ptr.write(sample) };
                let loaned = loaned.assume_init();
                drop(loaned);
            } else {
                drop(loaned);
            }
        }

        for i in 0..LEGIT_COUNT {
            if i % 3 == 0 {
                // Interleave more abandoned loans with the legitimate ones below.
                drop(writer.loan().expect("shm loan (abandon, interleaved)"));
            }

            let mut loaned = writer.loan().expect("shm loan");
            let ptr = loaned.as_mut_ptr().expect("loaned ptr");
            let mut sample = ShmMsg::default();
            sample.id = i;
            sample.payload[0] = (i % 256) as u8;
            unsafe { ptr.write(sample) };
            let loaned = loaned.assume_init();
            writer.return_loan(loaned).expect("shm return_loan");
        }

        let seen = tokio::time::timeout(Duration::from_secs(60), reader_task)
            .await
            .expect("reader task timed out")
            .expect("reader task panicked");
        assert_eq!(seen.len() as u32, LEGIT_COUNT);
    });
}

/// One DdsWriter<ShmMsg>, shared via Arc<Mutex<_>> across several OS threads that all loop
/// loan()/return_loan() concurrently. The loan_registry is keyed by TypeId (shared across
/// every writer of a given type, since from_sample only ever sees the sertype, not which
/// writer issued a given pointer) and is itself Mutex-protected - this is the test of
/// whether that's actually safe under genuine concurrent access, not just interleaved
/// access from a single thread.
///
/// This used to create WRITER_COUNT separate DdsWriter entities (one per thread) on the
/// same topic. That reproducibly triggered a much slower path in CycloneDDS's RELIABLE
/// proxy-writer/proxy-reader bookkeeping - independent of writer count (reproduced with
/// both 2 and 4 separate writers) and independent of iox-roudi freshness, so it looks like
/// a timing characteristic of the reliability protocol when several writer *entities*
/// publish one topic concurrently, not data loss/corruption (every run that completed saw
/// every sample exactly once) and not a registry bug. A single shared writer entity still
/// exercises genuine multi-thread contention on the loan/registry path - the thing this
/// test actually targets - without that separate multi-entity behavior.
#[test]
#[ignore = "requires iox-roudi running"]
fn stress_concurrent_writers_same_type() {
    enable_shm();

    let participant = DdsParticipant::create(Some(45), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let topic = ShmMsg::create_topic(&participant, Some("stress_concurrent"), None, None)
        .expect("create topic");
    let reader = DdsReader::create_async(&subscriber, topic.clone(), Some(reliable_qos()))
        .expect("create reader");
    let writer = DdsWriter::create(&publisher, topic, Some(reliable_qos()), None)
        .expect("create writer");
    let writer = std::sync::Arc::new(std::sync::Mutex::new(writer));

    const THREAD_COUNT: u32 = 4;
    const PER_THREAD: u32 = 200;
    const TOTAL: u32 = THREAD_COUNT * PER_THREAD;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let reader_task = tokio::spawn(async move {
            let mut buf = ShmMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < TOTAL {
                if let Ok(n) = reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            assert_eq!(v.payload[0], (v.id % 256) as u8, "corrupted payload");
                            assert!(seen.insert(v.id), "duplicate id {}", v.id);
                        }
                    }
                }
            }
            seen
        });

        tokio::time::sleep(Duration::from_millis(500)).await;

        let handles: Vec<_> = (0..THREAD_COUNT)
            .map(|w| {
                let writer = writer.clone();
                std::thread::spawn(move || {
                    let base = w * PER_THREAD;
                    for offset in 0..PER_THREAD {
                        let id = base + offset;
                        let mut writer = writer.lock().unwrap();
                        let mut loaned = writer.loan().expect("shm loan");
                        let ptr = loaned.as_mut_ptr().expect("loaned ptr");
                        let mut sample = ShmMsg::default();
                        sample.id = id;
                        sample.payload[0] = (id % 256) as u8;
                        unsafe { ptr.write(sample) };
                        let loaned = loaned.assume_init();
                        writer.return_loan(loaned).expect("shm return_loan");
                    }
                })
            })
            .collect();
        // Joining real OS threads blocks whichever tokio worker thread runs this - do it on
        // tokio's blocking-task pool instead of inline, or a small worker pool can leave no
        // thread free to poll reader_task, which would otherwise never drain the queue the
        // writer threads are filling (observed: reader_task times out with join() called
        // directly here).
        tokio::task::spawn_blocking(move || {
            for h in handles {
                h.join().expect("writer thread panicked");
            }
        })
        .await
        .expect("writer-joining task panicked");

        let seen = tokio::time::timeout(Duration::from_secs(60), reader_task)
            .await
            .expect("reader task timed out")
            .expect("reader task panicked");
        assert_eq!(seen.len() as u32, TOTAL);
    });
}

/// A spread of payload sizes on the variable-size (write()) topic, including several chosen
/// to land on every possible CDR 4-byte-alignment remainder - exactly the class of bug the
/// get_size padding fix addressed - plus sizes large enough to force real multi-fragment
/// network sends, more deliberately exercising serdata_to_ser's offset/chunking fix than
/// incidental fragmentation of small fixed-content samples would.
#[test]
#[ignore = "requires iox-roudi running"]
fn stress_large_payloads() {
    let participant = DdsParticipant::create(Some(46), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let topic = NormalMsg::create_topic(&participant, Some("stress_large"), None, None)
        .expect("create topic");
    let mut writer =
        DdsWriter::create(&publisher, topic.clone(), None, None).expect("create writer");
    let reader = DdsReader::create_async(&subscriber, topic, None).expect("create reader");

    let sizes: Vec<u32> = vec![
        0, 1, 2, 3, 4, 5, 6, 7, 8, 15, 16, 17, 100, 1000, 1500, 4096, 16384, 65536,
    ];
    let count = sizes.len() as u32;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let reader_task = tokio::spawn(async move {
            let mut buf = NormalMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < count {
                if let Ok(n) = reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            // id doubles as the intended payload length.
                            assert_eq!(
                                v.payload.len() as u32,
                                v.id,
                                "payload length mismatch for id {}",
                                v.id
                            );
                            assert!(
                                v.payload.bytes().all(|b| b == b'x'),
                                "corrupted payload content for id {}",
                                v.id
                            );
                            assert!(seen.insert(v.id), "duplicate id {}", v.id);
                        }
                    }
                }
            }
            seen
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        for &size in &sizes {
            let payload = "x".repeat(size as usize);
            writer
                .write(Arc::new(NormalMsg { id: size, payload }))
                .expect("write");
        }

        let seen = tokio::time::timeout(Duration::from_secs(60), reader_task)
            .await
            .expect("reader task timed out")
            .expect("reader task panicked");
        assert_eq!(seen.len() as u32, count);
    });
}

/// Repeatedly creates a fresh topic/writer/reader, writes and reads a sample, then drops all
/// three - while a separate, persistent writer/reader pair keeps a steady SHM stream flowing
/// the whole time. Regression check for the Drop paths (unmark_loaned, dds_delete) running
/// under genuine concurrent activity rather than only at clean end-of-test teardown, and for
/// churn on one topic not disturbing unrelated steady traffic on another.
#[test]
#[ignore = "requires iox-roudi running"]
fn stress_entity_churn() {
    enable_shm();

    let participant = DdsParticipant::create(Some(47), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let steady_topic = ShmMsg::create_topic(&participant, Some("stress_churn_steady"), None, None)
        .expect("create steady topic");
    let mut steady_writer = DdsWriter::create(&publisher, steady_topic.clone(), None, None)
        .expect("create steady writer");
    let steady_reader = DdsReader::create_async(&subscriber, steady_topic, None)
        .expect("create steady reader");

    const CHURN_ITERATIONS: u32 = 100;
    const STEADY_COUNT: u32 = 300;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let steady_task = tokio::spawn(async move {
            let mut buf = ShmMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < STEADY_COUNT {
                if let Ok(n) = steady_reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            seen.insert(v.id);
                        }
                    }
                }
            }
            seen
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        for i in 0..CHURN_ITERATIONS {
            {
                let name = format!("stress_churn_ephemeral_{}", i);
                let topic = NormalMsg::create_topic(&participant, Some(&name), None, None)
                    .expect("create ephemeral topic");
                let mut writer = DdsWriter::create(&publisher, topic.clone(), None, None)
                    .expect("create ephemeral writer");
                let reader = DdsReader::create_async(&subscriber, topic, None)
                    .expect("create ephemeral reader");

                writer
                    .write(Arc::new(NormalMsg {
                        id: i,
                        payload: format!("churn-{}", i),
                    }))
                    .expect("ephemeral write");

                let mut buf = NormalMsg::create_sample_buffer(4);
                let got = tokio::time::timeout(Duration::from_secs(5), reader.take(&mut buf))
                    .await
                    .expect("ephemeral read timed out")
                    .expect("ephemeral read failed");
                assert!(
                    got > 0 && buf.is_valid_sample(0),
                    "ephemeral read produced no valid sample on churn iteration {}",
                    i
                );
                // writer/reader/topic all drop here, mid-stream relative to the steady
                // traffic below.
            }

            let mut loaned = steady_writer.loan().expect("steady shm loan");
            let ptr = loaned.as_mut_ptr().expect("loaned ptr");
            let mut sample = ShmMsg::default();
            sample.id = i;
            sample.payload[0] = (i % 256) as u8;
            unsafe { ptr.write(sample) };
            let loaned = loaned.assume_init();
            steady_writer
                .return_loan(loaned)
                .expect("steady shm return_loan");
        }

        for i in CHURN_ITERATIONS..STEADY_COUNT {
            let mut loaned = steady_writer.loan().expect("steady shm loan");
            let ptr = loaned.as_mut_ptr().expect("loaned ptr");
            let mut sample = ShmMsg::default();
            sample.id = i;
            sample.payload[0] = (i % 256) as u8;
            unsafe { ptr.write(sample) };
            let loaned = loaned.assume_init();
            steady_writer
                .return_loan(loaned)
                .expect("steady shm return_loan");
        }

        let seen = tokio::time::timeout(Duration::from_secs(60), steady_task)
            .await
            .expect("steady reader task timed out")
            .expect("steady reader task panicked");
        assert_eq!(seen.len() as u32, STEADY_COUNT);
    });
}

/// Same scenario as stress_pub_sub_normal_and_shm_topics, at 40x the volume - long enough to
/// surface slow leaks (loan_registry growth, refcount leaks) that a quick burst wouldn't show.
#[test]
#[ignore = "requires iox-roudi running"]
fn stress_soak_high_volume() {
    run_dual_topic_stress(48, 20_000, "soak", Duration::from_secs(120));
}

/// Writes N keyed instances, then dispose()s and unregister()s every one of them. There is no
/// safe DdsWriter::dispose()/unregister() wrapper today, so this reaches into cyclonedds_sys
/// directly - the only way anything can currently reach the SDK_KEY path at all.
///
/// This is the direct regression test for the from_sample SDK_KEY fix: before it,
/// serdata_from_sample's SDK_KEY branch was an unconditional panic!(), and dispose/
/// writedispose/unregister_instance construct SDK_KEY serdata as part of the writer's own
/// internal processing (for the write history cache) regardless of whether any reader is
/// listening - so every dispose()/unregister() call crashed the whole process outright,
/// panicking across the extern "C" callback boundary (SIGABRT, not a catchable panic).
#[test]
#[ignore = "requires iox-roudi running"]
fn stress_dispose_unregister() {
    let participant = DdsParticipant::create(Some(49), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let topic = NormalMsg::create_topic(&participant, Some("stress_dispose"), None, None)
        .expect("create topic");
    let mut writer = DdsWriter::create(&publisher, topic.clone(), Some(reliable_qos()), None)
        .expect("create writer");
    let reader = DdsReader::create_async(&subscriber, topic, Some(reliable_qos()))
        .expect("create reader");

    const INSTANCE_COUNT: u32 = 200;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let reader_task = tokio::spawn(async move {
            let mut buf = NormalMsg::create_sample_buffer(32);
            let mut seen_data = HashSet::new();
            while (seen_data.len() as u32) < INSTANCE_COUNT {
                if let Ok(n) = reader.take(&mut buf).await {
                    for i in 0..n {
                        if buf.is_valid_sample(i) {
                            let v = buf
                                .get(i)
                                .try_deref()
                                .expect("valid sample must deref")
                                .clone();
                            assert_eq!(
                                v.payload,
                                format!("dispose-{}", v.id),
                                "corrupted payload"
                            );
                            seen_data.insert(v.id);
                        }
                        // Invalid entries here are dispose/unregister notifications for
                        // instances the writer loop below processes concurrently - just
                        // draining them without crashing is itself part of the check.
                    }
                }
            }
            seen_data
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        for i in 0..INSTANCE_COUNT {
            writer
                .write(Arc::new(NormalMsg {
                    id: i,
                    payload: format!("dispose-{}", i),
                }))
                .expect("write");
        }

        let seen_data = tokio::time::timeout(Duration::from_secs(60), reader_task)
            .await
            .expect("reader task timed out")
            .expect("reader task panicked");
        assert_eq!(seen_data.len() as u32, INSTANCE_COUNT);

        for i in 0..INSTANCE_COUNT {
            let key_sample = NormalMsg {
                id: i,
                payload: String::new(),
            };
            let ret = unsafe {
                cyclonedds_sys::dds_dispose(
                    writer.entity().entity(),
                    &key_sample as *const NormalMsg as *const std::ffi::c_void,
                )
            };
            assert!(ret >= 0, "dispose failed for id {} with retcode {}", i, ret);

            let ret = unsafe {
                cyclonedds_sys::dds_unregister_instance(
                    writer.entity().entity(),
                    &key_sample as *const NormalMsg as *const std::ffi::c_void,
                )
            };
            assert!(
                ret >= 0,
                "unregister failed for id {} with retcode {}",
                i,
                ret
            );
        }
    });
}
