use sqlx::PgPool;
use uuid::Uuid;

use crate::Result;
use crate::models::Note;

const SELECT_NOTE: &str = "
    SELECT n.id, n.owner_id, u.display_name AS owner_name, n.title, n.body,
           n.created_at, n.updated_at
    FROM notes n JOIN users u ON u.id = n.owner_id";

pub struct ListFilter<'a> {
    /// Restrict to notes of this owner.
    pub owner: Option<Uuid>,
    /// Case-insensitive search in title and body.
    pub search: Option<&'a str>,
    /// Keyset cursor: only notes older than this note.
    pub after: Option<Uuid>,
    pub limit: i64,
}

pub async fn list(pool: &PgPool, filter: ListFilter<'_>) -> Result<Vec<Note>> {
    // The trigram indexes (migration 2) serve rare terms; for frequent terms
    // the planner walks the date index and stops after `limit` hits. Forcing
    // either path (e.g. collecting all hits first) was measured slower.
    // The right choice depends on the search term, so the statement is not
    // cached: a cached generic plan cannot see the term and falls back to a
    // full table scan (measured 128 ms instead of 0.4 ms for a rare term).
    let pattern = filter.search.map(|s| format!("%{}%", escape_like(s)));
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_NOTE}
         WHERE ($1::uuid IS NULL OR n.owner_id = $1)
           AND ($2::text IS NULL OR n.title ILIKE $2 OR n.body ILIKE $2)
           AND ($3::uuid IS NULL OR (n.created_at, n.id) <
                (SELECT created_at, id FROM notes WHERE id = $3))
         ORDER BY n.created_at DESC, n.id DESC
         LIMIT $4"
    )))
    .persistent(false)
    .bind(filter.owner)
    .bind(pattern)
    .bind(filter.after)
    .bind(filter.limit)
    .fetch_all(pool)
    .await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Note>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_NOTE} WHERE n.id = $1"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn insert(pool: &PgPool, owner_id: Uuid, title: &str, body: &str) -> Result<Uuid> {
    sqlx::query_scalar("INSERT INTO notes (owner_id, title, body) VALUES ($1, $2, $3) RETURNING id")
        .bind(owner_id)
        .bind(title)
        .bind(body)
        .fetch_one(pool)
        .await
}

pub async fn update(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE notes SET title = coalesce($2, title), body = coalesce($3, body), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(body)
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM notes WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
