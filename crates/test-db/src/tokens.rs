use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Result;
use crate::models::{ApiToken, TokenLookup};

const SELECT_TOKEN: &str = "
    SELECT id, name, prefix, scopes, created_at, expires_at, last_used_at, revoked_at
    FROM api_tokens";

pub async fn list_for_user(pool: &PgPool, user_id: Uuid) -> Result<Vec<ApiToken>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_TOKEN} WHERE user_id = $1 ORDER BY created_at DESC"
    )))
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn get_for_user(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<Option<ApiToken>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_TOKEN} WHERE user_id = $1 AND id = $2"
    )))
    .bind(user_id)
    .bind(id)
    .fetch_optional(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    prefix: &str,
    token_hash: &[u8],
    scopes: Option<&[String]>,
    expires_at: Option<OffsetDateTime>,
) -> Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO api_tokens (user_id, name, prefix, token_hash, scopes, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(user_id)
    .bind(name)
    .bind(prefix)
    .bind(token_hash)
    .bind(scopes)
    .bind(expires_at)
    .fetch_one(pool)
    .await
}

/// Finds an active token and records its use.
pub async fn use_token(pool: &PgPool, token_hash: &[u8]) -> Result<Option<TokenLookup>> {
    sqlx::query_as(
        "UPDATE api_tokens SET last_used_at = now()
         WHERE token_hash = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())
         RETURNING id, user_id, scopes",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub async fn revoke(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE api_tokens SET revoked_at = now()
         WHERE user_id = $1 AND id = $2 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn revoke_all_for_user(ex: impl sqlx::PgExecutor<'_>, user_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(ex)
    .await
    .map(|_| ())
}
