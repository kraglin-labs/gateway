use std::fs::File;
use serde::{Serialize, Deserialize};
use std::io::BufReader;
use std::path::Path;
use notify::{Config, Event, RecommendedWatcher, Result, Watcher, RecursiveMode};
use std::sync::mpsc::channel;
use tokio::sync::watch;
use axum::extract::State;
use axum::response::IntoResponse;
use std::time::{Instant, Duration}; 

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

    let target_path = Path::new(path);
    let absolute_target = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(target_path)
    };

    let watch_dir = absolute_target.parent().unwrap_or(Path::new("."));
    watcher.watch(watch_dir, RecursiveMode::NonRecursive)?;

    println!("Monitoring directory {:?}", absolute_target.canonicalize()?);
    println!("Use ctrl+c to stop this madness");

    let target_file_name = absolute_target.file_name().unwrap_or_default();
    let debounce_duration = Duration::from_millis(300);
    let mut pending_reload = false;
    let mut last_event_time = Instant::now();

    loop {
        let timeout = if pending_reload {
            let elapsed = last_event_time.elapsed();
            if elapsed >= debounce_duration {
                pending_reload = false;
                println!("routes.yaml changed, now we've gotta reload");
                match File::open(&absolute_target) {
                    Ok(file) => {
                        match yaml_parser(file) {
                            Ok(new_config) => {
                                if let Err(e) = config_tx.send(new_config) {
                                    eprintln!("Umm, bad news: Failed to broadcast new config: {:?}", e);
                                } else {
                                    println!("gateway reconfiguration was successful");
                                }
                            }
                            Err(e) => eprintln!("YAML parser failed while tryna reload: {:?}", e),
                        }
                    } 
                    Err(e) => eprintln!("Failed to open file: {:?}", e),
                }

                None
            } else {
                Some(debounce_duration - elapsed)
            }
        } else {
            None
        };

        let event_result = match timeout {
            Some(duration) => {
                match rx.recv_timeout(duration) {
                    Ok(smn) => Some(smn),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => {
                match rx.recv() {
                    Ok(smn) => Some(smn),
                    Err(_) => break,
                }
            }
        };

        if let Some(event) = event_result {
            let mut matches_target = false;
            for p in event.paths {
                if p.file_name() == Some(target_file_name) {
                    matches_target = true;
                    break;
                }
            }
            if matches_target {
                pending_reload = true;
                last_event_time = Instant::now();
            }
        }
    }
    Ok(())
}

fn get_http_client() -> reqwest::Client{
	reqwest::Client::builder()
		.timeout(Duration::from_secs(30))
		.pool_max_idle_per_host(10)
		.build()
		.unwrap_or_default()
}
#[axum::debug_handler]
pub async fn proxy_handler(
    State(config_rx): State<watch::Receiver<GatewayConfig>>, 
    req: axum::extract::Request
) -> impl IntoResponse {
    let upstream_url = {
        let config = config_rx.borrow();
        let path = req.uri().path();
        let mut best_match: Option<&RouteRole> = None;
        
        for route in &config.route {
        	let is_match = if route.pathprefix == "/" {
        		true
        	} else if path == route.pathprefix {
        		true
        	} else if path.starts_with(&route.pathprefix) {
        		let next_character = path[route.pathprefix.len()..].chars().next();
        		next_character == Some('/')
        	} else {
        		false
        	};
        	if is_match {
        		let should_update = match best_match {
        			None => true,
        			Some(current_best) => route.pathprefix.len() > current_best.pathprefix.len(),
        		};
        		if should_update {
        			best_match = Some(route)
        		}
        	}
        }
        match best_match {
        	Some(route) => route.upstreamurl.clone(),
        	None => return axum::http::StatusCode::NOT_FOUND.into_response(),
        }
    };

    let path_and_query = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or(req.uri().path());
    let target_url = format!("{}{}", upstream_url, path_and_query);
    
    let (parts, body) = req.into_parts();
    const MAX_BODY_SIZE: usize = 2 * 1024 * 1024;
    let body_bytes = match axum::body::to_bytes(body, MAX_BODY_SIZE).await {
        Ok(bytes) => bytes,
        Err(_) => return axum::http::StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };

	let mut headers = parts.headers;
	let hop_by_hop_headers = [
		"connection",
		"keep-alive",
		"proxy-authenticate",
		"proxy-authorization",
		"te",
		"trailer",
		"transfer-encoding",
		"upgrade",
	];
	for header in &hop_by_hop_headers {
		headers.remove(*header);
	}
	
    let client = get_http_client();
    
    let upstream_req = client
    	.request(parts.method, &target_url)
        .headers(headers.clone())
        .body(body_bytes);

    match upstream_req.send().await {
        Ok(upstream_resp) => {
            let status = upstream_resp.status();
            let mut response_headers = upstream_resp.headers().clone();

            for header in &hop_by_hop_headers {
            	response_headers.remove(*header);
            }
            
            let bytes = match upstream_resp.bytes().await {
                Ok(b) => b,
                Err(_) => return axum::http::StatusCode::BAD_GATEWAY.into_response(),
            };

            let mut response_builder = axum::response::Response::builder().status(status);

            for (key, value) in response_headers.iter() {
                response_builder = response_builder.header(key, value);
            }

            response_builder.body(axum::body::Body::from(bytes))
                .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
        	if e.is_timeout() {
        		axum::http::StatusCode::GATEWAY_TIMEOUT.into_response()
        	} else {
        		axum::http::StatusCode::BAD_GATEWAY.into_response()
        	}
        }
    }
}
