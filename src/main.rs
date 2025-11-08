use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use http_body_util::{BodyExt, Full};
use tokio::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use anyhow::{Context, Result};
use log::{info, warn, error, debug};
use serde::Deserialize;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Deserialize)]
struct Config {
    server: ServerConfig,
    backends: Vec<Backend>,
    load_balancing: LoadBalancingConfig,
    health_checks: HealthCheckConfig,
    logging: LoggingConfig,
    timeouts: TimeoutConfig,
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

#[derive(Debug, Clone)]
struct BackendHealth {
    url: String,
    healthy: bool,
    failures: u32,
    last_check: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct LoadBalancer {
    backends: Vec<Backend>,
    health_status: Arc<tokio::sync::RwLock<HashMap<String, BackendHealth>>>,
    round_robin_counter: AtomicUsize,
}

impl LoadBalancer {
    fn new(backends: Vec<Backend>) -> Self {
        let health_status = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        
        // Initialize health status for all backends
        {
            let mut health = health_status.blocking_write();
            for backend in &backends {
                health.insert(backend.url.clone(), BackendHealth {
                    url: backend.url.clone(),
                    healthy: true, // Assume healthy initially
                    failures: 0,
                    last_check: None,
                });
            }
        }

        Self {
            backends,
            health_status,
            round_robin_counter: AtomicUsize::new(0),
        }
    }

    async fn get_healthy_backend(&self) -> Option<Backend> {
        let health = self.health_status.read().await;
        let healthy_backends: Vec<&Backend> = self.backends
            .iter()
            .filter(|backend| {
                health.get(&backend.url)
                    .map(|h| h.healthy)
                    .unwrap_or(true)
            })
            .collect();

        if healthy_backends.is_empty() {
            return None;
        }

        // Round-robin selection
        let index = self.round_robin_counter.fetch_add(1, Ordering::SeqCst) % healthy_backends.len();
        Some(healthy_backends[index].clone())
    }

    async fn mark_backend_unhealthy(&self, url: &str) {
        let mut health = self.health_status.write().await;
        if let Some(backend_health) = health.get_mut(url) {
            backend_health.failures += 1;
            if backend_health.failures >= 3 {
                backend_health.healthy = false;
                warn!("Backend {} marked as unhealthy after {} failures", url, backend_health.failures);
            }
        }
    }

    async fn health_check(&self, config: &HealthCheckConfig) {
        if !config.enabled {
            return;
        }

        let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build_http::<Full<hyper::body::Bytes>>();

        for backend in &self.backends {
            let health_url = format!("{}{}", backend.url, backend.health_check_path);
            
            match client.get(health_url.parse().unwrap()).await {
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
                Err(e) => {
                    debug!("Health check failed for {}: {}", backend.url, e);
                    let mut health = self.health_status.write().await;
                    
                    if let Some(backend_health) = health.get_mut(&backend.url) {
                        backend_health.last_check = Some(Utc::now());
                        backend_health.failures += 1;
                        
                        if backend_health.failures >= config.failure_threshold {
                            if backend_health.healthy {
                                warn!("Backend {} marked as unhealthy due to connection failure", backend.url);
                            }
                            backend_health.healthy = false;
                        }
                    }
                }
            }
        }
    }
}

struct ProxyService {
    load_balancer: Arc<LoadBalancer>,
    config: Arc<Config>,
}

impl ProxyService {
    fn new(config: Config) -> Self {
        let load_balancer = Arc::new(LoadBalancer::new(config.backends.clone()));
        let config = Arc::new(config);
        
        Self {
            load_balancer,
            config,
        }
    }

