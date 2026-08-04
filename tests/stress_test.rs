// Stress test for the writer-loan fix in serdes::loan_registry.
//
// A single publisher/subscriber pair drives two topics concurrently, at volume:
//
//  - NormalMsg (variable-size): its writer goes through DdsWriter::write(), the Arc-sharing
//    in-process zero-copy path that from_sample's *first* branch handles.
//  - ShmMsg (fixed-size): its writer goes through DdsWriter::loan()/return_loan(), the
//    CycloneDDS PSMX/Iceoryx shared-memory zero-copy path that from_sample's *other* branch
//    handles.
//
// Interleaving DdsWriter::write() and DdsWriter::loan() calls on two different types, at
// volume, is exactly the scenario where a registry mixup (wrong type, a stale entry, a race)
// would surface - either as the crash this fix resolves, or as silently corrupted data (which
// the content checks below would catch). Both topics' data is read back and verified.
//
// Requires a running iox-roudi (Iceoryx's shared-memory broker) for the ShmMsg half; see
// dds_writer::test::test_loan for the same requirement. Run explicitly via:
//   cargo test --test stress_test -- --ignored

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

// Distinct from other tests' domain ids (e.g. dds_writer::test::test_loan uses 42) so this
// test always gets a fresh, never-before-configured domain. CycloneDDS caches a domain's
// config at first use per process - see the note in test_loan for the full story.
const STRESS_DOMAIN_ID: u32 = 43;

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

    std::env::set_var("CYCLONEDDS_URI", SHM_CONFIG);

    let participant =
        DdsParticipant::create(Some(STRESS_DOMAIN_ID), None, None).expect("create participant");
    let publisher = DdsPublisher::create(&participant, None, None).expect("create publisher");
    let subscriber = DdsSubscriber::create(&participant, None, None).expect("create subscriber");

    let normal_topic = NormalMsg::create_topic(&participant, Some("stress_normal"), None, None)
        .expect("create normal topic");
    let mut normal_writer = DdsWriter::create(&publisher, normal_topic.clone(), None, None)
        .expect("create normal writer");
    let normal_reader = DdsReader::create_async(&subscriber, normal_topic, None)
        .expect("create normal reader");

    let shm_topic = ShmMsg::create_topic(&participant, Some("stress_shm"), None, None)
        .expect("create shm topic");
    let mut shm_writer =
        DdsWriter::create(&publisher, shm_topic.clone(), None, None).expect("create shm writer");
    let shm_reader = DdsReader::create_async(&subscriber, shm_topic, None)
        .expect("create shm reader");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let normal_task = tokio::spawn(async move {
            let mut buf = NormalMsg::create_sample_buffer(32);
            let mut seen = HashSet::new();
            while (seen.len() as u32) < SAMPLE_COUNT {
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
            while (seen.len() as u32) < SAMPLE_COUNT {
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

        for i in 0..SAMPLE_COUNT {
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

        let normal_seen = tokio::time::timeout(Duration::from_secs(30), normal_task)
            .await
            .expect("normal reader task timed out")
            .expect("normal reader task panicked");
        let shm_seen = tokio::time::timeout(Duration::from_secs(30), shm_task)
            .await
            .expect("shm reader task timed out")
            .expect("shm reader task panicked");

        assert_eq!(normal_seen.len() as u32, SAMPLE_COUNT);
        assert_eq!(shm_seen.len() as u32, SAMPLE_COUNT);
    });
}
