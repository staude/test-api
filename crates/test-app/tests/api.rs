//! End-to-end tests against the assembled router. Each test gets a fresh
//! database from `#[sqlx::test]` (needs `DATABASE_URL`).

// Helpers outside #[test] fns are not covered by `allow-unwrap-in-tests`.
#![allow(clippy::unwrap_used)]

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use test_app::{HttpConfig, build_router};
use test_core::Core;
use tower::ServiceExt;

const ADMIN_EMAIL: &str = "admin@example.org";
const PASSWORD: &str = "sehr-geheimes-passwort";

struct App {
    router: Router,
    admin: String,
}

impl App {
    async fn new(pool: PgPool) -> Self {
        let core = Core::new(pool).await.unwrap();
        core.bootstrap(Some((ADMIN_EMAIL.into(), PASSWORD.into())))
            .await
            .unwrap();
        let router = build_router(
            core,
            &HttpConfig {
                public_url: "http://localhost".into(),
                trusted_proxies: vec![],
            },
        );
        let mut app = Self {
            router,
            admin: String::new(),
        };
        app.admin = app.login(ADMIN_EMAIL).await;
        app
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "localhost");
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn status(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> StatusCode {
        self.request(method, path, Some(token), body).await.0
    }

    async fn login(&self, email: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                "/api/v1/auth/login",
                None,
                Some(json!({ "email": email, "password": PASSWORD })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "login {email}: {body}");
        body["token"].as_str().unwrap().to_owned()
    }

    /// Creates a user with the given roles and returns a login token.
    async fn user_with_roles(&self, email: &str, roles: &[&str]) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                "/api/v1/users",
                Some(&self.admin),
                Some(json!({
                    "email": email, "display_name": email, "password": PASSWORD, "roles": roles
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        self.login(email).await
    }

    async fn create_note(&self, token: &str, title: &str) -> String {
        let (status, body) = self
            .request(
                Method::POST,
                "/api/v1/notes",
                Some(token),
                Some(json!({ "title": title, "body": "x" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_str().unwrap().to_owned()
    }

    async fn user_id(&self, email: &str) -> String {
        let (_, users) = self
            .request(Method::GET, "/api/v1/users", Some(&self.admin), None)
            .await;
        users
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["email"] == email)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn login_me_and_problem_details(pool: PgPool) {
    let app = App::new(pool).await;
    let (status, me) = app
        .request(Method::GET, "/api/v1/me", Some(&app.admin), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], ADMIN_EMAIL);
    assert_eq!(me["roles"], json!(["admin"]));

    let (status, problem) = app.request(Method::GET, "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(problem["status"], 401);

    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": ADMIN_EMAIL, "password": "falsch-falsch" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Every endpoint × every system role. Protects against new endpoints that
/// forget a permission check.
#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn permission_matrix(pool: PgPool) {
    let app = App::new(pool).await;
    let editor = app.user_with_roles("editor@example.org", &["editor"]).await;
    let member = app.user_with_roles("member@example.org", &["member"]).await;
    let viewer = app.user_with_roles("viewer@example.org", &["viewer"]).await;
    let nobody = app.user_with_roles("nobody@example.org", &[]).await;
    let note = app.create_note(&app.admin, "Admin-Notiz").await;

    use StatusCode as S;
    let new_note = || Some(json!({ "title": "t" }));
    let new_role = |n: &str| Some(json!({ "name": n, "permissions": [] }));
    let cases: Vec<(Method, String, Option<Value>, [S; 5])> = vec![
        // expected for:                                  admin, editor, member, viewer, nobody
        (
            Method::GET,
            "/api/v1/notes".into(),
            None,
            [S::OK, S::OK, S::OK, S::OK, S::FORBIDDEN],
        ),
        (
            Method::POST,
            "/api/v1/notes".into(),
            new_note(),
            [
                S::CREATED,
                S::CREATED,
                S::CREATED,
                S::FORBIDDEN,
                S::FORBIDDEN,
            ],
        ),
        (
            Method::GET,
            format!("/api/v1/notes/{note}"),
            None,
            [S::OK, S::OK, S::NOT_FOUND, S::OK, S::FORBIDDEN],
        ),
        (
            Method::PATCH,
            format!("/api/v1/notes/{note}"),
            Some(json!({ "body": "neu" })),
            [S::OK, S::OK, S::NOT_FOUND, S::FORBIDDEN, S::FORBIDDEN],
        ),
        (
            Method::GET,
            "/api/v1/users".into(),
            None,
            [
                S::OK,
                S::FORBIDDEN,
                S::FORBIDDEN,
                S::FORBIDDEN,
                S::FORBIDDEN,
            ],
        ),
        (
            Method::GET,
            "/api/v1/roles".into(),
            None,
            [S::OK, S::OK, S::OK, S::OK, S::OK],
        ),
        (
            Method::GET,
            "/api/v1/permissions".into(),
            None,
            [S::OK, S::OK, S::OK, S::OK, S::OK],
        ),
        (
            Method::GET,
            "/api/v1/audit".into(),
            None,
            [
                S::OK,
                S::FORBIDDEN,
                S::FORBIDDEN,
                S::FORBIDDEN,
                S::FORBIDDEN,
            ],
        ),
        (
            Method::GET,
            "/api/v1/me/tokens".into(),
            None,
            [S::OK, S::OK, S::OK, S::OK, S::FORBIDDEN],
        ),
        (
            Method::GET,
            "/api/v1/me".into(),
            None,
            [S::OK, S::OK, S::OK, S::OK, S::OK],
        ),
    ];
    let tokens = [&app.admin, &editor, &member, &viewer, &nobody];
    for (method, path, body, expected) in cases {
        for (token, want) in tokens.iter().zip(expected) {
            let got = app.status(method.clone(), &path, token, body.clone()).await;
            assert_eq!(got, want, "{method} {path} with token {}", &token[..12]);
        }
    }
    // Role creation, with unique names per caller.
    for (i, (token, want)) in tokens
        .iter()
        .zip([
            S::CREATED,
            S::FORBIDDEN,
            S::FORBIDDEN,
            S::FORBIDDEN,
            S::FORBIDDEN,
        ])
        .enumerate()
    {
        let got = app
            .status(
                Method::POST,
                "/api/v1/roles",
                token,
                new_role(&format!("rolle-{i}")),
            )
            .await;
        assert_eq!(got, want, "POST /roles #{i}");
    }
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn members_only_see_their_own_notes(pool: PgPool) {
    let app = App::new(pool).await;
    let anna = app.user_with_roles("anna@example.org", &["member"]).await;
    let ben = app.user_with_roles("ben@example.org", &["member"]).await;
    let note = app.create_note(&anna, "Annas Notiz").await;
    app.create_note(&ben, "Bens Notiz").await;

    let (_, page) = app
        .request(Method::GET, "/api/v1/notes", Some(&ben), None)
        .await;
    let titles: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Bens Notiz"]);

    let path = format!("/api/v1/notes/{note}");
    assert_eq!(
        app.status(Method::GET, &path, &ben, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.status(Method::DELETE, &path, &ben, None).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.status(Method::DELETE, &path, &anna, None).await,
        StatusCode::NO_CONTENT
    );
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn scoped_tokens_cannot_exceed_their_scope(pool: PgPool) {
    let app = App::new(pool).await;
    let (status, created) = app
        .request(
            Method::POST,
            "/api/v1/me/tokens",
            Some(&app.admin),
            Some(json!({ "name": "nur lesen", "scopes": ["notes.read", "tokens.manage.own"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let scoped = created["secret"].as_str().unwrap().to_owned();

    assert_eq!(
        app.status(Method::GET, "/api/v1/notes", &scoped, None)
            .await,
        StatusCode::OK
    );
    assert_eq!(
        app.status(
            Method::POST,
            "/api/v1/notes",
            &scoped,
            Some(json!({ "title": "t" }))
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.status(Method::GET, "/api/v1/users", &scoped, None)
            .await,
        StatusCode::FORBIDDEN
    );

    // A token created by a scoped token without explicit scopes inherits the
    // restriction instead of getting the user's full rights.
    let (status, child) = app
        .request(
            Method::POST,
            "/api/v1/me/tokens",
            Some(&scoped),
            Some(json!({ "name": "kind" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let child = child["secret"].as_str().unwrap();
    assert_eq!(
        app.status(Method::GET, "/api/v1/users", child, None).await,
        StatusCode::FORBIDDEN
    );

    // Requesting more than the scoped token has is refused.
    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/me/tokens",
            Some(&scoped),
            Some(json!({ "name": "mehr", "scopes": ["users.manage"] })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn no_privilege_escalation_via_roles(pool: PgPool) {
    let app = App::new(pool).await;
    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/roles",
            Some(&app.admin),
            Some(json!({ "name": "useradmin", "permissions": ["users.read", "users.manage"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let ua = app.user_with_roles("ua@example.org", &["useradmin"]).await;
    let own_id = app.user_id("ua@example.org").await;

    // May not hand out a role with rights they do not have, not even to themselves.
    let (status, _) = app
        .request(
            Method::PUT,
            &format!("/api/v1/users/{own_id}/roles"),
            Some(&ua),
            Some(json!({ "roles": ["useradmin", "admin"] })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/users",
            Some(&ua),
            Some(json!({ "email": "x@example.org", "display_name": "X", "password": PASSWORD, "roles": ["admin"] })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Taking the admin role away from the admin is refused as well.
    let admin_id = app.user_id(ADMIN_EMAIL).await;
    let (status, _) = app
        .request(
            Method::PUT,
            &format!("/api/v1/users/{admin_id}/roles"),
            Some(&ua),
            Some(json!({ "roles": [] })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn last_admin_is_protected(pool: PgPool) {
    let app = App::new(pool).await;
    let admin_id = app.user_id(ADMIN_EMAIL).await;
    let (status, _) = app
        .request(
            Method::PUT,
            &format!("/api/v1/users/{admin_id}/roles"),
            Some(&app.admin),
            Some(json!({ "roles": ["member"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = app
        .request(
            Method::PATCH,
            &format!("/api/v1/users/{admin_id}"),
            Some(&app.admin),
            Some(json!({ "is_active": false })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn deactivation_and_role_changes_apply_immediately(pool: PgPool) {
    let app = App::new(pool).await;
    let anna = app.user_with_roles("anna@example.org", &["editor"]).await;
    let id = app.user_id("anna@example.org").await;
    assert_eq!(
        app.status(Method::GET, "/api/v1/notes", &anna, None).await,
        StatusCode::OK
    );

    app.request(
        Method::PUT,
        &format!("/api/v1/users/{id}/roles"),
        Some(&app.admin),
        Some(json!({ "roles": [] })),
    )
    .await;
    assert_eq!(
        app.status(Method::GET, "/api/v1/notes", &anna, None).await,
        StatusCode::FORBIDDEN
    );

    app.request(
        Method::PATCH,
        &format!("/api/v1/users/{id}"),
        Some(&app.admin),
        Some(json!({ "is_active": false })),
    )
    .await;
    assert_eq!(
        app.status(Method::GET, "/api/v1/me", &anna, None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn failed_logins_are_rate_limited(pool: PgPool) {
    let app = App::new(pool).await;
    let attempt = || {
        app.request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(json!({ "email": ADMIN_EMAIL, "password": "falsch-falsch" })),
        )
    };
    for _ in 0..10 {
        assert_eq!(attempt().await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(attempt().await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn audit_log_records_actions(pool: PgPool) {
    let app = App::new(pool).await;
    app.create_note(&app.admin, "Notiz").await;
    let (_, page) = app
        .request(Method::GET, "/api/v1/audit", Some(&app.admin), None)
        .await;
    let actions: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for expected in ["note.create", "auth.login", "user.create"] {
        assert!(
            actions.contains(&expected),
            "{expected} missing in {actions:?}"
        );
    }
}

// ---------------------------------------------------------------- MCP

async fn mcp(
    app: &App,
    token: &str,
    session: Option<&str>,
    body: Value,
) -> (StatusCode, Option<String>, Value) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(s) = session {
        req = req.header("mcp-session-id", s);
    }
    let res = app
        .router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let session = res
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    // SSE: take the last `data:` line that holds JSON.
    let json = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
        .next_back()
        .or_else(|| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    (status, session, json)
}

async fn mcp_session(app: &App, token: &str) -> String {
    let (status, session, init) = mcp(
        app,
        token,
        None,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" } } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{init}");
    let session = session.expect("session id");
    mcp(
        app,
        token,
        Some(&session),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    session
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn mcp_tools_are_filtered_by_permission(pool: PgPool) {
    let app = App::new(pool).await;
    let viewer = app.user_with_roles("viewer@example.org", &["viewer"]).await;

    let list = |token: String| {
        let app = &app;
        async move {
            let session = mcp_session(app, &token).await;
            let (_, _, res) = mcp(
                app,
                &token,
                Some(&session),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            )
            .await;
            let mut names: Vec<String> = res["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_owned())
                .collect();
            names.sort();
            (session, names)
        }
    };

    let (_, admin_tools) = list(app.admin.clone()).await;
    assert_eq!(admin_tools.len(), 7, "{admin_tools:?}");
    let (session, viewer_tools) = list(viewer.clone()).await;
    assert_eq!(viewer_tools, ["notes_get", "notes_search", "whoami"]);

    // Hidden tools cannot be called either.
    let (_, _, res) = mcp(
        &app,
        &viewer,
        Some(&session),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "notes_create", "arguments": { "title": "x" } } }),
    )
    .await;
    assert!(res.get("error").is_some(), "{res}");

    // Without a token the endpoint answers 401.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "localhost")
        .body(Body::from("{}"))
        .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn mcp_tool_call_uses_service_layer(pool: PgPool) {
    let app = App::new(pool).await;
    let session = mcp_session(&app, &app.admin).await;
    let (_, _, res) = mcp(
        &app,
        &app.admin,
        Some(&session),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "notes_create", "arguments": { "title": "Per MCP", "body": "hallo" } } }),
    )
    .await;
    assert_eq!(res["result"]["isError"], json!(false), "{res}");

    let (_, page) = app
        .request(Method::GET, "/api/v1/notes", Some(&app.admin), None)
        .await;
    assert_eq!(page["items"][0]["title"], "Per MCP");

    let (_, audit) = app
        .request(Method::GET, "/api/v1/audit", Some(&app.admin), None)
        .await;
    let created = audit["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "note.create")
        .unwrap();
    assert_eq!(created["channel"], "mcp");
}

// ---------------------------------------------------------------- Web

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn web_forms_require_csrf_token(pool: PgPool) {
    let app = App::new(pool).await;
    let form = |path: &str, body: String, cookie: Option<&str>| {
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::HOST, "localhost")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        req.body(Body::from(body)).unwrap()
    };
    let res = app
        .router
        .clone()
        .oneshot(form(
            "/login",
            format!("email={ADMIN_EMAIL}&password={PASSWORD}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let set_cookie = res.headers()[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    let cookie = set_cookie.split(';').next().unwrap().to_owned();

    let res = app
        .router
        .clone()
        .oneshot(form("/notes", "title=x&_csrf=falsch".into(), Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // The session cookie alone is not enough for writing API calls either.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/notes")
        .header(header::HOST, "localhost")
        .header(header::COOKIE, &cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"title":"x"}"#))
        .unwrap();
    assert_eq!(
        app.router.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

// ---------------------------------------------------------------- Audit

async fn audit_items(app: &App) -> Vec<Value> {
    let (_, page) = app
        .request(
            Method::GET,
            "/api/v1/audit?limit=200",
            Some(&app.admin),
            None,
        )
        .await;
    page["items"].as_array().unwrap().clone()
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn audit_records_cli_channel_details_and_reads(pool: PgPool) {
    let app = App::new(pool).await;
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/notes")
        .header(header::HOST, "localhost")
        .header(header::USER_AGENT, "test-app-cli/0.1.0")
        .header(header::AUTHORIZATION, format!("Bearer {}", app.admin))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"title":"das ist der titel 2","body":"abc"}"#,
        ))
        .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let id = serde_json::from_slice::<Value>(&bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    app.request(
        Method::GET,
        &format!("/api/v1/notes/{id}"),
        Some(&app.admin),
        None,
    )
    .await;
    app.request(Method::GET, "/api/v1/notes?q=titel", Some(&app.admin), None)
        .await;
    app.request(
        Method::PATCH,
        &format!("/api/v1/notes/{id}"),
        Some(&app.admin),
        Some(json!({ "title": "neuer titel" })),
    )
    .await;

    let items = audit_items(&app).await;
    let find = |action: &str| {
        items
            .iter()
            .find(|e| e["action"] == action)
            .unwrap_or_else(|| panic!("{action} missing"))
            .clone()
    };
    let create = find("note.create");
    assert_eq!(create["channel"], "cli");
    assert_eq!(create["details"]["title"], "das ist der titel 2");
    assert!(create["details"]["token_id"].is_string());

    let read = find("note.read");
    assert_eq!(read["channel"], "api");
    assert_eq!(read["target_id"], id.as_str());

    let list = find("note.list");
    assert_eq!(list["details"]["query"], "titel");
    assert_eq!(list["details"]["count"], 1);

    let update = find("note.update");
    assert_eq!(update["details"]["changed"], json!(["title"]));
    assert_eq!(update["details"]["title_before"], "das ist der titel 2");
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn read_audit_can_be_disabled(pool: PgPool) {
    let core = Core::with_options(
        pool,
        test_core::CoreOptions {
            audit_reads: false,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    core.bootstrap(Some((ADMIN_EMAIL.into(), PASSWORD.into())))
        .await
        .unwrap();
    let router = build_router(
        core,
        &HttpConfig {
            public_url: "http://localhost".into(),
            trusted_proxies: vec![],
        },
    );
    let mut app = App {
        router,
        admin: String::new(),
    };
    app.admin = app.login(ADMIN_EMAIL).await;
    let id = app.create_note(&app.admin, "Notiz").await;
    app.request(
        Method::GET,
        &format!("/api/v1/notes/{id}"),
        Some(&app.admin),
        None,
    )
    .await;

    let actions: Vec<String> = audit_items(&app)
        .await
        .iter()
        .map(|e| e["action"].as_str().unwrap().to_owned())
        .collect();
    assert!(actions.contains(&"note.create".to_owned()));
    assert!(
        !actions.iter().any(|a| a == "note.read" || a == "note.list"),
        "{actions:?}"
    );
}

// ---------------------------------------------------------------- Search & cleanup

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn search_finds_substrings_respects_scope_and_pages(pool: PgPool) {
    let app = App::new(pool).await;
    let anna = app.user_with_roles("anna@example.org", &["member"]).await;
    app.create_note(&anna, "Rezept: Kürbissuppe").await;
    app.create_note(&app.admin, "Admin: Suppenküche").await;
    for i in 0..5 {
        app.create_note(&anna, &format!("Suppe Nr. {i}")).await;
    }
    app.create_note(&anna, "Einkauf").await;

    let titles = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["title"].as_str().unwrap().to_owned())
            .collect()
    };

    // Substring in the middle of a word, case-insensitive; member sees only own notes.
    let (_, page) = app
        .request(
            Method::GET,
            "/api/v1/notes?q=SUPPE&limit=3",
            Some(&anna),
            None,
        )
        .await;
    assert_eq!(titles(&page), ["Suppe Nr. 4", "Suppe Nr. 3", "Suppe Nr. 2"]);
    let cursor = page["next_cursor"].as_str().unwrap().to_owned();

    let (_, page) = app
        .request(
            Method::GET,
            &format!("/api/v1/notes?q=SUPPE&limit=3&cursor={cursor}"),
            Some(&anna),
            None,
        )
        .await;
    assert_eq!(
        titles(&page),
        ["Suppe Nr. 1", "Suppe Nr. 0", "Rezept: Kürbissuppe"]
    );

    // Admin sees all hits, LIKE wildcards in the query are literal.
    let (_, page) = app
        .request(
            Method::GET,
            "/api/v1/notes?q=suppe&limit=50",
            Some(&app.admin),
            None,
        )
        .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 7);
    let (_, page) = app
        .request(Method::GET, "/api/v1/notes?q=%25", Some(&app.admin), None)
        .await;
    assert!(page["items"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn cleanup_removes_audit_entries_past_retention(pool: PgPool) {
    let core = Core::with_options(
        pool.clone(),
        test_core::CoreOptions {
            audit_retention_days: Some(30),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO audit_log (at, channel, action) VALUES
         (now() - interval '31 days', 'api', 'old'),
         (now() - interval '29 days', 'api', 'recent')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let report = core.cleanup().await.unwrap();
    assert_eq!(report.audit_entries, 1);
    let left: Vec<String> = sqlx::query_scalar("SELECT action FROM audit_log")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(left, ["recent"]);
}

// ---------------------------------------------------------------- Security regressions

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn user_manager_cannot_take_over_more_privileged_users(pool: PgPool) {
    let app = App::new(pool).await;
    let (status, _) = app
        .request(
            Method::POST,
            "/api/v1/roles",
            Some(&app.admin),
            Some(json!({ "name": "useradmin", "permissions": ["users.read", "users.manage"] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let ua = app.user_with_roles("ua@example.org", &["useradmin"]).await;
    let admin_id = app.user_id(ADMIN_EMAIL).await;

    // Resetting the admin's password would allow logging in as admin.
    let path = format!("/api/v1/users/{admin_id}");
    let reset = Some(json!({ "password": "uebernommen-123456" }));
    assert_eq!(
        app.status(Method::PATCH, &path, &ua, reset).await,
        StatusCode::FORBIDDEN
    );
    let lock = Some(json!({ "is_active": false }));
    assert_eq!(
        app.status(Method::PATCH, &path, &ua, lock).await,
        StatusCode::FORBIDDEN
    );
    app.login(ADMIN_EMAIL).await;

    // Users whose roles the manager could assign stay manageable.
    app.user_with_roles("plain@example.org", &[]).await;
    let plain = app.user_id("plain@example.org").await;
    let rename = Some(json!({ "display_name": "Neu" }));
    assert_eq!(
        app.status(
            Method::PATCH,
            &format!("/api/v1/users/{plain}"),
            &ua,
            rename
        )
        .await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "test_db::MIGRATOR")]
async fn role_changes_cannot_remove_the_last_user_manager(pool: PgPool) {
    let app = App::new(pool).await;
    let all: Vec<&str> = test_core::Permission::ALL.iter().map(|p| p.key()).collect();
    let (_, role) = app
        .request(
            Method::POST,
            "/api/v1/roles",
            Some(&app.admin),
            Some(json!({ "name": "manager", "permissions": all })),
        )
        .await;
    let role_id = role["id"].as_str().unwrap().to_owned();
    let boss = app.user_with_roles("boss@example.org", &["manager"]).await;
    let admin_id = app.user_id(ADMIN_EMAIL).await;
    // The admin gives up the admin role; the custom role now carries users.manage alone.
    assert_eq!(
        app.status(
            Method::PUT,
            &format!("/api/v1/users/{admin_id}/roles"),
            &boss,
            Some(json!({ "roles": [] }))
        )
        .await,
        StatusCode::OK
    );

    let path = format!("/api/v1/roles/{role_id}");
    let without_manage: Vec<&str> = all
        .iter()
        .copied()
        .filter(|k| *k != "users.manage")
        .collect();
    let strip = Some(json!({ "permissions": without_manage }));
    assert_eq!(
        app.status(Method::PATCH, &path, &boss, strip).await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.status(Method::DELETE, &path, &boss, None).await,
        StatusCode::CONFLICT
    );
    // Still a working manager afterwards.
    assert_eq!(
        app.status(Method::GET, "/api/v1/users", &boss, None).await,
        StatusCode::OK
    );
}
