use axum::{routing::get, Router};
use std::net::SocketAddr;
use tokio::sync::watch;
use gateway::{GatewayConfig, RouteRole, proxy_handler};

async fn create_backend(port: u16, response_text: &'static str) {
    let app = Router::new().fallback(move || async move {
        response_text
    });
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
}

#[tokio::test]
async fn test_routing() {
    create_backend(4001, "Hello, it's me, it's homelander").await;
    create_backend(4002, "Hi, it's me, bob").await;

    let config = GatewayConfig {
        route: vec![
            RouteRole {
                pathprefix: "/homelander".to_string(),
                upstreamurl: "http://127.0.0.1:4001".to_string(),
            },
            RouteRole {
                pathprefix: "/sentry".to_string(),
                upstreamurl: "http://127.0.0.1:4002".to_string(),
            },
        ],
    };

    let (tx, rx) = watch::channel(config);

    let app = Router::new()
        .fallback(proxy_handler)
        .with_state(rx);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:2026").await.unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    let resp = client.get("http://127.0.0.1:2026/homelander").send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("homelander"));

    let resp = client.get("http://127.0.0.1:2026/sentry/status").send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("bob"));

    let resp = client.get("http://127.0.0.1:2026/unknown-route").send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}
