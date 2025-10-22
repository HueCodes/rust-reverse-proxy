# Rust Reverse Proxy

[![Rust](https://github.com/HueCodes/rust-reverse-proxy/actions/workflows/rust.yml/badge.svg)](https://github.com/HueCodes/rust-reverse-proxy/actions/workflows/rust.yml)
[![Crates.io](https://img.shields.io/crates/v/rust-reverse-proxy.svg)](https://crates.io/crates/rust-reverse-proxy)

A lightweight and fast HTTP reverse proxy written in Rust. This project leverages the [Warp](https://github.com/seanmonstar/warp) framework for efficient request handling and routing, providing a simple yet powerful solution for proxying HTTP traffic.

## Features

- **High Performance**: Built with Rust's zero-cost abstractions and async runtime for low-latency proxying.
- **Flexible Routing**: Supports multiple upstream backends with easy configuration.
- **TLS Support**: Optional TLS termination and forwarding (planned).
- **Logging and Metrics**: Built-in structured logging and basic metrics (in development).
- **Configurable**: YAML or TOML-based configuration for backends, filters, and middleware.

## Quick Start

### Prerequisites

- Rust 1.70+ (stable)
- Cargo

### Installation

Clone the repository and build with Cargo:

```bash
git clone https://github.com/HueCodes/rust-reverse-proxy.git
cd rust-reverse-proxy
cargo build --release
```

### Running the Proxy

Create a basic configuration file `config.yaml`:

```yaml
backends:
  - name: "api"
    host: "http://localhost:8080"
    path_prefix: "/api"
  - name: "web"
    host: "http://localhost:3000"
    path_prefix: "/"

port: 80
```

Start the proxy:

```bash
cargo run -- --config config.yaml
```

Or with the release binary:

```bash
./target/release/rust-reverse-proxy --config config.yaml
```

The proxy will listen on port 80 and forward requests to the configured backends based on path prefixes.

## Configuration

The proxy uses a YAML configuration file. Key sections:

- `backends`: List of upstream services with `name`, `host`, `path_prefix`, and optional `strip_prefix`.
- `port`: Listening port (default: 80).
- `log_level`: Logging verbosity (e.g., "info", "debug").
- `tls`: TLS configuration (cert and key paths, enabled by default if provided).

For full options, see `config.yaml.example` (to be added).

## Usage Examples

### Basic Proxying

Proxy all `/api/*` requests to `http://backend:8080`:

```yaml
backends:
  - name: "backend"
    host: "http://backend:8080"
    path_prefix: "/api"
```

### Load Balancing

Support for round-robin load balancing across multiple hosts (planned feature):

```yaml
backends:
  - name: "api-cluster"
    hosts:
      - "http://backend1:8080"
      - "http://backend2:8080"
    path_prefix: "/api"
    strategy: "round_robin"
```

### Middleware

Add rate limiting or authentication middleware via configuration (in development).

## Development

This project is actively under development. Current focus:

- Implementing core proxy logic with Warp.
- Adding configuration parsing.
- TLS integration with Rustls.

### Building and Testing

```bash
# Run tests
cargo test

# Run with clippy
cargo clippy -- -D warnings

# Format code
cargo fmt
```

### Dependencies

See `Cargo.toml` for details. Key crates:

- `warp`: Web server framework.
- `tokio`: Async runtime.
- `serde`: Configuration serialization.
- `tracing`: Logging.

## Contributing

Contributions are welcome! Please:

1. Fork the repository.
2. Create a feature branch (`git checkout -b feature/my-feature`).
3. Commit changes (`git commit -am 'Add my feature'`).
4. Push to the branch (`git push origin feature/my-feature`).
5. Open a Pull Request.

See [CONTRIBUTING.md](CONTRIBUTING.md) for more details (to be created).

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Inspired by various Rust web projects and the need for a simple, performant proxy.
- Thanks to the Warp and Tokio teams for excellent crates.

---

*Project started in 2025. More features coming soon!*
