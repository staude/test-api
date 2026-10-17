//! MCP adapter: tools and resources over Streamable HTTP (`/mcp`) and stdio.
//!
//! Tools are filtered per caller in `tools/list`, and checked again on
//! `tools/call`. The service layer enforces permissions a third time.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use test_core::dto::{NewNote, NoteQuery, NoteUpdate};
use test_core::{Channel, Core, Error, Permission, Principal};
use uuid::Uuid;

const NOTE_URI_PREFIX: &str = "testapp://notes/";

/// How the server learns who is calling.
#[derive(Clone)]
enum Auth {
    /// HTTP: the auth middleware puts a [`Principal`] into the request.
    Http,
    /// stdio: a token from the environment, resolved on every call so that
    /// revocations and role changes apply immediately.
    Token(Arc<str>),
}

#[derive(Clone)]
pub struct McpServer {
    core: Core,
    auth: Auth,
    tool_router: ToolRouter<McpServer>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Search text for title and body. Empty lists the newest notes.
    #[serde(default)]
    pub query: Option<String>,
    /// Maximum number of results (1–50, default 20).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct IdArgs {
    /// Note id (UUID).
    pub id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateArgs {
    pub title: String,
    /// Markdown text.
    #[serde(default)]
    pub body: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateArgs {
    /// Note id (UUID).
    pub id: String,
    pub title: Option<String>,
    pub body: Option<String>,
}

/// Which permission makes a tool visible.
fn tool_allowed(name: &str, p: &Principal) -> bool {
    match name {
        "whoami" => true,
        "notes_search" | "notes_get" => p.has_any_scope(Permission::NotesRead),
        "notes_create" | "notes_update" | "notes_delete" => p.has_any_scope(Permission::NotesWrite),
        "users_list" => p.has(Permission::UsersRead),
        _ => false,
    }
}

fn parse_id(id: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(id.trim()).map_err(|_| Error::Validation(format!("Ungültige Id: {id}")))
}

/// Service errors become tool errors the model can read and react to.
fn reply<T: serde::Serialize>(res: Result<T, Error>) -> Result<CallToolResult, McpError> {
    Ok(match res {
        Ok(v) => CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&v).unwrap_or_default(),
        )]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e.to_string())]),
    })
}

#[tool_router]
impl McpServer {
    fn with_auth(core: Core, auth: Auth) -> Self {
        Self {
            core,
            auth,
            tool_router: Self::tool_router(),
        }
    }

    async fn principal(&self, ctx: &RequestContext<RoleServer>) -> Result<Principal, McpError> {
        let found = match &self.auth {
            Auth::Http => ctx
                .extensions
                .get::<axum::http::request::Parts>()
                .and_then(|parts| parts.extensions.get::<Principal>())
                .cloned(),
            Auth::Token(token) => self
                .core
                .resolve_token(token, Channel::Mcp)
                .await
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        };
        found.ok_or_else(|| McpError::invalid_request("Anmeldung erforderlich", None))
    }

