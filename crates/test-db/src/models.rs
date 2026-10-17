use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub is_active: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_login_at: Option<OffsetDateTime>,
    /// Names of the roles assigned to the user.
    pub roles: Vec<String>,
}

/// Data needed to verify a login. Never leaves the core.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Credentials {
    pub id: Uuid,
    pub password_hash: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Role {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// System roles are created by the application and cannot be changed.
    pub is_system: bool,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct PermissionInfo {
    pub key: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Note {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_name: String,
    pub title: String,
    pub body: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ApiToken {
    pub id: Uuid,
    pub name: String,
    /// Visible start of the token, to recognise it later.
    pub prefix: String,
    /// `null` means the token carries all permissions of its user.
    pub scopes: Option<Vec<String>>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub revoked_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenLookup {
    pub id: Uuid,
    pub user_id: Uuid,
    pub scopes: Option<Vec<String>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionLookup {
    pub user_id: Uuid,
    pub csrf_token: String,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct AuditEntry {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub actor_id: Option<Uuid>,
    pub actor_email: Option<String>,
    pub channel: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    #[schema(value_type = Object)]
    pub details: serde_json::Value,
}

/// Identity data of a user plus their effective role permissions.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Identity {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub is_active: bool,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}
