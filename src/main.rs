use std::fs::File;
use tokio::sync::watch;

#[tokio::main]
async fn main() {
    let initial_file = File::open("routes.yaml").expect("Failed to find routes.yaml at startup!");
    let initial_config = gateway::yaml_parser(initial_file).expect("Initial YAML parsing failed!");

    let (config_tx, config_rx) = watch::channel(initial_config);

    tokio::spawn(async move {
        if let Err(e) = gateway::watcher("routes.yaml", config_tx) {
            eprintln!("Background watcher crashed: {:?}", e);
        }
    });

    let app = axum::Router::new()
        .fallback(gateway::proxy_handler)
        .with_state(config_rx);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:2026").await.unwrap();
    println!("Edge API Gateway running on http://127.0.0.1:2026");

    axum::serve(listener, app).await.unwrap();
}
