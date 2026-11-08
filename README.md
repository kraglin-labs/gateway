# Gateway

A lightweight edge API gateway / reverse proxy written in Rust. Route HTTP requests
to upstream services by path prefix, with hot-reloading route configuration —
edit `routes.yaml` and the gateway picks it up without a restart.

## Features

- **Path-prefix routing** with longest-prefix-wins matching and segment-boundary rules
- **Hot reload** — `routes.yaml` is watched and changes are applied with a 300 ms debounce; a bad edit logs an error and keeps the last good config
- **Full-duplex proxying** — forwards method, headers, and body; streams responses back to the client
- **Hop-by-hop header stripping** (`connection`, `transfer-encoding`, `upgrade`, etc.) in both directions
- **Connection pooling** to upstreams (10 idle connections per host)
- **Structured error mapping** — `404` no route, `413` body too large, `502` upstream failure, `504` upstream timeout
- **Integration tested** — routing verified against live mock backends

## Tech stack

| Component | Crate |
| --- | --- |
| HTTP server | `axum` 0.8 |
| Async runtime | `tokio` |
| Upstream HTTP client | `reqwest` |
| Config parsing | `serde` + `serde_yaml` |
| File watching | `notify` |

## How it works

```
 client ──► gateway (127.0.0.1:2026)
                │
                ▼  longest matching pathprefix
            routes.yaml ──► upstream service
                ▲
                └── notify watcher (hot reload, 300 ms debounce)
```

Incoming requests are matched against the configured path prefixes. The longest
matching prefix wins (`/orders/item/99` matches `/orders`), a prefix only matches
at a path-segment boundary (`/orders` does **not** match `/ordersfoo`), and `/`
acts as a catch-all if configured. Unmatched paths return `404`.

## Quickstart

```bash
# build and run (serves on http://127.0.0.1:2026)
cargo run

# in another terminal, assuming the sample routes.yaml backends are running
curl http://127.0.0.1:2026/orders/item/99
curl http://127.0.0.1:2026/login
```

## Configuration

Routes live in `routes.yaml` at the project root:

```yaml
route:
  - pathprefix: "/changed"
    upstreamurl: "http://127.0.0.1:4001"

  - pathprefix: "/orders"
    upstreamurl: "http://127.0.0.1:5002"

  - pathprefix: "/login"
    upstreamurl: "http://127.0.0.1:3000"

  - pathprefix: "/signup"
    upstreamurl: "http://127.0.0.1:3001"
```

| Field | Description |
| --- | --- |
| `pathprefix` | Path prefix to match. Must match exactly or at a segment boundary. `/` matches everything. |
| `upstreamurl` | Base URL of the upstream service. The original path and query string are appended. |

Save the file while the gateway is running — the watcher reloads it automatically.

## Proxy behavior

| Behavior | Value |
| --- | --- |
| Max request body | 2 MiB (larger → `413`) |
| Upstream timeout | 30 s (→ `504`) |
| Upstream connection errors | `502` |
| Idle connections per upstream host | 10 |

## Development

```bash
# unit + integration tests (spins up mock backends on local ports)
cargo test

# manual end-to-end smoke test
cargo run          # terminal 1
python test.py     # terminal 2, exercises /changed, /orders, and the 404 fallback
```

## Limitations

- Binds to loopback only (`127.0.0.1:2026`) — not reachable from other machines
- One upstream per prefix: no load balancing, retries, or circuit breaking
- No TLS termination, authentication, rate limiting, or request logging
- 2 MiB request body cap

## Roadmap

- [ ] Multiple upstreams per route with round-robin load balancing
- [ ] Request logging and basic metrics
- [ ] Configurable listen address via CLI/env
- [ ] Rate limiting per route
- [ ] Health checks and automatic upstream failover

## License

TBD
