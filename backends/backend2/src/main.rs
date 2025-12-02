use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::net::SocketAddr;
use tokio::net::TcpListener;

const SERVER_NAME: &str = "Backend-2";
const PORT: u16 = 8002;

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    client_addr: SocketAddr,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let headers: Vec<(String, String)> = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Health check endpoint
    if path == "/health" {
        let response = json!({
            "status": "healthy",
            "server": SERVER_NAME,
            "port": PORT
        });

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(response.to_string())))
            .unwrap());
    }

    // Echo endpoint - returns request information
    let response = json!({
        "server": SERVER_NAME,
        "port": PORT,
        "method": method,
        "path": path,
        "client_address": client_addr.to_string(),
        "headers": headers,
        "message": format!("Hello from {}!", SERVER_NAME)
    });

    println!("[{}] {} {} from {}", SERVER_NAME, method, path, client_addr);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .header("X-Served-By", SERVER_NAME)
        .body(Full::new(Bytes::from(response.to_string())))
        .unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], PORT));
    let listener = TcpListener::bind(addr).await?;

    println!("[{}] Listening on http://{}", SERVER_NAME, addr);

    loop {
        let (stream, client_addr) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(move |req| handle_request(req, client_addr)),
                )
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}
