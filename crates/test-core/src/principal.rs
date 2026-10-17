use std::collections::BTreeSet;

use uuid::Uuid;

use crate::error::{Error, Result};
use crate::permission::Permission;

/// Through which interface a request arrived. Recorded in the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Web,
    Api,
    /// The `test-app` CLI client. Self-declared via User-Agent, so it is
    /// only a label for the audit log, never a security decision.
    Cli,
    Mcp,
    CliAdmin,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Web => "web",
            Channel::Api => "api",
            Channel::Cli => "cli",
            Channel::Mcp => "mcp",
            Channel::CliAdmin => "cli-admin",
        }
    }
}

/// Who performs an action and with which effective permissions.
///
/// Every service call takes a `Principal`. Permissions are already the
/// intersection of the user's role permissions and the token scopes.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub roles: Vec<String>,
    pub permissions: BTreeSet<Permission>,
    pub channel: Channel,
    /// Id of the API token used for this request, if any.
    pub token_id: Option<Uuid>,
    /// Set when the request is authenticated with a scoped API token.
    pub token_scoped: bool,
    /// The bootstrap/admin CLI acts without a user account.
    pub system: bool,
}

impl Principal {
    /// Principal for `test-app admin …`: all permissions, no user account.
    pub fn system() -> Self {
        Self {
            user_id: Uuid::nil(),
            email: "system".into(),
            display_name: "System".into(),
            roles: vec![],
            permissions: Permission::ALL.into_iter().collect(),
            channel: Channel::CliAdmin,
            token_id: None,
            token_scoped: false,
            system: true,
        }
    }

    pub fn has(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }

    pub fn require(&self, p: Permission) -> Result<()> {
        if self.has(p) {
            Ok(())
        } else {
            Err(Error::Forbidden(p.key().into()))
        }
    }

    /// True if the principal holds `global`, or its `.own` variant and owns
    /// the resource.
    pub fn can_on(&self, global: Permission, owner_id: Uuid) -> bool {
        self.has(global)
            || (global.own_variant().is_some_and(|own| self.has(own)) && owner_id == self.user_id)
    }

    /// True if the principal holds `global` or at least its `.own` variant.
    pub fn has_any_scope(&self, global: Permission) -> bool {
        self.has(global) || global.own_variant().is_some_and(|own| self.has(own))
    }

    /// The actor id written to the audit log.
    pub fn actor_id(&self) -> Option<Uuid> {
        (!self.system).then_some(self.user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(perms: &[Permission]) -> Principal {
        Principal {
            user_id: Uuid::from_u128(1),
            email: "a@b.c".into(),
            display_name: "A".into(),
            roles: vec![],
            permissions: perms.iter().copied().collect(),
            channel: Channel::Api,
            token_id: None,
            token_scoped: false,
            system: false,
        }
    }

    #[test]
    fn own_permission_only_applies_to_own_resources() {
        let p = with(&[Permission::NotesWriteOwn]);
        assert!(p.can_on(Permission::NotesWrite, Uuid::from_u128(1)));
        assert!(!p.can_on(Permission::NotesWrite, Uuid::from_u128(2)));
        assert!(p.has_any_scope(Permission::NotesWrite));
        assert!(p.require(Permission::NotesWrite).is_err());
    }

    #[test]
    fn global_permission_applies_to_everything() {
        let p = with(&[Permission::NotesRead]);
        assert!(p.can_on(Permission::NotesRead, Uuid::from_u128(2)));
        assert!(!p.can_on(Permission::NotesWrite, Uuid::from_u128(1)));
    }

    #[test]
    fn permissions_without_own_variant() {
        let p = with(&[Permission::UsersRead]);
        assert!(p.has_any_scope(Permission::UsersRead));
        assert!(!p.has_any_scope(Permission::UsersManage));
    }
}
