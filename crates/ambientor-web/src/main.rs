#![deny(unsafe_code)]

use std::net::SocketAddr;

use axum::{
    Router,
    body::Body,
    http::{Response, StatusCode, header},
    response::{Html, IntoResponse},
    routing::get,
};

const INDEX: &str = include_str!("../assets/index.html");
const MOCKUP_DASHBOARD: &str = include_str!("../assets/mockup-dashboard.html");
const STYLE: &str = include_str!("../assets/style.css");
const APP_JS: &str = include_str!("../assets/app.js");
const LOGO_ICON_64: &[u8] = include_bytes!("../assets/logo/icon-64.png");
const LOGO_ICON_256: &[u8] = include_bytes!("../assets/logo/icon-256.png");
const FAVICON: &[u8] = include_bytes!("../assets/logo/favicon.ico");

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/", get(index))
        .route("/mockup-dashboard.html", get(mockup_dashboard))
        .route("/style.css", get(style))
        .route("/app.js", get(app_js))
        .route("/config.js", get(config_js))
        .route("/logo/icon-64.png", get(logo_icon_64))
        .route("/logo/icon-256.png", get(logo_icon_256))
        .route("/favicon.ico", get(favicon));

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

async fn mockup_dashboard() -> Html<&'static str> {
    Html(MOCKUP_DASHBOARD)
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

fn png_response(bytes: &'static [u8]) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from(bytes))
        .expect("png response")
}

async fn logo_icon_64() -> Response<Body> {
    png_response(LOGO_ICON_64)
}

async fn logo_icon_256() -> Response<Body> {
    png_response(LOGO_ICON_256)
}

async fn favicon() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/x-icon")
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(Body::from(FAVICON))
        .expect("favicon response")
}
