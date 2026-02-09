# Makefile for x402 LLM Gateway

.PHONY: all
all: pre-commit

# Build the project in release mode
.PHONY: build
build:
	cargo build --release

# Update dependencies to their latest compatible versions
.PHONY: update
update:
	cargo update

# Run the gateway in release mode
.PHONY: run
run:
	cargo run --release

# Run all tests
.PHONY: test
test:
	cargo test

# Run Clippy linter with nightly toolchain, fixing issues automatically
.PHONY: clippy
clippy:
	cargo +nightly clippy --fix \
		--all-targets \
		--allow-dirty \
		--allow-staged \
		-- -D warnings

# Format the code using rustfmt with nightly toolchain
.PHONY: fmt
fmt:
	cargo +nightly fmt

# Generate documentation and open it in the browser
.PHONY: doc
doc:
	cargo +nightly doc --no-deps --open

# Generate CHANGELOG.md using git-cliff
.PHONY: cliff
cliff:
	git cliff --output CHANGELOG.md

# Check for unused dependencies using cargo-udeps
.PHONY: udeps
udeps:
	cargo +nightly udeps

# Run pre-commit checks
.PHONY: pre-commit
pre-commit:
	$(MAKE) build
	$(MAKE) test
	$(MAKE) clippy
	$(MAKE) fmt
