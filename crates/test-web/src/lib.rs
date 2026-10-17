//! Web UI: server-rendered HTML (askama) with htmx, all assets embedded.
//!
//! Handlers call the same `Core` services as REST and MCP.

mod pages;
mod views;

use askama::Template;
use axum::Router;
use axum::extract::{FromRef, FromRequestParts, Path};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum_extra::extract::CookieJar;
use rust_embed::RustEmbed;
use test_core::{Channel, Core, Error, Permission, Principal, SESSION_COOKIE};
use tower_http::set_header::SetResponseHeaderLayer;

#[derive(Clone)]
pub struct WebState {
    pub core: Core,
    pub cookie_secure: bool,
}

impl FromRef<WebState> for Core {
    fn from_ref(s: &WebState) -> Core {
        s.core.clone()
    }
}

const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; \
                   frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

pub fn router(core: Core, cookie_secure: bool) -> Router {
    pages::routes()
        .route("/static/{*path}", get(static_file))
        .with_state(WebState {
            core,
            cookie_secure,
        })
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
}

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

async fn static_file(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [
                    (header::CONTENT_TYPE, mime.as_ref().to_owned()),
                    (header::CACHE_CONTROL, "public, max-age=3600".to_owned()),
                ],
                file.data,
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Signed-in browser user plus the CSRF token of the session.
pub struct WebSession {
    pub principal: Principal,
    pub csrf: String,
    pub cookie: String,
}

impl WebSession {
    /// Forms carry the CSRF token as hidden field `_csrf`.
    pub fn check_csrf(&self, sent: &str) -> Result<(), WebError> {
        if sent == self.csrf {
            Ok(())
        } else {
            Err(WebError(Error::Forbidden(
                "Formular abgelaufen, bitte Seite neu laden".into(),
            )))
        }
    }

    pub fn layout(&self, active: &'static str) -> views::Layout {
        let p = &self.principal;
        views::Layout {
            signed_in: true,
            display_name: p.display_name.clone(),
            csrf: self.csrf.clone(),
            active,
            can_users: p.has(Permission::UsersRead),
            can_roles: p.has(Permission::RolesManage),
            can_audit: p.has(Permission::AuditRead),
        }
    }
}

impl<S> FromRequestParts<S> for WebSession
where
    S: Send + Sync,
    Core: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let core = Core::from_ref(state);
        let jar = CookieJar::from_headers(&parts.headers);
        let to_login = || Redirect::to("/login").into_response();
        let Some(cookie) = jar.get(SESSION_COOKIE).map(|c| c.value().to_owned()) else {
            return Err(to_login());
        };
        match core.resolve_session(&cookie, Channel::Web).await {
            Ok(Some((principal, csrf))) => Ok(Self {
                principal,
                csrf,
                cookie,
            }),
            Ok(None) => Err(to_login()),
            Err(e) => Err(WebError(e).into_response()),
        }
    }
}

/// Renders service errors as an HTML error page.
pub struct WebError(pub Error);

impl From<Error> for WebError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl From<askama::Error> for WebError {
    fn from(e: askama::Error) -> Self {
        Self(Error::Internal(e.to_string()))
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let status = test_rest::error::status_of(&self.0);
        let page = views::ErrorPage {
            layout: views::Layout::anonymous(),
            status: status.as_u16(),
            message: self.0.to_string(),
        };
        match page.render() {
            Ok(html) => (status, Html(html)).into_response(),
            Err(_) => status.into_response(),
        }
    }
}

pub type WebResult<T> = Result<T, WebError>;

pub(crate) fn render<T: Template>(t: T) -> WebResult<Html<String>> {
    Ok(Html(t.render()?))
}
