# Rust Reverse Proxy

A lightweight, high-performance HTTP reverse proxy written in Rust, prioritizing security, memory safety, and concurrency.

## Features

- **High Performance**: Built with Tokio and Hyper for maximum throughput
- **Load Balancing**: Round-robin load balancing across multiple backend servers
- **Health Checks**: Automatic health monitoring of backend servers
- **Configuration-Driven**: YAML-based configuration for easy management
- **Structured Logging**: Comprehensive request/response logging with configurable levels
- **Fault Tolerance**: Automatic failover and error handling
- **Request Timeouts**: Configurable timeouts for reliability
- **Header Forwarding**: Proper X-Forwarded-* headers for backend services

## Quick Start

1. **Build the project**:
   ```bash
   cargo build --release
   ```

2. **Configure your backends** in `config.yaml`:
   ```yaml
   server:
     host: "127.0.0.1"
     port: 3000
   
   backends:
     - url: "http://backend1.example.com"
       weight: 1
       health_check_path: "/health"
     - url: "http://backend2.example.com"
       weight: 1  
       health_check_path: "/health"
   ```

3. **Run the proxy**:
   ```bash
   cargo run --release
   ```

## Configuration

The proxy is configured using a `config.yaml` file:

```yaml
server:
  host: "127.0.0.1"      # Interface to bind to
  port: 3000             # Port to listen on

backends:
  - url: "http://httpbin.org"       # Backend server URL
    weight: 1                       # Load balancing weight
    health_check_path: "/status/200" # Health check endpoint
  - url: "http://example.com"
    weight: 1
    health_check_path: "/"

load_balancing:
  strategy: "round_robin"  # Load balancing strategy

health_checks:
  enabled: true           # Enable/disable health checks
  interval_seconds: 30    # Health check interval
  timeout_seconds: 5      # Health check timeout
  failure_threshold: 3    # Failures before marking unhealthy

logging:
  level: "info"          # Log level (debug, info, warn, error)
  format: "json"         # Log format (json, text)

timeouts:
  request_timeout_seconds: 30   # Backend request timeout
  connect_timeout_seconds: 10   # Connection timeout
```

## Architecture

The proxy consists of several key components:

- **ProxyService**: Main request handling service
- **LoadBalancer**: Manages backend selection and health status
- **HealthChecker**: Monitors backend server health
- **Configuration**: YAML-based configuration management

## Load Balancing

Currently supports round-robin load balancing:
- Requests are distributed evenly across healthy backends
- Unhealthy backends are automatically excluded
- Backends are marked unhealthy after consecutive failures

## Health Checks

- Periodic health checks to all configured backends
- Configurable health check paths and intervals
- Automatic failover when backends become unhealthy
- Recovery detection when backends come back online

## Logging

Structured logging with configurable levels:
- Request/response logging with timing information
- Health check status updates
- Error tracking and debugging information
- JSON or plain text output formats

## Error Handling

Robust error handling with appropriate HTTP status codes:
- `503 Service Unavailable`: No healthy backends
- `502 Bad Gateway`: Backend request failures
- `504 Gateway Timeout`: Backend request timeouts
- `400 Bad Request`: Client request issues

## Performance

Optimized for high performance:
- Async/await with Tokio runtime
- Connection pooling with Hyper client
- Zero-copy request/response forwarding where possible
- Minimal memory allocations

## Development

### Building
```bash
cargo build
```

### Testing
```bash
cargo test
```

### Running with Debug Logging
```bash
RUST_LOG=debug cargo run
```

## Docker Support

Create a `Dockerfile`:
```dockerfile
FROM rust:1.70 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/reverse_proxy /usr/local/bin/
COPY config.yaml /etc/reverse-proxy/
WORKDIR /etc/reverse-proxy
EXPOSE 3000
CMD ["reverse_proxy"]
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests if applicable
5. Submit a pull request

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Roadmap

- [ ] HTTPS/TLS termination support
- [ ] WebSocket proxying
- [ ] Metrics and monitoring endpoints
- [ ] Rate limiting
- [ ] Path-based routing
- [ ] Circuit breaker pattern
- [ ] Admin API for runtime configuration
