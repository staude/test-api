//! Persistence layer: connection pool, migrations and repository functions.
//!
//! Repository functions do no permission checks. They are only called from
//! `test-core`, which owns all business rules.

pub mod audit;
pub mod models;
pub mod notes;
pub mod roles;
pub mod sessions;
pub mod tokens;
pub mod users;

pub use sqlx::{PgPool, Postgres, Transaction};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn connect(url: &str, max_connections: u32) -> Result<PgPool> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(url)
        .await
}

pub async fn migrate(pool: &PgPool) -> std::result::Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

pub async fn ping(pool: &PgPool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await.map(|_| ())
}

/// True if the error is a unique constraint violation.
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}
