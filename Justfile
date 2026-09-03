# justfile for Rust project using Cargo

default: all

all: layout fmt clippy-fix deny test doc-check

layout:
    bash scripts/check-layout.sh

list:
    @just --list

build:
    cargo build --workspace --release --all-features

check:
    cargo check --workspace --all-features

update:
    cargo update

test:
    cargo test --workspace --all-features

bench:
    cargo bench --all-features

# Prerequisites: `rustup toolchain install nightly --component clippy`
clippy:
    cargo +nightly clippy --workspace \
        --all-targets \
        --all-features \
        -- -D warnings

# Prerequisites: `rustup toolchain install nightly --component clippy`
clippy-fix:
    cargo +nightly clippy --workspace \
        --fix \
        --all-targets \
        --all-features \
        --allow-dirty \
        --allow-staged \
        -- -D warnings

# Nightly rustfmt provides import grouping.
# Prerequisites: `rustup toolchain install nightly --component rustfmt`
fmt:
    cargo +nightly fmt --all -- \
        --config unstable_features=true,group_imports=StdExternalCrate,imports_granularity=Module

fmt-check:
    cargo +nightly fmt --all -- \
        --check \
        --config unstable_features=true,group_imports=StdExternalCrate,imports_granularity=Module

# Prerequisites: `rustup toolchain install nightly`
doc:
    cargo +nightly doc --workspace --all-features --no-deps --open

doc-check:
    RUSTDOCFLAGS="-D warnings" cargo +nightly doc --workspace --all-features --no-deps

# Prerequisites: `cargo install cargo-deny`
deny:
    cargo deny check

clean:
    cargo clean
