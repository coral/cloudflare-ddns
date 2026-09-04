# The Rust toolchain exists only in this build stage.
FROM rust:1.98.0-alpine3.24 AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

# The published runtime contains only the statically linked executable.
FROM scratch AS runner

LABEL org.opencontainers.image.title="cf-ddns" \
      org.opencontainers.image.description="Small Cloudflare dynamic DNS client" \
      org.opencontainers.image.source="https://github.com/coral/cloudflare-ddns" \
      org.opencontainers.image.licenses="MIT"

COPY --from=builder /build/target/release/cf-ddns /cf-ddns

USER 65532:65532
ENTRYPOINT ["/cf-ddns"]
