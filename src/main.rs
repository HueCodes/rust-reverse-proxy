use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server, Uri};
use std::convert::Infallible;
use std::net::SocketAddr;

async fn handle_request(req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    // Define the target backend server
    let target = "http://example.com";
    
    // Build the new URI for the target server
    let uri = format!("{}{}", target, req.uri().path_and_query().map(|x| x.as_str()).unwrap_or(""))
        .parse::<Uri>()
        .expect("Invalid URI");

    // Create a new request to the target server
    let mut new_req = Request::builder()
        .method(req.method())
        .uri(uri)
        .body(req.into_body())
        .expect("Failed to build request");

    // Copy headers from the original request
    *new_req.headers_mut() = req.headers().clone();

    // Send the request to the target server
    let client = Client::new();
    let response = client.request(new_req).await?;

    Ok(response)
}

#[tokio::main]
async fn main() {
    // Define the address to listen on
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    // Create a service to handle incoming requests
    let make_service = make_service_fn(|_conn| async {
        Ok::<_, Infallible>(service_fn(handle_request))
    });

    // Start the server
    let server = Server::bind(&addr).serve(make_service);

    println!("Reverse proxy running on http://{}", addr);

    // Run the server
    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }