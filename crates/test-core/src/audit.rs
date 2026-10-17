use serde_json::Value;

use crate::dto::{AuditPage, AuditQuery};
use crate::{Core, Permission, Principal, Result};

impl Core {
    /// Writes an audit entry. Failures are logged, never propagated: a full
    /// audit table must not block the actual operation.
    pub(crate) async fn audit(
        &self,
        p: &Principal,
        action: &str,
        target: Option<(&str, String)>,
        mut details: Value,
    ) {
        // Record which token acted, so a leaked token can be traced.
        if let (Some(token_id), Some(obj)) = (p.token_id, details.as_object_mut()) {
            obj.insert("token_id".into(), Value::String(token_id.to_string()));
        }
        self.audit_raw(p.actor_id(), p.channel.as_str(), action, target, details)
            .await;
    }

    /// Like [`Core::audit`], but only if read auditing is enabled.
    pub(crate) async fn audit_read(
        &self,
        p: &Principal,
        action: &str,
        target: Option<(&str, String)>,
        details: Value,
    ) {
        if self.inner.options.audit_reads {
            self.audit(p, action, target, details).await;
        }
    }

    pub(crate) async fn audit_raw(
        &self,
        actor_id: Option<uuid::Uuid>,
        channel: &str,
        action: &str,
        target: Option<(&str, String)>,
        details: Value,
    ) {
        let (target_type, target_id) = match target {
            Some((t, id)) => (Some(t), Some(id)),
            None => (None, None),
        };
        let entry = test_db::audit::NewEntry {
            actor_id,
            channel,
            action,
            target_type,
            target_id,
            details,
        };
        if let Err(e) = test_db::audit::insert(&self.pool, entry).await {
            tracing::warn!(error = %e, action, "audit log write failed");
        }
    }

    pub async fn list_audit(&self, p: &Principal, q: AuditQuery) -> Result<AuditPage> {
        p.require(Permission::AuditRead)?;
        let limit = q.limit.unwrap_or(50).clamp(1, 200);
        let items = test_db::audit::list(&self.pool, q.before, limit).await?;
        let next_cursor = (items.len() as i64 == limit)
            .then(|| items.last().map(|e| e.id))
            .flatten();
        Ok(AuditPage { items, next_cursor })
    }
}
