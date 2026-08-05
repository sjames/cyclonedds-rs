// Genuinely cross-process test for the Dynamic Type MVP: spawns examples/dynamic_cross_reader
// and examples/dynamic_cross_writer as two separate OS processes on the same domain, and
// checks that samples published by one are received correctly by the other over real DDS
// network transport (RTPS/UDP, not SHM) - unlike every other dynamic-type test, which runs
// writer and reader in the same process and could in principle be passing for a reason
// specific to sharing process memory. See examples/dynamic_cross_shared/mod.rs for the
// (must-match-exactly) type both processes build.
//
// Requires the example binaries to be built first: `cargo test` alone does not build
// examples, so this test builds them itself via `cargo build --examples` as a first step
// (this is what makes the test self-contained rather than requiring a separate manual build
// step, at the cost of a slower first run).

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn example_path(name: &str) -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR/target/debug/examples/<name> - matches this crate's own target dir
    // regardless of the workspace-relative path cargo test happens to be invoked from.
    workspace_root().join("target/debug/examples").join(name)
}

fn build_examples() {
    // Deliberately does not set LD_LIBRARY_PATH itself: it inherits whatever the parent test
    // process's own environment already is, which - since the parent is running at all - is
    // demonstrably already correct for finding libddsc.so on this machine, wherever that
    // actually is (a hardcoded path here previously assumed /usr/local/lib, which is only
    // where this happens to live on a machine with a manual system-wide install; CI's actual
    // library location is elsewhere, and hardcoding it broke there).
    let status = Command::new("cargo")
        .args(["build", "--examples"])
        .current_dir(workspace_root())
        .status()
        .expect("failed to run cargo build --examples");
    assert!(status.success(), "cargo build --examples failed");
}

fn read_all(mut child: std::process::Child) -> (bool, String, String) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut stdout);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut stderr);
    }
    let status = child.wait().expect("wait on child failed");
    (status.success(), stdout, stderr)
}

#[test]
fn dynamic_type_cross_process_round_trip() {
    build_examples();

    const DOMAIN_ID: u32 = 68;
    const COUNT: u32 = 50;
    const READER_TIMEOUT_SECS: u64 = 30;

    // Reader first (background) so it's up and matched before the writer publishes -
    // default QoS durability is volatile, so a reader started after the writer already
    // finished would simply miss everything.
    let reader_child = Command::new(example_path("dynamic_cross_reader"))
        .args([
            DOMAIN_ID.to_string(),
            COUNT.to_string(),
            READER_TIMEOUT_SECS.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn dynamic_cross_reader");

    std::thread::sleep(Duration::from_millis(500));

    let writer_output = Command::new(example_path("dynamic_cross_writer"))
        .args([DOMAIN_ID.to_string(), COUNT.to_string()])
        .output()
        .expect("failed to spawn dynamic_cross_writer");

    assert!(
        writer_output.status.success(),
        "writer process failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&writer_output.stdout),
        String::from_utf8_lossy(&writer_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&writer_output.stdout).contains("WRITER_DONE"),
        "writer did not report WRITER_DONE: {}",
        String::from_utf8_lossy(&writer_output.stdout)
    );

    let deadline = Instant::now() + Duration::from_secs(READER_TIMEOUT_SECS + 10);
    let (ok, stdout, stderr) = read_all(reader_child);
    assert!(
        Instant::now() < deadline,
        "reader process took longer than its own timeout plus margin - likely hung"
    );
    assert!(
        ok,
        "reader process failed:\nstdout: {}\nstderr: {}",
        stdout, stderr
    );
    assert!(
        stdout.contains(&format!("READER_OK {}", COUNT)),
        "reader did not report success for all {} samples: {}",
        COUNT,
        stdout
    );
}
