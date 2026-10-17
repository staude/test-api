use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// All permissions known to the application.
///
/// The keys are synchronised into the `permissions` table on startup, so
/// adding a variant here is all it takes to introduce a new permission.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
pub enum Permission {
    #[serde(rename = "notes.read")]
    NotesRead,
    #[serde(rename = "notes.read.own")]
    NotesReadOwn,
    #[serde(rename = "notes.write")]
    NotesWrite,
    #[serde(rename = "notes.write.own")]
    NotesWriteOwn,
    #[serde(rename = "users.read")]
    UsersRead,
    #[serde(rename = "users.manage")]
    UsersManage,
    #[serde(rename = "roles.manage")]
    RolesManage,
    #[serde(rename = "audit.read")]
    AuditRead,
    #[serde(rename = "tokens.manage.own")]
    TokensManageOwn,
}

impl Permission {
    pub const ALL: [Permission; 9] = [
        Permission::NotesRead,
        Permission::NotesReadOwn,
        Permission::NotesWrite,
        Permission::NotesWriteOwn,
        Permission::UsersRead,
        Permission::UsersManage,
        Permission::RolesManage,
        Permission::AuditRead,
        Permission::TokensManageOwn,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Permission::NotesRead => "notes.read",
            Permission::NotesReadOwn => "notes.read.own",
            Permission::NotesWrite => "notes.write",
            Permission::NotesWriteOwn => "notes.write.own",
            Permission::UsersRead => "users.read",
            Permission::UsersManage => "users.manage",
            Permission::RolesManage => "roles.manage",
            Permission::AuditRead => "audit.read",
            Permission::TokensManageOwn => "tokens.manage.own",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Permission::NotesRead => "Alle Notizen lesen",
            Permission::NotesReadOwn => "Eigene Notizen lesen",
            Permission::NotesWrite => "Alle Notizen anlegen, ändern und löschen",
            Permission::NotesWriteOwn => "Eigene Notizen anlegen, ändern und löschen",
            Permission::UsersRead => "Benutzerliste sehen",
            Permission::UsersManage => "Benutzer anlegen, sperren und Rollen zuweisen",
            Permission::RolesManage => "Rollen anlegen und Rechte zuweisen",
            Permission::AuditRead => "Audit-Log einsehen",
            Permission::TokensManageOwn => "Eigene API-Tokens verwalten",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// The `.own` counterpart of a global permission, if there is one.
    pub fn own_variant(self) -> Option<Self> {
        match self {
            Permission::NotesRead => Some(Permission::NotesReadOwn),
            Permission::NotesWrite => Some(Permission::NotesWriteOwn),
            _ => None,
        }
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key())
    }
}

/// Roles that are created on startup and cannot be modified.
pub struct SystemRole {
    pub name: &'static str,
    pub description: &'static str,
    pub permissions: &'static [Permission],
}

pub const SYSTEM_ROLES: [SystemRole; 4] = [
    SystemRole {
        name: "admin",
        description: "Vollzugriff",
        permissions: &Permission::ALL,
    },
    SystemRole {
        name: "editor",
        description: "Alle Notizen lesen und bearbeiten",
        permissions: &[
            Permission::NotesRead,
            Permission::NotesWrite,
            Permission::TokensManageOwn,
        ],
    },
    SystemRole {
        name: "member",
        description: "Nur eigene Notizen",
        permissions: &[
            Permission::NotesReadOwn,
            Permission::NotesWriteOwn,
            Permission::TokensManageOwn,
        ],
    },
    SystemRole {
        name: "viewer",
        description: "Alle Notizen lesen",
        permissions: &[Permission::NotesRead, Permission::TokensManageOwn],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_roundtrip_and_match_serde() {
        for p in Permission::ALL {
            assert_eq!(Permission::from_key(p.key()), Some(p));
            let json = serde_json::to_string(&p).unwrap_or_default();
            assert_eq!(json, format!("\"{}\"", p.key()));
        }
    }
}
