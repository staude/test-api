//! Template structs and view models. Formatting happens here, templates
//! only display prepared strings.

use askama::Template;
use test_core::dto::{ApiToken, AuditEntry, Note, PermissionInfo, Role, User};
use time::OffsetDateTime;
use time::macros::format_description;

pub fn fmt_dt(dt: OffsetDateTime) -> String {
    dt.format(format_description!("[day].[month].[year] [hour]:[minute]"))
        .unwrap_or_default()
}

fn fmt_opt(dt: Option<OffsetDateTime>) -> String {
    dt.map(fmt_dt).unwrap_or_else(|| "–".into())
}

pub struct Layout {
    pub signed_in: bool,
    pub display_name: String,
    pub csrf: String,
    pub active: &'static str,
    pub can_users: bool,
    pub can_roles: bool,
    pub can_audit: bool,
}

impl Layout {
    pub fn anonymous() -> Self {
        Self {
            signed_in: false,
            display_name: String::new(),
            csrf: String::new(),
            active: "",
            can_users: false,
            can_roles: false,
            can_audit: false,
        }
    }
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorPage {
    pub layout: Layout,
    pub status: u16,
    pub message: String,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginPage {
    pub layout: Layout,
    pub error: Option<String>,
    pub email: String,
    pub setup_pending: bool,
}

#[derive(Template)]
#[template(path = "setup.html")]
pub struct SetupPage {
    pub layout: Layout,
    pub code: String,
    pub error: Option<String>,
}

pub struct NoteView {
    pub id: String,
    pub title: String,
    pub body: String,
    pub excerpt: String,
    pub owner_name: String,
    pub created: String,
    pub updated: String,
    pub can_edit: bool,
}

impl NoteView {
    pub fn new(n: Note, can_edit: bool) -> Self {
        let excerpt: String = n.body.chars().take(160).collect();
        Self {
            id: n.id.to_string(),
            excerpt: if n.body.chars().count() > 160 {
                format!("{excerpt}…")
            } else {
                excerpt
            },
            title: n.title,
            body: n.body,
            owner_name: n.owner_name,
            created: fmt_dt(n.created_at),
            updated: fmt_dt(n.updated_at),
            can_edit,
        }
    }
}

#[derive(Template)]
#[template(path = "notes_list.html")]
pub struct NotesPage {
    pub layout: Layout,
    pub notes: Vec<NoteView>,
    pub q: String,
    pub next_cursor: Option<String>,
    pub can_create: bool,
    pub scope_hint: &'static str,
}

#[derive(Template)]
#[template(path = "note_detail.html")]
pub struct NoteDetailPage {
    pub layout: Layout,
    pub note: NoteView,
}

#[derive(Template)]
#[template(path = "note_form.html")]
pub struct NoteFormPage {
    pub layout: Layout,
    /// `None` for a new note.
    pub id: Option<String>,
    pub title: String,
    pub body: String,
}

pub struct TokenView {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub scopes: String,
    pub created: String,
    pub expires: String,
    pub last_used: String,
    pub status: &'static str,
    pub active: bool,
}

impl From<ApiToken> for TokenView {
    fn from(t: ApiToken) -> Self {
        let now = OffsetDateTime::now_utc();
        let (status, active) = if t.revoked_at.is_some() {
            ("widerrufen", false)
        } else if t.expires_at.is_some_and(|e| e <= now) {
            ("abgelaufen", false)
        } else {
            ("aktiv", true)
        };
        Self {
            id: t.id.to_string(),
            name: t.name,
            prefix: t.prefix,
            scopes: t
                .scopes
                .map(|s| {
                    if s.is_empty() {
                        "keine".into()
                    } else {
                        s.join(", ")
                    }
                })
                .unwrap_or_else(|| "alle eigenen Rechte".into()),
            created: fmt_dt(t.created_at),
            expires: fmt_opt(t.expires_at),
            last_used: fmt_opt(t.last_used_at),
            status,
            active,
        }
    }
}

#[derive(Template)]
#[template(path = "profile.html")]
pub struct ProfilePage {
    pub layout: Layout,
    pub email: String,
    pub roles: String,
    pub permissions: Vec<String>,
    pub tokens: Vec<TokenView>,
    pub can_tokens: bool,
    /// Plain text of a freshly created token, shown exactly once.
    pub new_secret: Option<String>,
    pub message: Option<String>,
}

pub struct UserView {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub roles: String,
    pub is_active: bool,
    pub last_login: String,
    pub created: String,
}

impl From<User> for UserView {
    fn from(u: User) -> Self {
        Self {
            id: u.id.to_string(),
            email: u.email,
            display_name: u.display_name,
            roles: if u.roles.is_empty() {
                "–".into()
            } else {
                u.roles.join(", ")
            },
            is_active: u.is_active,
            last_login: fmt_opt(u.last_login_at),
            created: fmt_dt(u.created_at),
        }
    }
}

#[derive(Template)]
#[template(path = "users_list.html")]
pub struct UsersPage {
    pub layout: Layout,
    pub users: Vec<UserView>,
    pub roles: Vec<Role>,
    pub can_manage: bool,
}

pub struct RoleChoice {
    pub name: String,
    pub description: String,
    pub checked: bool,
}

#[derive(Template)]
#[template(path = "user_detail.html")]
pub struct UserDetailPage {
    pub layout: Layout,
    pub user: UserView,
    pub roles: Vec<RoleChoice>,
    pub can_manage: bool,
    pub is_self: bool,
    pub message: Option<String>,
}

#[derive(Template)]
#[template(path = "roles_list.html")]
pub struct RolesPage {
    pub layout: Layout,
    pub roles: Vec<Role>,
    pub permissions: Vec<PermissionInfo>,
}

pub struct PermChoice {
    pub key: String,
    pub description: String,
    pub checked: bool,
}

#[derive(Template)]
#[template(path = "role_detail.html")]
pub struct RoleDetailPage {
    pub layout: Layout,
    pub role: Role,
    pub permissions: Vec<PermChoice>,
    pub message: Option<String>,
}

pub struct AuditView {
    pub at: String,
    pub actor: String,
    pub channel: String,
    pub action: String,
    pub target: String,
    pub details: String,
}

impl From<AuditEntry> for AuditView {
    fn from(e: AuditEntry) -> Self {
        let details = if e.details.as_object().is_some_and(|o| o.is_empty()) {
            String::new()
        } else {
            e.details.to_string()
        };
        Self {
            at: fmt_dt(e.at),
            actor: e.actor_email.unwrap_or_else(|| "System".into()),
            channel: e.channel,
            action: e.action,
            target: match (e.target_type, e.target_id) {
                (Some(t), Some(id)) => format!("{t}:{}", id.chars().take(8).collect::<String>()),
                _ => String::new(),
            },
            details,
        }
    }
}

#[derive(Template)]
#[template(path = "audit.html")]
pub struct AuditLogPage {
    pub layout: Layout,
    pub entries: Vec<AuditView>,
    pub next_cursor: Option<i64>,
}
