# justfile for Rust project using Cargo

# Default: run the standard local check suite.
# Use `just list` to print recipes instead of running them.
default: all

# Run the most common checks (includes tests; mirrors CI coverage locally).
all: fmt clippy-fix deny test doc-check

# List available recipes
list:
    @just --list

# Build the project with all features enabled in release mode
build:
    cargo build --workspace --release --all-features

# Check the project for compilation errors without producing binaries
check:
    cargo check --workspace --all-features

# Update dependencies to their latest compatible versions
update:
    cargo update

# Run all tests with all features enabled
test:
    cargo test --workspace --all-features

# Run benchmarks with all features enabled
bench:
    cargo bench --all-features

# Run Clippy linter (nightly is only required for a few unstable lints).
# Uses workspace lints from Cargo.toml. Falls back to stable cleanly.
# Prerequisites: `rustup toolchain install nightly --component clippy`
clippy:
    cargo +nightly clippy --workspace \
        --all-targets \
        --all-features \
        -- -D warnings

# Run Clippy linter with auto-fix (for development).
# Prerequisites: `rustup toolchain install nightly --component clippy`
clippy-fix:
    cargo +nightly clippy --workspace \
        --fix \
        --all-targets \
        --all-features \
        --allow-dirty \
        --allow-staged \
        -- -D warnings

# Format the code using rustfmt (nightly provides import grouping).
# Prerequisites: `rustup toolchain install nightly --component rustfmt`
fmt:
    cargo +nightly fmt --all -- \
        --config unstable_features=true,group_imports=StdExternalCrate,imports_granularity=Module

# Check formatting without writing
fmt-check:
    cargo +nightly fmt --all -- \
        --check \
        --config unstable_features=true,group_imports=StdExternalCrate,imports_granularity=Module

# Generate documentation for all crates and open it in the browser.
# Prerequisites: `rustup toolchain install nightly`
doc:
    cargo +nightly doc --workspace --all-features --no-deps --open

# Rustdoc with warnings denied (missing docs, broken links, rustdoc::* lints)
doc-check:
    RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --all-features --no-deps

# Dependency policy (licenses, bans, advisories, sources).
# Prerequisites: `cargo install cargo-deny`
# `all-features` is set in deny.toml [graph]. CI deny-args also pass --all-features.
deny:
    cargo deny check

# Clean build artifacts
clean:
    cargo clean
