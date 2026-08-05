// Helper binary for tests/dynamic_type_cross_process_test.rs: publishes samples of a
// dynamic type from a genuinely separate OS process (not just a separate thread in the same
// process, as tests/dynamic_type_test.rs's dynamic_type_concurrent_writers exercises), so
// the reader on the other end has to actually discover it over the network and decode real
// CDR bytes rather than share process memory.
//
// Usage: dynamic_cross_writer <domain_id> <count>
// Prints "WRITER_DONE" and exits 0 on success; prints "WRITER_FAIL: ..." and exits 1
// otherwise.

use cyclonedds_rs::*;

#[path = "dynamic_cross_shared/mod.rs"]
mod shared;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: dynamic_cross_writer <domain_id> <count>");
        std::process::exit(1);
    }
    let domain_id: u32 = args[1].parse().expect("domain_id must be a u32");
    let count: u32 = args[2].parse().expect("count must be a u32");

    if let Err(e) = run(domain_id, count) {
        eprintln!("WRITER_FAIL: {:?}", e);
        std::process::exit(1);
    }
    println!("WRITER_DONE");
}

fn run(domain_id: u32, count: u32) -> Result<(), DDSError> {
    let participant = DdsParticipant::create(Some(domain_id), None, None)?;
    let dtype = shared::build_type(&participant)?;
    let publisher = DdsPublisher::create(&participant, None, None)?;
    let topic = DdsDynamicTopic::create(
        &participant,
        &dtype,
        shared::TOPIC_NAME,
        Some(shared::reliable_qos()),
        None,
    )?;
    let mut writer =
        DdsDynamicWriter::create(&publisher, &topic, Some(shared::reliable_qos()), None)?;

    // Give discovery a moment before publishing - the reader process is expected to already
    // be up and matched by the time this runs (the test starts it first), but a real network
    // match still takes a little longer than an in-process one.
    std::thread::sleep(std::time::Duration::from_millis(500));

    for id in 0..count {
        let mut sample = writer.new_sample();
        shared::fill_sample(&mut sample, id)?;
        writer.write(&sample)?;
    }
    Ok(())
}
