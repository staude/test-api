//! Service layer: all use cases, validation and permission checks.
//!
//! REST, MCP, Web and the admin CLI are thin adapters around [`Core`].

mod audit;
mod auth;
pub mod crypto;
pub mod dto;
pub mod error;
mod limiter;
mod notes;
pub mod permission;
pub mod principal;
mod roles;
mod tokens;
mod users;

use std::sync::{Arc, Mutex};

pub use auth::{LoginOutcome, SessionInfo};
pub use error::{Error, Result};
pub use permission::Permission;
pub use principal::{Channel, Principal};
pub use test_db::PgPool;

/// Name of the browser session cookie.
pub const SESSION_COOKIE: &str = "tapp_session";

/// Client IP as determined by the HTTP layer (respecting trusted proxies).
#[derive(Debug, Clone)]
pub struct ClientIp(pub String);

#[derive(Clone)]
pub struct Core {
    pool: PgPool,
    inner: Arc<Inner>,
}

struct Inner {
    limiter: limiter::LoginLimiter,
    /// One-time code for creating the first admin via the web UI.
    setup_code: Mutex<Option<String>>,
    /// Hash used to keep login timing constant for unknown e-mails.
    dummy_hash: String,
    options: CoreOptions,
}

#[derive(Debug, Clone)]
pub struct CoreOptions {
    /// Also record read access (viewing and searching notes, users).
    pub audit_reads: bool,
    /// Delete audit entries older than this many days; `None` keeps them.
    pub audit_retention_days: Option<u32>,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            audit_reads: true,
            audit_retention_days: Some(90),
        }
    }
}

/// What a cleanup run removed.
#[derive(Debug, Default)]
pub struct CleanupReport {
    pub sessions: u64,
    pub audit_entries: u64,
}

pub enum BootstrapOutcome {
    Ready,
    /// No user exists; the first admin can be created with this code.
    NeedsSetup {
        code: String,
    },
}

impl Core {
    pub async fn new(pool: PgPool) -> Result<Self> {
        Self::with_options(pool, CoreOptions::default()).await
    }

    pub async fn with_options(pool: PgPool, options: CoreOptions) -> Result<Self> {
        let dummy_hash = crypto::hash_password(crypto::random_secret()).await?;
        Ok(Self {
            pool,
            inner: Arc::new(Inner {
                limiter: limiter::LoginLimiter::default(),
                setup_code: Mutex::new(None),
                dummy_hash,
                options,
            }),
        })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn ping(&self) -> Result<()> {
        Ok(test_db::ping(&self.pool).await?)
    }

    /// Synchronises permissions and system roles, optionally creates the
    /// first admin. Safe to call on every start.
    pub async fn bootstrap(&self, admin: Option<(String, String)>) -> Result<BootstrapOutcome> {
        let mut tx = self.pool.begin().await?;
        let perms: Vec<(&str, &str)> = Permission::ALL
            .iter()
            .map(|p| (p.key(), p.description()))
            .collect();
        test_db::roles::sync_permissions(&mut tx, &perms).await?;
        for role in &permission::SYSTEM_ROLES {
            let id = match test_db::roles::get_by_name(&mut *tx, role.name).await? {
                Some(r) => r.id,
                None => test_db::roles::insert(&mut *tx, role.name, role.description, true).await?,
            };
            let keys: Vec<String> = role
                .permissions
                .iter()
                .map(|p| p.key().to_owned())
                .collect();
            test_db::roles::set_permissions(&mut tx, id, &keys).await?;
        }
        tx.commit().await?;

        if test_db::users::count(&self.pool).await? > 0 {
            return Ok(BootstrapOutcome::Ready);
        }
        if let Some((email, password)) = admin {
            let display_name = email.split('@').next().unwrap_or("Admin").to_owned();
            self.create_user(
                &Principal::system(),
                dto::NewUser {
                    email,
                    display_name,
                    password,
                    roles: vec!["admin".into()],
                },
            )
            .await?;
            return Ok(BootstrapOutcome::Ready);
        }
        let code = crypto::random_secret();
        if let Ok(mut slot) = self.inner.setup_code.lock() {
            *slot = Some(code.clone());
        }
        Ok(BootstrapOutcome::NeedsSetup { code })
    }

    pub fn setup_pending(&self) -> bool {
        self.inner
            .setup_code
            .lock()
            .map(|s| s.is_some())
            .unwrap_or(false)
    }

    /// Creates the first admin with the one-time setup code.
    pub async fn complete_setup(&self, code: &str, user: dto::NewUser) -> Result<User> {
        let valid = self
            .inner
            .setup_code
            .lock()
            .map(|s| s.as_deref() == Some(code))
            .unwrap_or(false);
        if !valid || test_db::users::count(&self.pool).await? > 0 {
            return Err(Error::Forbidden("setup".into()));
        }
        let user = self
            .create_user(
                &Principal::system(),
                dto::NewUser {
                    roles: vec!["admin".into()],
                    ..user
                },
            )
            .await?;
        if let Ok(mut slot) = self.inner.setup_code.lock() {
            *slot = None;
        }
        Ok(user)
    }

    /// Removes expired sessions and audit entries past the retention period.
    pub async fn cleanup(&self) -> Result<CleanupReport> {
        let sessions = test_db::sessions::delete_expired(&self.pool).await?;
        let audit_entries = match self.inner.options.audit_retention_days {
            Some(days) => test_db::audit::delete_older_than(&self.pool, days).await?,
            None => 0,
        };
        Ok(CleanupReport {
            sessions,
            audit_entries,
        })
    }
}

pub use dto::{ApiToken, AuditEntry, Note, PermissionInfo, Role, User};
