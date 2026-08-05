// Helper binary for tests/dynamic_type_cross_process_test.rs - see dynamic_cross_writer.rs's
// header comment for what this pair is testing.
//
// Usage: dynamic_cross_reader <domain_id> <count> <timeout_secs>
// Prints "READER_OK <count>" and exits 0 once `count` valid, uncorrupted, non-duplicate
// samples have been received; prints "READER_FAIL: ..." and exits 1 on error or timeout.

use cyclonedds_rs::*;
use std::time::{Duration, Instant};

#[path = "dynamic_cross_shared/mod.rs"]
mod shared;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: dynamic_cross_reader <domain_id> <count> <timeout_secs>");
        std::process::exit(1);
    }
    let domain_id: u32 = args[1].parse().expect("domain_id must be a u32");
    let count: u32 = args[2].parse().expect("count must be a u32");
    let timeout_secs: u64 = args[3].parse().expect("timeout_secs must be a u64");

    match run(domain_id, count, Duration::from_secs(timeout_secs)) {
        Ok(n) => println!("READER_OK {}", n),
        Err(e) => {
            eprintln!("READER_FAIL: {}", e);
            std::process::exit(1);
        }
    }
}

fn run(domain_id: u32, count: u32, timeout: Duration) -> Result<u32, String> {
    let participant = DdsParticipant::create(Some(domain_id), None, None)
        .map_err(|e| format!("create participant: {:?}", e))?;
    let dtype = shared::build_type(&participant).map_err(|e| format!("build type: {:?}", e))?;
    let subscriber = DdsSubscriber::create(&participant, None, None)
        .map_err(|e| format!("create subscriber: {:?}", e))?;
    let topic = DdsDynamicTopic::create(
        &participant,
        &dtype,
        shared::TOPIC_NAME,
        Some(shared::reliable_qos()),
        None,
    )
    .map_err(|e| format!("create topic: {:?}", e))?;
    let reader = DdsDynamicReader::create(&subscriber, &topic, Some(shared::reliable_qos()), None)
        .map_err(|e| format!("create reader: {:?}", e))?;

    let mut seen = std::collections::HashSet::new();
    let deadline = Instant::now() + timeout;
    while (seen.len() as u32) < count && Instant::now() < deadline {
        match reader.take(32) {
            Ok(samples) => {
                for sample in &samples {
                    shared::check_sample(sample, &mut seen)?;
                }
            }
            Err(DDSError::NoData) => {}
            Err(e) => return Err(format!("take: {:?}", e)),
        }
        if (seen.len() as u32) < count {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    if (seen.len() as u32) < count {
        return Err(format!(
            "timed out: got {} of {} samples",
            seen.len(),
            count
        ));
    }
    Ok(seen.len() as u32)
}
