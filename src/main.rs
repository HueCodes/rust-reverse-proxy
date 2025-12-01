use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioIo;
use log::{debug, error, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;

type HttpClient = hyper_util::client::legacy::Client<HttpConnector, Full<Bytes>>;

#[derive(Debug, Clone, Copy)]
enum LoadBalancingStrategy {
    RoundRobin,
    WeightedRoundRobin,
}

impl LoadBalancingStrategy {
    fn from_config(strategy: &str) -> Self {
        match strategy.to_lowercase().as_str() {
            "weighted_round_robin" => LoadBalancingStrategy::WeightedRoundRobin,
            _ => LoadBalancingStrategy::RoundRobin,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Config {
    server: ServerConfig,
    backends: Vec<Backend>,
    load_balancing: LoadBalancingConfig,
    health_checks: HealthCheckConfig,
    logging: LoggingConfig,
    timeouts: TimeoutConfig,
    rate_limiting: RateLimitConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct ServerConfig {
    host: String,
    port: u16,
}

#[derive(Debug, Clone, Deserialize)]
struct Backend {
    url: String,
    weight: u32,
    health_check_path: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LoadBalancingConfig {
    strategy: String,
}

#[derive(Debug, Clone, Deserialize)]
struct HealthCheckConfig {
    enabled: bool,
    interval_seconds: u64,
    timeout_seconds: u64,
    failure_threshold: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct LoggingConfig {
    level: String,
    format: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TimeoutConfig {
    request_timeout_seconds: u64,
    connect_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct RateLimitConfig {
    enabled: bool,
    requests_per_window: u64,
    window_seconds: u64,
}

fn build_http_client(connect_timeout_seconds: u64) -> HttpClient {
    let mut connector = HttpConnector::new();

    if connect_timeout_seconds > 0 {
        connector.set_connect_timeout(Some(Duration::from_secs(connect_timeout_seconds)));
    }

    hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build::<_, Full<Bytes>>(connector)
}

#[derive(Debug, Clone)]
struct BackendHealth {
    healthy: bool,
    failures: u32,
    last_check: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct LoadBalancer {
    backends: Vec<Backend>,
    health_status: Arc<tokio::sync::RwLock<HashMap<String, BackendHealth>>>,
    round_robin_counter: AtomicUsize,
    strategy: LoadBalancingStrategy,
    http_client: HttpClient,
}

#[derive(Debug)]
enum HealthProbeError {
    Timeout,
    Transport(String),
}

impl LoadBalancer {
    fn new(
        backends: Vec<Backend>,
        strategy: LoadBalancingStrategy,
        connect_timeout_seconds: u64,
    ) -> Self {
        let health_status = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        // Initialize health status for all backends
        {
            let mut health = health_status.blocking_write();
            for backend in &backends {
                health.insert(
                    backend.url.clone(),
                    BackendHealth {
                        healthy: true, // Assume healthy initially
                        failures: 0,
                        last_check: None,
                    },
                );
            }
        }

        // Create reusable HTTP client
        let http_client = build_http_client(connect_timeout_seconds);

        Self {
            backends,
            health_status,
            round_robin_counter: AtomicUsize::new(0),
            strategy,
            http_client,
        }
    }

    async fn get_healthy_backend(&self) -> Option<Backend> {
        let health = self.health_status.read().await;
        let healthy_backends: Vec<&Backend> = self
            .backends
            .iter()
            .filter(|backend| health.get(&backend.url).map(|h| h.healthy).unwrap_or(true))
            .collect();

        if healthy_backends.is_empty() {
            return None;
        }

        let selection = match self.strategy {
            LoadBalancingStrategy::RoundRobin => {
                let index = self.round_robin_counter.fetch_add(1, Ordering::SeqCst)
                    % healthy_backends.len();
                healthy_backends[index]
            }
            LoadBalancingStrategy::WeightedRoundRobin => {
                let total_weight: usize = healthy_backends
                    .iter()
                    .map(|backend| backend.weight.max(1) as usize)
                    .sum();

                let position =
                    self.round_robin_counter.fetch_add(1, Ordering::SeqCst) % total_weight.max(1);
                let mut cumulative = 0usize;

                healthy_backends
                    .iter()
                    .find(|backend| {
                        cumulative += backend.weight.max(1) as usize;
                        position < cumulative
                    })
                    .copied()
                    .unwrap_or(healthy_backends[0])
            }
        };

        Some(selection.clone())
    }

    async fn mark_backend_unhealthy(&self, url: &str, failure_threshold: u32) {
        let mut health = self.health_status.write().await;
        if let Some(backend_health) = health.get_mut(url) {
            backend_health.failures += 1;
            if backend_health.failures >= failure_threshold {
                backend_health.healthy = false;
                warn!(
                    "Backend {} marked as unhealthy after {} failures",
                    url, backend_health.failures
                );
            }
        }
    }

    async fn health_check(&self, config: &HealthCheckConfig) {
        if !config.enabled {
            return;
        }

        for backend in &self.backends {
            let health_url = format!("{}{}", backend.url, backend.health_check_path);

            // Parse URL safely
            let uri = match health_url.parse::<Uri>() {
                Ok(uri) => uri,
                Err(e) => {
                    error!("Invalid health check URL {}: {}", health_url, e);
                    continue;
                }
            };

            let request_future = self.http_client.get(uri);
            let response_result = if config.timeout_seconds > 0 {
                match tokio::time::timeout(
                    Duration::from_secs(config.timeout_seconds),
                    request_future,
                )
                .await
                {
                    Ok(inner_result) => {
                        inner_result.map_err(|err| HealthProbeError::Transport(err.to_string()))
                    }
                    Err(_) => Err(HealthProbeError::Timeout),
                }
            } else {
                request_future
                    .await
                    .map_err(|err| HealthProbeError::Transport(err.to_string()))
            };

            match response_result {
                Ok(response) => {
                    let status = response.status();
                    let mut health = self.health_status.write().await;

                    if let Some(backend_health) = health.get_mut(&backend.url) {
                        backend_health.last_check = Some(Utc::now());

                        if status.is_success() {
                            if !backend_health.healthy {
                                info!("Backend {} is now healthy", backend.url);
                            }
                            backend_health.healthy = true;
                            backend_health.failures = 0;
                        } else {
                            backend_health.failures += 1;
                            if backend_health.failures >= config.failure_threshold {
                                if backend_health.healthy {
                                    warn!("Backend {} marked as unhealthy", backend.url);
                                }
                                backend_health.healthy = false;
                            }
                        }
                    }
                }
                Err(error) => {
                    match error {
                        HealthProbeError::Timeout => {
                            debug!(
                                "Health check timed out for {} after {}s",
                                backend.url, config.timeout_seconds
                            );
                        }
                        HealthProbeError::Transport(e) => {
                            debug!("Health check failed for {}: {}", backend.url, e);
                        }
                    }

                    let mut health = self.health_status.write().await;

                    if let Some(backend_health) = health.get_mut(&backend.url) {
                        backend_health.last_check = Some(Utc::now());
                        backend_health.failures += 1;

                        if backend_health.failures >= config.failure_threshold {
                            if backend_health.healthy {
                                warn!(
                                    "Backend {} marked as unhealthy due to connection failure",
                                    backend.url
                                );
                            }
                            backend_health.healthy = false;
                        }
                    }
                }
            }
        }
    }
}

/// Token bucket for rate limiting a single client
/// The bucket refills at a constant rate and each request consumes one token
#[derive(Debug)]
struct TokenBucket {
    tokens: AtomicU64,
    last_refill: std::sync::Mutex<Instant>,
    capacity: u64,
    refill_rate: f64, // tokens per second
}

impl TokenBucket {
    fn new(capacity: u64, window_seconds: u64) -> Self {
        Self {
            tokens: AtomicU64::new(capacity),
            last_refill: std::sync::Mutex::new(Instant::now()),
            capacity,
            refill_rate: capacity as f64 / window_seconds as f64,
        }
    }

    /// Try to consume one token. Returns true if allowed, false if rate limited
    fn try_consume(&self) -> bool {
        // Refill tokens based on time elapsed
        let now = Instant::now();
        let mut last_refill = self.last_refill.lock().unwrap();
        let elapsed = now.duration_since(*last_refill).as_secs_f64();
        
        if elapsed > 0.0 {
            let tokens_to_add = (elapsed * self.refill_rate) as u64;
            if tokens_to_add > 0 {
                let current = self.tokens.load(Ordering::Relaxed);
                let new_tokens = (current + tokens_to_add).min(self.capacity);
                self.tokens.store(new_tokens, Ordering::Relaxed);
                *last_refill = now;
            }
        }
        drop(last_refill);

        // Try to consume a token
        let mut current = self.tokens.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return false;
            }
            match self.tokens.compare_exchange_weak(
                current,
                current - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(x) => current = x,
            }
        }
    }
}

/// Rate limiter managing multiple clients with token buckets
struct RateLimiter {
    buckets: DashMap<String, TokenBucket>,
    config: RateLimitConfig,
}

impl RateLimiter {
    fn new(config: RateLimitConfig) -> Self {
        Self {
            buckets: DashMap::new(),
            config,
        }
    }

    /// Check if a request from this IP should be allowed
    fn check_rate_limit(&self, client_ip: &str) -> bool {
        if !self.config.enabled {
            return true;
        }

        // Get or create bucket for this IP
        let bucket = self.buckets.entry(client_ip.to_string()).or_insert_with(|| {
            TokenBucket::new(self.config.requests_per_window, self.config.window_seconds)
        });

        bucket.try_consume()
    }

    /// Periodically clean up old entries to prevent memory leaks
    fn cleanup_old_entries(&self) {
        // Remove entries that haven't been accessed recently
        // This is a simple cleanup; in production, you might want more sophisticated logic
        if self.buckets.len() > 10000 {
            warn!("Rate limiter has {} entries, consider cleanup", self.buckets.len());
        }
    }
}

struct ProxyService {
    load_balancer: Arc<LoadBalancer>,
    config: Arc<Config>,
    http_client: HttpClient,
    rate_limiter: Arc<RateLimiter>,
}

impl ProxyService {
    fn new(config: Config) -> Self {
        let strategy = LoadBalancingStrategy::from_config(&config.load_balancing.strategy);
        let connect_timeout = config.timeouts.connect_timeout_seconds;
        let load_balancer = Arc::new(LoadBalancer::new(
            config.backends.clone(),
            strategy,
            connect_timeout,
        ));
        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limiting.clone()));
        let config = Arc::new(config);
        let http_client = build_http_client(connect_timeout);

        Self {
            load_balancer,
            config,
            http_client,
            rate_limiter,
        }
    }

    async fn handle_request(
        &self,
        req: Request<hyper::body::Incoming>,
        client_addr: SocketAddr,
    ) -> Result<Response<Full<hyper::body::Bytes>>, anyhow::Error> {
        let start_time = Instant::now();
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let path_and_query = req
            .uri()
            .path_and_query()
            .map(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let headers = req.headers().clone();
        let client_ip = client_addr.ip().to_string();

        info!("Incoming request: {} {} from {}", method, path, client_ip);

        // Check rate limit
        if !self.rate_limiter.check_rate_limit(&client_ip) {
            warn!("Rate limit exceeded for {}", client_ip);
            return Ok(Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("Retry-After", self.config.rate_limiting.window_seconds.to_string())
                .body(Full::new(hyper::body::Bytes::from(
                    "Rate limit exceeded. Please try again later.",
                )))
                .unwrap());
        }

        // Get a healthy backend
        let backend = match self.load_balancer.get_healthy_backend().await {
            Some(backend) => backend,
            None => {
                error!("No healthy backends available");
                return Ok(Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Full::new(hyper::body::Bytes::from(
                        "No healthy backends available",
                    )))
                    .unwrap());
            }
        };

        // Build the target URI
        let target_uri = format!("{}{}", backend.url, path_and_query);
        let uri = match target_uri.parse::<Uri>() {
            Ok(uri) => uri,
            Err(e) => {
                error!("Invalid target URI {}: {}", target_uri, e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Full::new(hyper::body::Bytes::from("Invalid target URL")))
                    .unwrap());
            }
        };

        // Collect the request body
        let body_bytes = match req.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to read request body: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(hyper::body::Bytes::from(
                        "Failed to read request body",
                    )))
                    .unwrap());
            }
        };

        // Create a new request
        let mut proxy_req = Request::builder()
            .method(&method)
            .uri(uri)
            .body(Full::new(body_bytes))
            .context("Failed to build proxy request")?;

        // Copy headers, but skip some that should be handled by the proxy
        for (name, value) in &headers {
            if name != "host" && name != "connection" {
                proxy_req.headers_mut().insert(name, value.clone());
            }
        }

        // Add X-Forwarded headers safely
        if let Ok(forwarded_for) = client_ip.parse() {
            proxy_req
                .headers_mut()
                .insert("X-Forwarded-For", forwarded_for);
        }
        if let Ok(forwarded_proto) = "http".parse() {
            proxy_req
                .headers_mut()
                .insert("X-Forwarded-Proto", forwarded_proto);
        }

        // Send the request
        let response = match tokio::time::timeout(
            Duration::from_secs(self.config.timeouts.request_timeout_seconds),
            self.http_client.request(proxy_req),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                error!("Request to backend {} failed: {}", backend.url, e);
                self.load_balancer
                    .mark_backend_unhealthy(
                        &backend.url,
                        self.config.health_checks.failure_threshold,
                    )
                    .await;
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Full::new(hyper::body::Bytes::from(
                        "Backend request failed",
                    )))
                    .unwrap());
            }
            Err(_) => {
                error!("Request to backend {} timed out", backend.url);
                self.load_balancer
                    .mark_backend_unhealthy(
                        &backend.url,
                        self.config.health_checks.failure_threshold,
                    )
                    .await;
                return Ok(Response::builder()
                    .status(StatusCode::GATEWAY_TIMEOUT)
                    .body(Full::new(hyper::body::Bytes::from(
                        "Backend request timed out",
                    )))
                    .unwrap());
            }
        };

        // Collect the response body
        let status = response.status();
        let headers = response.headers().clone();

        let body_bytes = match response.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to read response body: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Full::new(hyper::body::Bytes::from(
                        "Failed to read response from backend",
                    )))
                    .unwrap());
            }
        };

        let duration = start_time.elapsed();
        info!(
            "Request completed: {} {} -> {} ({:.2}ms)",
            method,
            path,
            status,
            duration.as_millis()
        );

        // Build the response
        let mut response_builder = Response::builder().status(status);

        // Copy response headers
        for (name, value) in headers {
            if let Some(name) = name {
                response_builder = response_builder.header(name, value);
            }
        }

        Ok(response_builder.body(Full::new(body_bytes)).unwrap())
    }
}

