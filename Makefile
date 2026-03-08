fmt:
	cargo fmt --all
	cargo clippy --all -- -D warnings

check-fmt:
	cargo fmt --all -- --check
	cargo clippy --all -- -D warnings
