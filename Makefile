fmt:
	cargo fmt --all
	cargo clippy --all --fix --allow-staged --allow-dirty -- -D warnings

check-fmt:
	cargo fmt --all -- --check
	cargo clippy --all

test:
	cargo test --all -- --test-threads=1

deb-cdds-cli:
	mkdir -p deb
	cargo deb --manifest-path cyclonedds/Cargo.toml -p cyclonedds-rs
	cp -f target/debian/cdds-cli_*.deb deb/
