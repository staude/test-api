use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use crate::Result;
use crate::models::{PermissionInfo, Role};

const SELECT_ROLE: &str = "
    SELECT r.id, r.name, r.description, r.is_system,
           coalesce(array_agg(rp.permission_key ORDER BY rp.permission_key)
                    FILTER (WHERE rp.permission_key IS NOT NULL), '{}') AS permissions
    FROM roles r
    LEFT JOIN role_permissions rp ON rp.role_id = r.id";

pub async fn list(pool: &PgPool) -> Result<Vec<Role>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_ROLE} GROUP BY r.id ORDER BY r.is_system DESC, r.name"
    )))
    .fetch_all(pool)
    .await
}

pub async fn get(ex: impl PgExecutor<'_>, id: Uuid) -> Result<Option<Role>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_ROLE} WHERE r.id = $1 GROUP BY r.id"
    )))
    .bind(id)
    .fetch_optional(ex)
    .await
}

pub async fn get_by_name(ex: impl PgExecutor<'_>, name: &str) -> Result<Option<Role>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{SELECT_ROLE} WHERE r.name = $1 GROUP BY r.id"
    )))
    .bind(name)
    .fetch_optional(ex)
    .await
}

/// Returns the subset of `names` that exist as roles.
pub async fn existing_names(pool: &PgPool, names: &[String]) -> Result<Vec<String>> {
    sqlx::query_scalar("SELECT name FROM roles WHERE name = ANY($1)")
        .bind(names)
        .fetch_all(pool)
        .await
}

pub async fn insert(
    ex: impl PgExecutor<'_>,
    name: &str,
    description: &str,
    is_system: bool,
) -> Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO roles (name, description, is_system) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(description)
    .bind(is_system)
    .fetch_one(ex)
    .await
}

pub async fn update_description(
    ex: impl PgExecutor<'_>,
    id: Uuid,
    description: &str,
) -> Result<()> {
    sqlx::query("UPDATE roles SET description = $2 WHERE id = $1")
        .bind(id)
        .bind(description)
        .execute(ex)
        .await
        .map(|_| ())
}

pub async fn set_permissions(ex: &mut sqlx::PgConnection, id: Uuid, keys: &[String]) -> Result<()> {
    sqlx::query("DELETE FROM role_permissions WHERE role_id = $1")
        .bind(id)
        .execute(&mut *ex)
        .await?;
    sqlx::query(
        "INSERT INTO role_permissions (role_id, permission_key)
         SELECT $1, key FROM permissions WHERE key = ANY($2)",
    )
    .bind(id)
    .bind(keys)
    .execute(&mut *ex)
    .await?;
    Ok(())
}

pub async fn delete(ex: impl PgExecutor<'_>, id: Uuid) -> Result<bool> {
    let res = sqlx::query("DELETE FROM roles WHERE id = $1 AND NOT is_system")
        .bind(id)
        .execute(ex)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_permissions(pool: &PgPool) -> Result<Vec<PermissionInfo>> {
    sqlx::query_as("SELECT key, description FROM permissions ORDER BY key")
        .fetch_all(pool)
        .await
}

/// Inserts or updates a permission and removes keys no longer known.
pub async fn sync_permissions(ex: &mut sqlx::PgConnection, perms: &[(&str, &str)]) -> Result<()> {
    let keys: Vec<String> = perms.iter().map(|(k, _)| (*k).to_owned()).collect();
    for (key, description) in perms {
        sqlx::query(
            "INSERT INTO permissions (key, description) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET description = EXCLUDED.description",
        )
        .bind(key)
        .bind(description)
        .execute(&mut *ex)
        .await?;
    }
    sqlx::query("DELETE FROM permissions WHERE NOT (key = ANY($1))")
        .bind(&keys)
        .execute(&mut *ex)
        .await?;
    Ok(())
}
