use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;
use crate::models::AuditEntry;

pub struct NewEntry<'a> {
    pub actor_id: Option<Uuid>,
    pub channel: &'a str,
    pub action: &'a str,
    pub target_type: Option<&'a str>,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
}

pub async fn insert(ex: impl PgExecutor<'_>, e: NewEntry<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor_id, channel, action, target_type, target_id, details)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(e.actor_id)
    .bind(e.channel)
    .bind(e.action)
    .bind(e.target_type)
    .bind(e.target_id)
    .bind(e.details)
    .execute(ex)
    .await
    .map(|_| ())
}

/// Newest entries first; `before` is an id cursor.
pub async fn list(pool: &PgPool, before: Option<i64>, limit: i64) -> Result<Vec<AuditEntry>> {
    sqlx::query_as(
        "SELECT a.id, a.at, a.actor_id, u.email::text AS actor_email, a.channel, a.action,
                a.target_type, a.target_id, a.details
         FROM audit_log a LEFT JOIN users u ON u.id = a.actor_id
         WHERE ($1::bigint IS NULL OR a.id < $1)
         ORDER BY a.id DESC
         LIMIT $2",
    )
    .bind(before)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Deletes entries older than `days` days. Returns the number removed.
pub async fn delete_older_than(pool: &PgPool, days: u32) -> Result<u64> {
    let res = sqlx::query("DELETE FROM audit_log WHERE at < now() - make_interval(days => $1)")
        .bind(i32::try_from(days).unwrap_or(i32::MAX))
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
