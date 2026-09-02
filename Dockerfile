# Build stage
FROM rust:latest as builder

WORKDIR /app

# Copy all source code
COPY . .

# Remove problematic Cargo.lock and let Cargo generate a fresh one
# Build only the binaries we need (skip main which has compilation errors)
RUN rm -f Cargo.lock && cargo build --release --bin web_server --bin analyzer --bin test_hierarchical

# Runtime stage
FROM ubuntu:24.04

# Install required runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create app user
RUN useradd -r -s /bin/false appuser

# Create directories for persistent storage
RUN mkdir -p /app/uploads /app/generated && \
    chown -R appuser:appuser /app

WORKDIR /app

# Copy the binaries from builder stage
COPY --from=builder /app/target/release/web_server /app/web_server
COPY --from=builder /app/target/release/analyzer /app/analyzer
COPY --from=builder /app/target/release/test_hierarchical /app/test_hierarchical
COPY --from=builder /app/templates /app/templates

# Change ownership
RUN chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Expose port
EXPOSE 3000

# Run the application
CMD ["./web_server"]