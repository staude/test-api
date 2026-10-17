use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;
use crate::models::{Credentials, Identity, User};

const SELECT_USER: &str = "
    SELECT u.id, u.email::text AS email, u.display_name, u.is_active,
           u.created_at, u.updated_at, u.last_login_at,
           coalesce(array_agg(r.name ORDER BY r.name) FILTER (WHERE r.name IS NOT NULL), '{}') AS roles
    FROM users u
    LEFT JOIN user_roles ur ON ur.user_id = u.id
    LEFT JOIN roles r ON r.id = ur.role_id";

pub async fn count(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await
}

pub async fn list(pool: &PgPool) -> Result<Vec<User>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_USER} GROUP BY u.id ORDER BY u.email"
    )))
    .fetch_all(pool)
    .await
}

pub async fn get(ex: impl PgExecutor<'_>, id: Uuid) -> Result<Option<User>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_USER} WHERE u.id = $1 GROUP BY u.id"
    )))
    .bind(id)
    .fetch_optional(ex)
    .await
}

pub async fn find_id_by_email(pool: &PgPool, email: &str) -> Result<Option<Uuid>> {
    sqlx::query_scalar("SELECT id FROM users WHERE email = $1::citext")
        .bind(email)
        .fetch_optional(pool)
        .await
}

pub async fn credentials_by_email(pool: &PgPool, email: &str) -> Result<Option<Credentials>> {
    sqlx::query_as("SELECT id, password_hash, is_active FROM users WHERE email = $1::citext")
        .bind(email)
        .fetch_optional(pool)
        .await
}

pub async fn credentials_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Credentials>> {
    sqlx::query_as("SELECT id, password_hash, is_active FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Loads a user with the union of all permissions of their roles.
pub async fn identity(pool: &PgPool, id: Uuid) -> Result<Option<Identity>> {
    sqlx::query_as(
        "SELECT u.id, u.email::text AS email, u.display_name, u.is_active,
                coalesce((SELECT array_agg(r.name ORDER BY r.name)
                          FROM user_roles ur JOIN roles r ON r.id = ur.role_id
                          WHERE ur.user_id = u.id), '{}') AS roles,
                coalesce((SELECT array_agg(DISTINCT rp.permission_key)
                          FROM user_roles ur JOIN role_permissions rp ON rp.role_id = ur.role_id
                          WHERE ur.user_id = u.id), '{}') AS permissions
         FROM users u WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn insert(
    ex: impl PgExecutor<'_>,
    email: &str,
    display_name: &str,
    password_hash: &str,
) -> Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO users (email, display_name, password_hash) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(email)
    .bind(display_name)
    .bind(password_hash)
    .fetch_one(ex)
    .await
}

pub async fn update_profile(
    ex: impl PgExecutor<'_>,
    id: Uuid,
    display_name: Option<&str>,
    is_active: Option<bool>,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE users SET display_name = coalesce($2, display_name),
                          is_active = coalesce($3, is_active),
                          updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(display_name)
    .bind(is_active)
    .execute(ex)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn set_password(ex: impl PgExecutor<'_>, id: Uuid, password_hash: &str) -> Result<bool> {
    let res = sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(password_hash)
        .execute(ex)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn touch_login(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE users SET last_login_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Replaces the role assignment of a user. Unknown role names are ignored;
/// callers validate them beforehand.
pub async fn set_roles(ex: &mut sqlx::PgConnection, id: Uuid, role_names: &[String]) -> Result<()> {
    sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
        .bind(id)
        .execute(&mut *ex)
        .await?;
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_id)
         SELECT $1, id FROM roles WHERE name = ANY($2)",
    )
    .bind(id)
    .bind(role_names)
    .execute(&mut *ex)
    .await?;
    Ok(())
}

/// Number of active users that hold the given permission through a role.
pub async fn count_active_with_permission(
    ex: impl PgExecutor<'_>,
    permission: &str,
) -> Result<i64> {
    sqlx::query_scalar(
        "SELECT count(DISTINCT u.id) FROM users u
         JOIN user_roles ur ON ur.user_id = u.id
         JOIN role_permissions rp ON rp.role_id = ur.role_id
         WHERE u.is_active AND rp.permission_key = $1",
    )
    .bind(permission)
    .fetch_one(ex)
    .await
}
