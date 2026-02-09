# Stage 1: Build the facilitator binary
FROM rust:1.93-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY r402/Cargo.toml r402/Cargo.toml
COPY r402-evm/Cargo.toml r402-evm/Cargo.toml
COPY r402-http/Cargo.toml r402-http/Cargo.toml
COPY r402-svm/Cargo.toml r402-svm/Cargo.toml
COPY r402-facilitator/Cargo.toml r402-facilitator/Cargo.toml

# Create stub lib/main files so cargo can resolve the dependency graph
RUN mkdir -p r402/src r402-evm/src r402-http/src r402-svm/src r402-facilitator/src \
    && echo "pub fn _stub() {}" > r402/src/lib.rs \
    && echo "pub fn _stub() {}" > r402-evm/src/lib.rs \
    && echo "pub fn _stub() {}" > r402-http/src/lib.rs \
    && echo "pub fn _stub() {}" > r402-svm/src/lib.rs \
    && echo "pub fn _stub() {}" > r402-facilitator/src/lib.rs \
    && echo "fn main() {}" > r402-facilitator/src/main.rs

# Pre-build dependencies (cached unless Cargo.toml/lock changes)
RUN cargo build --release -p r402-facilitator --features bin 2>/dev/null || true

# Copy actual source code
COPY r402/src r402/src
COPY r402-evm/src r402-evm/src
COPY r402-http/src r402-http/src
COPY r402-svm/src r402-svm/src
COPY r402-facilitator/src r402-facilitator/src

# Touch source files to invalidate the stub build cache
RUN find . -name "*.rs" -exec touch {} +

# Build the release binary
RUN cargo build --release -p r402-facilitator --features bin \
    && strip /build/target/release/facilitator

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system facilitator \
    && useradd --system --gid facilitator --create-home facilitator

COPY --from=builder /build/target/release/facilitator /usr/local/bin/facilitator

# Default configuration directory
WORKDIR /app
RUN chown facilitator:facilitator /app

USER facilitator

EXPOSE 4021

# Health check against the /health endpoint
HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -sf http://localhost:4021/health || exit 1

ENV RUST_LOG=info
ENV CONFIG=/app/config.toml

ENTRYPOINT ["/usr/local/bin/facilitator"]