#![deny(unsafe_code)]

use std::net::SocketAddr;

use axum::{Router, response::Html, routing::get};

const INDEX: &str = include_str!("../assets/index.html");
const STYLE: &str = include_str!("../assets/style.css");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/", get(index))
        .route("/style.css", get(style));

    let addr: SocketAddr = std::env::var("AMBIENTOR_WEB_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".into())
        .parse()
        .expect("valid AMBIENTOR_WEB_ADDR");
    tracing::info!(%addr, "starting ambientor-web");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    axum::serve(listener, app).await.expect("serve");
}

async fn index() -> Html<&'static str> {
    Html(INDEX)
}

async fn style() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "text/css")], STYLE)
}
