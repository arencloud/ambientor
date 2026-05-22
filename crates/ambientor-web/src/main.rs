#![deny(unsafe_code)]

use std::net::SocketAddr;

use axum::{
    Router,
    response::{Html, IntoResponse},
    routing::get,
};

const INDEX: &str = include_str!("../assets/index.html");
const STYLE: &str = include_str!("../assets/style.css");
const APP_JS: &str = include_str!("../assets/app.js");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/", get(index))
        .route("/style.css", get(style))
        .route("/app.js", get(app_js))
        .route("/config.js", get(config_js));

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

async fn app_js() -> ([(&'static str, &'static str); 1], &'static str) {
    ([("content-type", "application/javascript")], APP_JS)
}

async fn config_js() -> impl IntoResponse {
    let url = std::env::var("AMBIENTOR_API_URL").unwrap_or_default();
    let body = format!("window.AMBIENTOR_API_URL = {url:?};\n");
    ([("content-type", "application/javascript")], body)
}