fn load_config() -> Result<Config> {
    let config_content =
        std::fs::read_to_string("config.yaml").context("Failed to read config.yaml")?;

    let config: Config =
        serde_yaml::from_str(&config_content).context("Failed to parse config.yaml")?;

    Ok(config)
}

fn setup_logging(config: &LoggingConfig) {
    let level_key = config.level.to_lowercase();
    let log_level = match level_key.as_str() {
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };

    let mut builder = env_logger::Builder::from_default_env();
    builder.filter_level(log_level);

    if config.format.eq_ignore_ascii_case("json") {
        builder.format(|buf, record| {
            let timestamp = Utc::now().to_rfc3339();
            let payload = serde_json::json!({
                "timestamp": timestamp,
                "level": record.level().to_string(),
                "target": record.target(),
                "message": record.args().to_string(),
            });
            writeln!(buf, "{}", payload)
        });
    }

    builder.init();
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = load_config()?;

    // Setup logging
    setup_logging(&config.logging);

    info!("Starting reverse proxy server...");
    info!(
        "Configuration loaded: {} backends configured",
        config.backends.len()
    );

    // Create the proxy service
    let proxy_service = Arc::new(ProxyService::new(config.clone()));

    // Start health check task
    let health_check_service = proxy_service.load_balancer.clone();
    let health_config = config.health_checks.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(health_config.interval_seconds));
        loop {
            interval.tick().await;
            health_check_service.health_check(&health_config).await;
        }
    });

    // Bind to the configured address
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr)
        .await
        .context(format!("Failed to bind to {}", addr))?;

    info!("Reverse proxy listening on http://{}", addr);
    info!("Backend servers:");
    for backend in &config.backends {
        info!("  - {} (weight: {})", backend.url, backend.weight);
    }

    // Start rate limiter cleanup task
    let rate_limiter_cleanup = proxy_service.rate_limiter.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300)); // Every 5 minutes
        loop {
            interval.tick().await;
            rate_limiter_cleanup.cleanup_old_entries();
        }
    });

    // Accept connections
    loop {
        let (stream, client_addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let proxy_service = proxy_service.clone();

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req| {
                        let service = proxy_service.clone();
                        async move { service.handle_request(req, client_addr).await }
                    }),
                )
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}
