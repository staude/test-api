use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Result;
use crate::models::SessionLookup;

pub async fn insert(
    pool: &PgPool,
    id_hash: &[u8],
    user_id: Uuid,
    csrf_token: &str,
    expires_at: OffsetDateTime,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO sessions (id_hash, user_id, csrf_token, expires_at, user_agent, ip)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id_hash)
    .bind(user_id)
    .bind(csrf_token)
    .bind(expires_at)
    .bind(user_agent)
    .bind(ip)
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn find_valid(pool: &PgPool, id_hash: &[u8]) -> Result<Option<SessionLookup>> {
    sqlx::query_as(
        "SELECT user_id, csrf_token, expires_at FROM sessions
         WHERE id_hash = $1 AND expires_at > now()",
    )
    .bind(id_hash)
    .fetch_optional(pool)
    .await
}

pub async fn extend(pool: &PgPool, id_hash: &[u8], expires_at: OffsetDateTime) -> Result<()> {
    sqlx::query("UPDATE sessions SET expires_at = $2 WHERE id_hash = $1")
        .bind(id_hash)
        .bind(expires_at)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn delete(pool: &PgPool, id_hash: &[u8]) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id_hash = $1")
        .bind(id_hash)
        .execute(pool)
        .await
        .map(|_| ())
}

pub async fn delete_for_user(ex: impl sqlx::PgExecutor<'_>, user_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(ex)
        .await
        .map(|_| ())
}

pub async fn delete_expired(pool: &PgPool) -> Result<u64> {
    let res = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
