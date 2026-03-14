# Build stage
FROM rust:latest AS builder

WORKDIR /app

# Install dependencies for native-tls
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for caching
COPY Cargo.toml Cargo.lock ./

# Create dummy src to build dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "pub fn dummy() {}" > src/lib.rs && \
    echo "fn main() {}" > src/bin/backtest.rs && \
    echo "fn main() {}" > src/bin/export_csv.rs && \
    echo "fn main() {}" > src/bin/fetch_activity.rs

# Build dependencies (cached layer)
RUN cargo build --release && rm -rf src

# Copy actual source code
COPY src ./src

# Touch main.rs to force rebuild of actual code
RUN touch src/main.rs src/lib.rs

# Build the actual application
RUN cargo build --release --bin white-shark

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /app/target/release/white-shark /app/white-shark

# Run as non-root user
RUN useradd -m -u 1000 appuser && chown -R appuser:appuser /app
USER appuser

CMD ["/app/white-shark"]