    #[tool(
        description = "Zeigt den angemeldeten Benutzer, seine Rollen und Rechte.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn whoami(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        reply(Ok(self.core.me(&p)))
    }

    #[tool(
        description = "Sucht Notizen nach Titel und Text. Liefert die neuesten zuerst.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn notes_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        let q = NoteQuery {
            q: args.query,
            limit: Some(args.limit.unwrap_or(20).clamp(1, 50)),
            cursor: None,
        };
        reply(self.core.list_notes(&p, q).await.map(|page| page.items))
    }

    #[tool(
        description = "Liefert eine Notiz mit vollständigem Text.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn notes_get(
        &self,
        Parameters(args): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        let res = match parse_id(&args.id) {
            Ok(id) => self.core.get_note(&p, id).await,
            Err(e) => Err(e),
        };
        reply(res)
    }

    #[tool(
        description = "Legt eine neue Notiz an.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn notes_create(
        &self,
        Parameters(args): Parameters<CreateArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        reply(
            self.core
                .create_note(
                    &p,
                    NewNote {
                        title: args.title,
                        body: args.body,
                    },
                )
                .await,
        )
    }

    #[tool(
        description = "Ändert Titel und/oder Text einer Notiz. Nicht angegebene Felder bleiben unverändert.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn notes_update(
        &self,
        Parameters(args): Parameters<UpdateArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        let res = match parse_id(&args.id) {
            Ok(id) => {
                self.core
                    .update_note(
                        &p,
                        id,
                        NoteUpdate {
                            title: args.title,
                            body: args.body,
                        },
                    )
                    .await
            }
            Err(e) => Err(e),
        };
        reply(res)
    }

    #[tool(
        description = "Löscht eine Notiz endgültig.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn notes_delete(
        &self,
        Parameters(args): Parameters<IdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        let res = match parse_id(&args.id) {
            Ok(id) => self.core.delete_note(&p, id).await.map(|()| "gelöscht"),
            Err(e) => Err(e),
        };
        reply(res)
    }

    #[tool(
        description = "Listet alle Benutzer mit ihren Rollen.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn users_list(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx).await?;
        reply(self.core.list_users(&p).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("test-app", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "Notizen-Verwaltung. Welche Tools sichtbar sind, hängt von den Rechten des Tokens ab.",
        )
    }

    async fn list_tools(
        &self,
        _req: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let p = self.principal(&ctx).await?;
        let tools = self
            .tool_router
            .list_all()
            .into_iter()
            .filter(|t| tool_allowed(&t.name, &p))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let p = self.principal(&ctx).await?;
        if !tool_allowed(&request.name, &p) {
            return Err(McpError::invalid_params("tool not found", None));
        }
        let tcc = ToolCallContext::new(self, request, ctx);
        self.tool_router.call(tcc).await
    }

    async fn list_resources(
        &self,
        _req: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let p = self.principal(&ctx).await?;
        if !p.has_any_scope(Permission::NotesRead) {
            return Ok(ListResourcesResult::with_all_items(vec![]));
        }
        let page = self
            .core
            .list_notes(
                &p,
                NoteQuery {
                    limit: Some(50),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let resources = page
            .items
            .into_iter()
            .map(|n| {
                Resource::new(format!("{NOTE_URI_PREFIX}{}", n.id), n.title)
                    .with_mime_type("text/markdown")
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _req: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new(format!("{NOTE_URI_PREFIX}{{id}}"), "note")
                .with_description("Eine Notiz als Markdown")
                .with_mime_type("text/markdown"),
        ]))
    }

    async fn read_resource(
        &self,
        req: ReadResourceRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let p = self.principal(&ctx).await?;
        let not_found = || McpError::resource_not_found("Ressource nicht gefunden", None);
        let id = req
            .uri
            .strip_prefix(NOTE_URI_PREFIX)
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(not_found)?;
        let note = self.core.get_note(&p, id).await.map_err(|e| match e {
            Error::NotFound | Error::Forbidden(_) => not_found(),
            other => McpError::internal_error(other.to_string(), None),
        })?;
        let text = format!("# {}\n\n{}", note.title, note.body);
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, req.uri.clone()).with_mime_type("text/markdown"),
        ])
        .into())
    }
}

/// Router for `/mcp`: bearer auth middleware plus the Streamable HTTP service.
pub fn router(core: Core, allowed_hosts: Vec<String>) -> Router {
    let factory_core = core.clone();
    let config = StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts);
    let service: StreamableHttpService<McpServer, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(McpServer::with_auth(factory_core.clone(), Auth::Http)),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .fallback_service(service)
        .layer(middleware::from_fn_with_state(core, require_token))
}

async fn require_token(State(core): State<Core>, mut req: Request, next: Next) -> Response {
    let principal = match test_rest::auth::bearer(req.headers()) {
        Some(token) => core.resolve_token(token, Channel::Mcp).await,
        None => Ok(None),
    };
    match principal {
        Ok(Some(p)) => {
            req.extensions_mut().insert(p);
            next.run(req).await
        }
        Ok(None) => test_rest::ApiError(Error::Unauthenticated).into_response(),
        Err(e) => test_rest::ApiError(e).into_response(),
    }
}

/// Serves MCP over stdin/stdout with a fixed API token.
pub async fn serve_stdio(core: Core, token: String) -> Result<(), Box<dyn std::error::Error>> {
    if core.resolve_token(&token, Channel::Mcp).await?.is_none() {
        return Err("Token ungültig, abgelaufen oder widerrufen".into());
    }
    let server = McpServer::with_auth(core, Auth::Token(token.into()))
        .serve(rmcp::transport::stdio())
        .await?;
    server.waiting().await?;
    Ok(())
}
