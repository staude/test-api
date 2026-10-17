//! REST adapter: `/api/v1` with OpenAPI spec and docs UI.

pub mod auth;
pub mod error;
mod routes;

use axum::Router;
use axum::extract::FromRef;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use test_core::Core;
use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

pub use auth::{ApiPrincipal, AuthConfig, ClientMeta};
pub use error::{ApiError, ApiResult, Problem};

#[derive(Clone)]
pub struct RestState {
    pub core: Core,
    pub auth: AuthConfig,
}

impl FromRef<RestState> for Core {
    fn from_ref(s: &RestState) -> Core {
        s.core.clone()
    }
}

/// Router to be nested under `/api/v1`.
pub fn router(core: Core, auth: AuthConfig) -> Router {
    routes::routes().with_state(RestState { core, auth })
}

/// `/api/v1/openapi.json` is part of [`router`]; this serves the docs UI.
pub fn docs_router() -> Router {
    let html = utoipa_scalar::Scalar::new(ApiDoc::openapi())
        .title("test-app API")
        .to_html();
    Router::new().route(
        "/",
        get(move || async move { Html(html.clone()).into_response() }),
    )
}

struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "test-app API", description = "API first: alles, was Web-UI, CLI und MCP können, geht auch hier."),
    servers((url = "/api/v1")),
    modifiers(&BearerAuth),
    security(("bearer" = [])),
    paths(
        routes::login, routes::logout, routes::me, routes::change_password,
        routes::list_tokens, routes::create_token, routes::revoke_token,
        routes::list_notes, routes::create_note, routes::get_note, routes::update_note, routes::delete_note,
        routes::list_users, routes::create_user, routes::get_user, routes::update_user, routes::set_user_roles,
        routes::list_roles, routes::create_role, routes::get_role, routes::update_role, routes::delete_role,
        routes::list_permissions, routes::list_audit,
    ),
    tags(
        (name = "auth", description = "Anmeldung und eigenes Konto"),
        (name = "notes", description = "Notizen (Beispiel-Ressource)"),
        (name = "users", description = "Benutzerverwaltung"),
        (name = "roles", description = "Rollen und Rechte"),
        (name = "audit", description = "Audit-Log"),
    )
)]
pub struct ApiDoc;
