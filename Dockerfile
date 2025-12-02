FROM rust:1.75 AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/reverse_proxy /usr/local/bin/
COPY config.docker.yaml /etc/reverse-proxy/config.yaml

WORKDIR /etc/reverse-proxy

ENV CONFIG_PATH=/etc/reverse-proxy/config.yaml

EXPOSE 3000

CMD ["reverse_proxy"]