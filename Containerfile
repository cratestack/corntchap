# syntax=docker/dockerfile:1
# Static musl build: Alpine's rust toolchain is natively musl and links statically by default (do not force
# +crt-static: it breaks proc-macros). Works for amd64 and arm64.
FROM rust:1.100-alpine AS builder
ARG TARGETPLATFORM
RUN apk add --no-cache musl-dev build-base cmake perl
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=target-$TARGETPLATFORM,target=/src/target \
    cargo build --release -p corntchap \
 && strip target/release/corntchap \
 && cp target/release/corntchap /corntchap

FROM gcr.io/distroless/static-debian13:nonroot
LABEL org.opencontainers.image.title="corntchap" \
      org.opencontainers.image.description="Coordination server for coding agents: tasks, leases, MCP, REST" \
      org.opencontainers.image.source="https://github.com/stephane-segning/corntchap" \
      org.opencontainers.image.url="https://github.com/stephane-segning/corntchap" \
      org.opencontainers.image.licenses="MIT"
COPY --from=builder /corntchap /corntchap
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/corntchap"]
CMD ["serve"]
