//! Assembles all adapters into one HTTP server.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use ipnet::IpNet;
use test_core::{ClientIp, Core};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

pub struct HttpConfig {
    /// External base URL, e.g. `https://notes.example.org`.
    pub public_url: String,
    /// Proxies whose `X-Forwarded-For` header is trusted.
    pub trusted_proxies: Vec<IpNet>,
}

impl HttpConfig {
    fn is_https(&self) -> bool {
        self.public_url.starts_with("https://")
    }

    /// Hosts the MCP endpoint accepts (DNS-rebinding protection).
    fn mcp_hosts(&self) -> Vec<String> {
        let mut hosts = vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
        ];
        let without_scheme = self
            .public_url
            .split_once("://")
            .map_or(self.public_url.as_str(), |(_, rest)| rest);
        let authority = without_scheme.split('/').next().unwrap_or_default();
        let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
        if !host.is_empty() && !hosts.iter().any(|h| h == host) {
            hosts.push(host.to_owned());
        }
        hosts
    }
}

pub fn build_router(core: Core, cfg: &HttpConfig) -> Router {
    let secure = cfg.is_https();
    let mut router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(ready).with_state(core.clone()))
        .nest(
            "/api/v1",
            test_rest::router(
                core.clone(),
                test_rest::AuthConfig {
                    cookie_secure: secure,
                },
            ),
        )
        .nest("/api/docs", test_rest::docs_router())
        .nest("/mcp", test_mcp::router(core.clone(), cfg.mcp_hosts()))
        .merge(test_web::router(core, secure))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ));
    if secure {
        router = router.layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        ));
    }
    router
        .layer(middleware::from_fn_with_state(
            Arc::new(cfg.trusted_proxies.clone()),
            client_ip,
        ))
        .layer(TraceLayer::new_for_http())
}

async fn ready(State(core): State<Core>) -> Response {
    match core.ping().await {
        Ok(()) => "ok".into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response(),
    }
}

/// Determines the client IP. `X-Forwarded-For` is only honoured when the
/// direct peer is a trusted proxy; then the rightmost untrusted address wins.
async fn client_ip(
    State(trusted): State<Arc<Vec<IpNet>>>,
    mut req: Request,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let is_trusted = |ip: &IpAddr| trusted.iter().any(|net| net.contains(ip));
    let ip = match peer {
        Some(peer) if is_trusted(&peer) => req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.split(',')
                    .filter_map(|s| s.trim().parse::<IpAddr>().ok())
                    .rev()
                    .find(|ip| !is_trusted(ip))
            })
            .or(Some(peer)),
        other => other,
    };
    if let Some(ip) = ip {
        req.extensions_mut().insert(ClientIp(ip.to_string()));
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_hosts_include_public_host() {
        let cfg = HttpConfig {
            public_url: "https://notes.example.org:8443/".into(),
            trusted_proxies: vec![],
        };
        assert!(cfg.mcp_hosts().contains(&"notes.example.org".to_owned()));
    }
}
