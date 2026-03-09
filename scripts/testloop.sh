#!/bin/sh
set -eu
# First, build the tests without running them to get the binary
cargo test --no-run

# Find the path to the generated test binary
# TESTBINARY=$(find target/debug -maxdepth 1 -name cyclonedds_rs-\* -type f -executable)

# Loop indefinitely, running the test binary each time
count=0
while true; do
    # $TESTBINARY || break # Break the loop if any test fails
    cargo test -p cyclonedds-rs -- --test-threads=1 || break # Break the loop if any test fails
    count=$((count + 1))
    echo "Test run #$count completed successfully. Restarting..."
done
