.PHONY: build run ui-check test format clippy release

# Build the app in debug mode
build:
	cargo build

# Run the app locally in debug mode
run:
	cargo run

# Build the app in release mode (closer to CI artifacts)
release:
	cargo build --release

# Run all tests
test:
	cargo test

# Format all code (matches CI expectation)
format:
	cargo fmt --all

# Run linting (matches CI expectation)
clippy:
	cargo clippy -- -D warnings

# View the Slint UI standalone (hot-reloading enabled if you have slint-viewer installed)
ui-check:
	@echo "Install slint-viewer with: cargo install slint-viewer"
	slint-viewer simply-transfer-ui/ui/main.slint