    async fn handle_request(&self, req: Request<hyper::body::Incoming>) -> Result<Response<Full<hyper::body::Bytes>>, anyhow::Error> {
        let start_time = Instant::now();
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let path_and_query = req.uri().path_and_query().map(|x| x.as_str()).unwrap_or("").to_string();
        let headers = req.headers().clone();
        let remote_addr = "unknown"; // We'll get this from connection context in a real implementation

        info!("Incoming request: {} {}", method, path);

        // Get a healthy backend
        let backend = match self.load_balancer.get_healthy_backend().await {
            Some(backend) => backend,
            None => {
                error!("No healthy backends available");
                return Ok(Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Full::new(hyper::body::Bytes::from("No healthy backends available")))
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

        // Create the client
        let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build_http::<Full<hyper::body::Bytes>>();

        // Collect the request body
        let body_bytes = match req.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to read request body: {}", e);
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(hyper::body::Bytes::from("Failed to read request body")))
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

        // Add X-Forwarded headers
        proxy_req.headers_mut().insert("X-Forwarded-For", remote_addr.parse().unwrap());
        proxy_req.headers_mut().insert("X-Forwarded-Proto", "http".parse().unwrap());

        // Send the request
        let response = match tokio::time::timeout(
            Duration::from_secs(self.config.timeouts.request_timeout_seconds),
            client.request(proxy_req)
        ).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                error!("Request to backend {} failed: {}", backend.url, e);
                self.load_balancer.mark_backend_unhealthy(&backend.url).await;
                return Ok(Response::builder()
                    .status(StatusCode::BAD_GATEWAY)
                    .body(Full::new(hyper::body::Bytes::from("Backend request failed")))
                    .unwrap());
            }
            Err(_) => {
                error!("Request to backend {} timed out", backend.url);
                self.load_balancer.mark_backend_unhealthy(&backend.url).await;
                return Ok(Response::builder()
                    .status(StatusCode::GATEWAY_TIMEOUT)
                    .body(Full::new(hyper::body::Bytes::from("Backend request timed out")))
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
                    .body(Full::new(hyper::body::Bytes::from("Failed to read response from backend")))
                    .unwrap());
            }
        };

        let duration = start_time.elapsed();
        info!("Request completed: {} {} -> {} ({:.2}ms)", 
              method, path, status, duration.as_millis());

        // Build the response
        let mut response_builder = Response::builder().status(status);
        
        // Copy response headers
        for (name, value) in headers {
            if let Some(name) = name {
                response_builder = response_builder.header(name, value);
            }
        }

        Ok(response_builder
            .body(Full::new(body_bytes))
            .unwrap())
    }
}

fn load_config() -> Result<Config> {
    let config_content = std::fs::read_to_string("config.yaml")
        .context("Failed to read config.yaml")?;
    
    let config: Config = serde_yaml::from_str(&config_content)
        .context("Failed to parse config.yaml")?;
    
    Ok(config)
}

fn setup_logging(config: &LoggingConfig) {
    let log_level = match config.level.as_str() {
        "debug" => log::LevelFilter::Debug,
        "info" => log::LevelFilter::Info,
        "warn" => log::LevelFilter::Warn,
        "error" => log::LevelFilter::Error,
        _ => log::LevelFilter::Info,
    };

    env_logger::Builder::from_default_env()
        .filter_level(log_level)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = load_config()?;
    
    // Setup logging
    setup_logging(&config.logging);
    
    info!("Starting reverse proxy server...");
    info!("Configuration loaded: {} backends configured", config.backends.len());
    
    // Create the proxy service
    let proxy_service = Arc::new(ProxyService::new(config.clone()));
    
    // Start health check task
    let health_check_service = proxy_service.load_balancer.clone();
    let health_config = config.health_checks.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(health_config.interval_seconds));
        loop {
            interval.tick().await;
            health_check_service.health_check(&health_config).await;
        }
    });

    // Bind to the configured address
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = TcpListener::bind(&addr).await
        .context(format!("Failed to bind to {}", addr))?;
    
    info!("Reverse proxy listening on http://{}", addr);
    info!("Backend servers:");
    for backend in &config.backends {
        info!("  - {} (weight: {})", backend.url, backend.weight);
    }

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let proxy_service = proxy_service.clone();

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(move |req| {
                    let service = proxy_service.clone();
                    async move { service.handle_request(req).await }
                }))
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}