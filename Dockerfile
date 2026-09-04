FROM rust:1.98.0-alpine3.24 AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM scratch

LABEL org.opencontainers.image.title="cf-ddns" \
      org.opencontainers.image.description="Small Cloudflare dynamic DNS client"

COPY --from=builder /build/target/release/cf-ddns /cf-ddns

USER 65532:65532
ENTRYPOINT ["/cf-ddns"]
