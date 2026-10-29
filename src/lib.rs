use std::fs::File;
use serde::{Serialize, Deserialize};
use std::io::BufReader;
use std::path::Path;
use notify::{Config, Event, RecommendedWatcher, Result, Watcher, RecursiveMode};
use std::sync::mpsc::channel;
use tokio::sync::watch;
use axum::extract::State;
use axum::response::IntoResponse;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRole {
	pub pathprefix: String,
	pub upstreamurl: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
	pub route: Vec<RouteRole>,
}

#[derive(Debug)]
pub enum Errors {
	Io(std::io::Error),
	Serde(serde_yaml::Error),
}

pub fn yaml_parser(file: File) -> std::result::Result<GatewayConfig, Errors> {
	let reader = BufReader::new(file);
	let config: GatewayConfig = serde_yaml::from_reader(reader).map_err(Errors::Serde)?;

	Ok(config)
}

use std::time::{Instant, Duration}; 
pub fn watcher(path: &str, config_tx: watch::Sender<GatewayConfig>) -> Result<()> {
    let (tx, rx) = channel();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event>| {
            match res {
                Ok(event) => {
                    if let Err(e) = tx.send(event) {
                        eprintln!("Channel error: {:?}", e);
                    }
                }
                Err(e) => eprintln!("Watch error: {:?}", e),
            }
        },
        Config::default(),
    )?;

    let target = Path::new(path);
    watcher.watch(target, RecursiveMode::NonRecursive)?;

    println!("Monitoring directory {:?}", target.canonicalize()?);
    println!("Use ctrl+c to stop this madness");

    let mut last_reload = Instant::now();

    for res in rx {
        let mut triggered = false;
        for path in res.paths {
            if path.ends_with("routes.yaml") {
                triggered = true;
                break;
            }
        }

        if triggered {
            if last_reload.elapsed() < Duration::from_millis(300) {
                continue;
            }

            println!("routes.yaml changed. we've gotta reload now :(");

            match File::open("routes.yaml") {
                Ok(file) => {
                    match yaml_parser(file) {
                        Ok(new_configuration) => {
                            if let Err(e) = config_tx.send(new_configuration) {
                                eprintln!("Ummmm, bad news: Failed to broadcast new config: {:?}", e);
                            } else {
                                println!("Gateway reconfiguration was successful.");
                                last_reload = Instant::now(); 
                            }
                        },
                        Err(e) => eprintln!("YAML parser failed while tryna reload: {:?}", e),
                    }
                }
                Err(e) => eprintln!("Failed to open file: {:?}", e),
            }
        }
    }

    Ok(())
}


#[axum::debug_handler]
pub async fn proxy_handler(
    State(config_rx): State<watch::Receiver<GatewayConfig>>, 
    req: axum::extract::Request
) -> impl IntoResponse {
    let upstream_url = {
        let config = config_rx.borrow();
        let path = req.uri().path();
        
        let mut match_upstream = None;
        for route in &config.route {
            if path.starts_with(&route.pathprefix) {
                match_upstream = Some(route.upstreamurl.clone());
                break;
            }
        }

        match match_upstream {
            Some(url) => url,
            None => return axum::http::StatusCode::NOT_FOUND.into_response(),
        }
    };

    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or(req.uri().path());
    let target_url = format!("{}{}", upstream_url, path_and_query);
    
    let (parts, body) = req.into_parts();
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let client = reqwest::Client::new();
    let upstream_req = client.request(parts.method, &target_url)
        .headers(parts.headers)
        .body(body_bytes);

    match upstream_req.send().await {
        Ok(upstream_resp) => {
            let status = upstream_resp.status();
            let headers = upstream_resp.headers().clone();
            
            let bytes = match upstream_resp.bytes().await {
                Ok(b) => b,
                Err(_) => return axum::http::StatusCode::BAD_GATEWAY.into_response(),
            };

            let mut response_builder = axum::response::Response::builder().status(status);

            for (key, value) in headers.iter() {
                response_builder = response_builder.header(key, value);
            }

            response_builder.body(axum::body::Body::from(bytes))
                .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}
