//! Request and response types shared by REST, MCP, Web and the CLI client.

use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use time::OffsetDateTime;

pub use test_db::models::{ApiToken, AuditEntry, Note, PermissionInfo, Role, User};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum LoginMode {
    /// Returns an API token (default, for CLI and scripts).
    #[default]
    Token,
    /// Sets a session cookie and returns the CSRF token (for browsers).
    Session,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub mode: LoginMode,
    /// Name of the created token (only for `mode = token`).
    #[serde(default)]
    pub token_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoginResponse {
    pub me: Me,
    /// The API token in plain text. Shown only once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Must be sent as `X-CSRF-Token` with writing requests in session mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csrf_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Me {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub roles: Vec<String>,
    /// Effective permissions of this request (roles ∩ token scopes).
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChangePassword {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NoteQuery {
    /// Search in title and body.
    pub q: Option<String>,
    /// Page size, 1–100 (default 20).
    pub limit: Option<i64>,
    /// `next_cursor` of the previous page.
    pub cursor: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NotePage {
    pub items: Vec<Note>,
    pub next_cursor: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewNote {
    pub title: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct NoteUpdate {
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewUser {
    pub email: String,
    pub display_name: String,
    pub password: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UserUpdate {
    pub display_name: Option<String>,
    pub is_active: Option<bool>,
    /// Sets a new password and ends all sessions of the user.
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetRoles {
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewRole {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct RoleUpdate {
    pub description: Option<String>,
    pub permissions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewToken {
    pub name: String,
    /// Restrict the token to these permissions. `null` = all of your permissions.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
    #[serde(default)]
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatedToken {
    pub token: ApiToken,
    /// The token in plain text. Shown only once.
    pub secret: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditQuery {
    pub limit: Option<i64>,
    /// `next_cursor` of the previous page.
    pub before: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditPage {
    pub items: Vec<AuditEntry>,
    pub next_cursor: Option<i64>,
}
