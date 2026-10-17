//! Typed HTTP client for `/api/v1`, used by the CLI.

use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use test_core::dto::*;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Verbindung fehlgeschlagen: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{detail} (HTTP {status})")]
    Api { status: u16, detail: String },
}

pub type Result<T> = std::result::Result<T, ClientError>;

pub struct Client {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base_url: &str, token: Option<String>) -> Self {
        Self {
            base: format!("{}/api/v1", base_url.trim_end_matches('/')),
            token,
            http: reqwest::Client::builder()
                .user_agent(concat!("test-app-cli/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
        }
    }

    fn req(&self, method: Method, path: &str) -> RequestBuilder {
        let rb = self.http.request(method, format!("{}{path}", self.base));
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    async fn send<T: DeserializeOwned>(rb: RequestBuilder) -> Result<T> {
        let res = Self::check(rb.send().await?).await?;
        Ok(res.json().await?)
    }

    async fn send_empty(rb: RequestBuilder) -> Result<()> {
        Self::check(rb.send().await?).await.map(|_| ())
    }

    async fn check(res: reqwest::Response) -> Result<reqwest::Response> {
        let status = res.status();
        if status.is_success() {
            return Ok(res);
        }
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        let detail = body
            .get("detail")
            .and_then(|d| d.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("Fehler").to_owned());
        Err(ClientError::Api {
            status: status.as_u16(),
            detail,
        })
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.req(Method::GET, path)
    }

    fn json<B: Serialize>(&self, method: Method, path: &str, body: &B) -> RequestBuilder {
        self.req(method, path).json(body)
    }

    pub async fn login(
        &self,
        email: &str,
        password: &str,
        token_name: &str,
    ) -> Result<LoginResponse> {
        let body = LoginRequest {
            email: email.into(),
            password: password.into(),
            mode: LoginMode::Token,
            token_name: Some(token_name.into()),
        };
        Self::send(self.json(Method::POST, "/auth/login", &body)).await
    }

    pub async fn logout(&self) -> Result<()> {
        Self::send_empty(self.req(Method::POST, "/auth/logout")).await
    }

    pub async fn me(&self) -> Result<Me> {
        Self::send(self.get("/me")).await
    }

    pub async fn list_notes(&self, q: Option<&str>, limit: i64) -> Result<NotePage> {
        let mut rb = self.get("/notes").query(&[("limit", limit.to_string())]);
        if let Some(q) = q {
            rb = rb.query(&[("q", q)]);
        }
        Self::send(rb).await
    }

    pub async fn get_note(&self, id: Uuid) -> Result<Note> {
        Self::send(self.get(&format!("/notes/{id}"))).await
    }

    pub async fn create_note(&self, body: &NewNote) -> Result<Note> {
        Self::send(self.json(Method::POST, "/notes", body)).await
    }

    pub async fn update_note(&self, id: Uuid, body: &NoteUpdate) -> Result<Note> {
        Self::send(self.json(Method::PATCH, &format!("/notes/{id}"), body)).await
    }

    pub async fn delete_note(&self, id: Uuid) -> Result<()> {
        Self::send_empty(self.req(Method::DELETE, &format!("/notes/{id}"))).await
    }

    pub async fn list_users(&self) -> Result<Vec<User>> {
        Self::send(self.get("/users")).await
    }

    pub async fn create_user(&self, body: &NewUser) -> Result<User> {
        Self::send(self.json(Method::POST, "/users", body)).await
    }

    pub async fn update_user(&self, id: Uuid, body: &UserUpdate) -> Result<User> {
        Self::send(self.json(Method::PATCH, &format!("/users/{id}"), body)).await
    }

    pub async fn set_user_roles(&self, id: Uuid, roles: Vec<String>) -> Result<User> {
        Self::send(self.json(
            Method::PUT,
            &format!("/users/{id}/roles"),
            &SetRoles { roles },
        ))
        .await
    }

    /// Resolves a user by e-mail via the user list.
    pub async fn user_by_email(&self, email: &str) -> Result<User> {
        self.list_users()
            .await?
            .into_iter()
            .find(|u| u.email.eq_ignore_ascii_case(email))
            .ok_or(ClientError::Api {
                status: StatusCode::NOT_FOUND.as_u16(),
                detail: format!("Benutzer {email} nicht gefunden"),
            })
    }

    pub async fn list_roles(&self) -> Result<Vec<Role>> {
        Self::send(self.get("/roles")).await
    }

    pub async fn role_by_name(&self, name: &str) -> Result<Role> {
        self.list_roles()
            .await?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or(ClientError::Api {
                status: StatusCode::NOT_FOUND.as_u16(),
                detail: format!("Rolle {name} nicht gefunden"),
            })
    }

    pub async fn create_role(&self, body: &NewRole) -> Result<Role> {
        Self::send(self.json(Method::POST, "/roles", body)).await
    }

    pub async fn update_role(&self, id: Uuid, body: &RoleUpdate) -> Result<Role> {
        Self::send(self.json(Method::PATCH, &format!("/roles/{id}"), body)).await
    }

    pub async fn delete_role(&self, id: Uuid) -> Result<()> {
        Self::send_empty(self.req(Method::DELETE, &format!("/roles/{id}"))).await
    }

    pub async fn list_permissions(&self) -> Result<Vec<PermissionInfo>> {
        Self::send(self.get("/permissions")).await
    }

    pub async fn list_tokens(&self) -> Result<Vec<ApiToken>> {
        Self::send(self.get("/me/tokens")).await
    }

    pub async fn create_token(&self, body: &NewToken) -> Result<CreatedToken> {
        Self::send(self.json(Method::POST, "/me/tokens", body)).await
    }

    pub async fn revoke_token(&self, id: Uuid) -> Result<()> {
        Self::send_empty(self.req(Method::DELETE, &format!("/me/tokens/{id}"))).await
    }

    pub async fn list_audit(&self, limit: i64, before: Option<i64>) -> Result<AuditPage> {
        let mut rb = self.get("/audit").query(&[("limit", limit)]);
        if let Some(b) = before {
            rb = rb.query(&[("before", b)]);
        }
        Self::send(rb).await
    }
}
