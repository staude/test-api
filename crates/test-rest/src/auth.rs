//! Authentication for HTTP adapters: bearer tokens and session cookies.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use test_core::{Channel, ClientIp, Core, Error, Principal, SESSION_COOKIE, SessionInfo};

use crate::error::ApiError;

pub const CSRF_HEADER: &str = "x-csrf-token";

/// Extracts the bearer token from an `Authorization` header.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

pub fn client_ip(parts: &Parts) -> Option<String> {
    parts.extensions.get::<ClientIp>().map(|ip| ip.0.clone())
}

pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(300).collect())
}

pub fn session_cookie(info: &SessionInfo, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, info.cookie_value.clone()))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path("/")
        .expires(info.expires_at)
        .build()
}

pub fn removal_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, "")).path("/").build()
}

/// User-Agent prefix of the `test-app` CLI client.
pub const CLI_USER_AGENT: &str = "test-app-cli/";

/// Audit channel of a REST request: `cli` for the own CLI, else `api`.
pub fn api_channel(headers: &HeaderMap) -> Channel {
    let is_cli = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| ua.starts_with(CLI_USER_AGENT));
    if is_cli { Channel::Cli } else { Channel::Api }
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Settings the auth extractor needs from the router state.
#[derive(Clone)]
pub struct AuthConfig {
    pub cookie_secure: bool,
}

/// An authenticated caller of the REST API.
///
/// Accepts `Authorization: Bearer tapp_…` or the session cookie. Writing
/// requests with a cookie additionally need a matching `X-CSRF-Token`.
pub struct ApiPrincipal(pub Principal);

impl<S> FromRequestParts<S> for ApiPrincipal
where
    S: Send + Sync,
    Core: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let core = Core::from_ref(state);
        let channel = api_channel(&parts.headers);
        if let Some(token) = bearer(&parts.headers) {
            return match core.resolve_token(token, channel).await? {
                Some(p) => Ok(Self(p)),
                None => Err(Error::Unauthenticated.into()),
            };
        }
        let jar = CookieJar::from_headers(&parts.headers);
        let Some(cookie) = jar.get(SESSION_COOKIE) else {
            return Err(Error::Unauthenticated.into());
        };
        let Some((principal, csrf)) = core.resolve_session(cookie.value(), channel).await? else {
            return Err(Error::Unauthenticated.into());
        };
        if !is_safe(&parts.method) {
            let sent = parts.headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok());
            if sent != Some(csrf.as_str()) {
                return Err(Error::Forbidden("CSRF-Token fehlt oder ist ungültig".into()).into());
            }
        }
        Ok(Self(principal))
    }
}

/// Client IP and user agent of a request.
pub struct ClientMeta {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ClientMeta {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            ip: client_ip(parts),
            user_agent: user_agent(&parts.headers),
        })
    }
}
