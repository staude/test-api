use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum_extra::extract::{CookieJar, Form};
use serde::Deserialize;
use test_core::dto::*;
use test_core::{Channel, Error, Permission};
use test_rest::ClientMeta;
use test_rest::auth::{removal_cookie, session_cookie};
use uuid::Uuid;

use crate::views::*;
use crate::{WebError, WebResult, WebSession, WebState, render};

pub fn routes() -> Router<WebState> {
    Router::new()
        .route("/", get(|| async { Redirect::to("/notes") }))
        .route("/login", get(login_form).post(login))
        .route("/logout", post(logout))
        .route("/setup", get(setup_form).post(setup))
        .route("/notes", get(notes).post(create_note))
        .route("/notes/new", get(new_note))
        .route("/notes/{id}", get(note).post(update_note))
        .route("/notes/{id}/edit", get(edit_note))
        .route("/notes/{id}/delete", post(delete_note))
        .route("/profile", get(profile))
        .route("/profile/password", post(change_password))
        .route("/profile/tokens", post(create_token))
        .route("/profile/tokens/{id}/revoke", post(revoke_token))
        .route("/admin/users", get(users).post(create_user))
        .route("/admin/users/{id}", get(user).post(update_user))
        .route("/admin/users/{id}/roles", post(set_user_roles))
        .route("/admin/roles", get(roles).post(create_role))
        .route("/admin/roles/{id}", get(role).post(update_role))
        .route("/admin/roles/{id}/delete", post(delete_role))
        .route("/admin/audit", get(audit))
}

#[derive(Deserialize)]
struct Csrf {
    _csrf: String,
}

#[derive(Deserialize, Default)]
struct Msg {
    msg: Option<String>,
}

/// Only known message codes are shown, nothing from the URL is reflected.
fn message(code: Option<&str>) -> Option<String> {
    let text = match code? {
        "saved" => "Gespeichert.",
        "created" => "Angelegt.",
        "password" => "Passwort geändert.",
        "revoked" => "Token widerrufen.",
        _ => return None,
    };
    Some(text.into())
}

fn see_other(to: impl AsRef<str>) -> Response {
    Redirect::to(to.as_ref()).into_response()
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------- login

async fn login_form(State(s): State<WebState>) -> WebResult<Html<String>> {
    render(LoginPage {
        layout: Layout::anonymous(),
        error: None,
        email: String::new(),
        setup_pending: s.core.setup_pending(),
    })
}

#[derive(Deserialize)]
struct LoginForm {
    email: String,
    password: String,
}

async fn login(
    State(s): State<WebState>,
    meta: ClientMeta,
    jar: CookieJar,
    Form(f): Form<LoginForm>,
) -> WebResult<Response> {
    let res = s
        .core
        .authenticate_password(&f.email, &f.password, Channel::Web, meta.ip.as_deref())
        .await;
    match res {
        Ok(p) => {
            let info = s
                .core
                .create_session(p.user_id, meta.user_agent.as_deref(), meta.ip.as_deref())
                .await?;
            let jar = jar.add(session_cookie(&info, s.cookie_secure));
            Ok((jar, Redirect::to("/notes")).into_response())
        }
        Err(e @ (Error::InvalidCredentials | Error::RateLimited)) => Ok(render(LoginPage {
            layout: Layout::anonymous(),
            error: Some(e.to_string()),
            email: f.email,
            setup_pending: s.core.setup_pending(),
        })?
        .into_response()),
        Err(e) => Err(e.into()),
    }
}

async fn logout(
    State(s): State<WebState>,
    session: WebSession,
    jar: CookieJar,
    Form(f): Form<Csrf>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core
        .logout_session(&session.principal, &session.cookie)
        .await?;
    Ok((jar.remove(removal_cookie()), Redirect::to("/login")).into_response())
}

#[derive(Deserialize)]
struct SetupQuery {
    code: Option<String>,
}

async fn setup_form(State(s): State<WebState>, Query(q): Query<SetupQuery>) -> WebResult<Response> {
    if !s.core.setup_pending() {
        return Ok(see_other("/login"));
    }
    Ok(render(SetupPage {
        layout: Layout::anonymous(),
        code: q.code.unwrap_or_default(),
        error: None,
    })?
    .into_response())
}

#[derive(Deserialize)]
struct SetupForm {
    code: String,
    email: String,
    display_name: String,
    password: String,
}

async fn setup(
    State(s): State<WebState>,
    meta: ClientMeta,
    jar: CookieJar,
    Form(f): Form<SetupForm>,
) -> WebResult<Response> {
    let user = NewUser {
        email: f.email,
        display_name: f.display_name,
        password: f.password,
        roles: vec![],
    };
    match s.core.complete_setup(&f.code, user).await {
        Ok(u) => {
            let info = s
                .core
                .create_session(u.id, meta.user_agent.as_deref(), meta.ip.as_deref())
                .await?;
            Ok((
                jar.add(session_cookie(&info, s.cookie_secure)),
                Redirect::to("/notes"),
            )
                .into_response())
        }
        Err(e @ (Error::Validation(_) | Error::Forbidden(_) | Error::Conflict(_))) => {
            Ok(render(SetupPage {
                layout: Layout::anonymous(),
                code: f.code,
                error: Some(match e {
                    Error::Forbidden(_) => "Setup-Code ungültig".into(),
                    other => other.to_string(),
                }),
            })?
            .into_response())
        }
        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------- notes

#[derive(Deserialize, Default)]
struct NotesQuery {
    q: Option<String>,
    cursor: Option<Uuid>,
}

async fn notes(
    State(s): State<WebState>,
    session: WebSession,
    Query(q): Query<NotesQuery>,
) -> WebResult<Html<String>> {
    let p = &session.principal;
    let page = s
        .core
        .list_notes(
            p,
            NoteQuery {
                q: q.q.clone(),
                limit: Some(20),
                cursor: q.cursor,
            },
        )
        .await?;
    let notes = page
        .items
        .into_iter()
        .map(|n| {
            let can_edit = p.can_on(Permission::NotesWrite, n.owner_id);
            NoteView::new(n, can_edit)
        })
        .collect();
    render(NotesPage {
        layout: session.layout("notes"),
        notes,
        q: q.q.unwrap_or_default(),
        next_cursor: page.next_cursor.map(|c| c.to_string()),
        can_create: p.has_any_scope(Permission::NotesWrite),
        scope_hint: if p.has(Permission::NotesRead) {
            "Alle Notizen"
        } else {
            "Deine Notizen"
        },
    })
}

async fn note(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
) -> WebResult<Html<String>> {
    let p = &session.principal;
    let n = s.core.get_note(p, id).await?;
    let can_edit = p.can_on(Permission::NotesWrite, n.owner_id);
    render(NoteDetailPage {
        layout: session.layout("notes"),
        note: NoteView::new(n, can_edit),
    })
}

async fn new_note(session: WebSession) -> WebResult<Html<String>> {
    session
        .principal
        .has_any_scope(Permission::NotesWrite)
        .then_some(())
        .ok_or(WebError(Error::Forbidden(
            Permission::NotesWrite.key().into(),
        )))?;
    render(NoteFormPage {
        layout: session.layout("notes"),
        id: None,
        title: String::new(),
        body: String::new(),
    })
}

async fn edit_note(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
) -> WebResult<Html<String>> {
    let p = &session.principal;
    let n = s.core.get_note(p, id).await?;
    if !p.can_on(Permission::NotesWrite, n.owner_id) {
        return Err(Error::Forbidden(Permission::NotesWrite.key().into()).into());
    }
    render(NoteFormPage {
        layout: session.layout("notes"),
        id: Some(n.id.to_string()),
        title: n.title,
        body: n.body,
    })
}

#[derive(Deserialize)]
struct NoteForm {
    _csrf: String,
    title: String,
    #[serde(default)]
    body: String,
}

async fn create_note(
    State(s): State<WebState>,
    session: WebSession,
    Form(f): Form<NoteForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    let n = s
        .core
        .create_note(
            &session.principal,
            NewNote {
                title: f.title,
                body: f.body,
            },
        )
        .await?;
    Ok(see_other(format!("/notes/{}", n.id)))
}

async fn update_note(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<NoteForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core
        .update_note(
            &session.principal,
            id,
            NoteUpdate {
                title: Some(f.title),
                body: Some(f.body),
            },
        )
        .await?;
    Ok(see_other(format!("/notes/{id}")))
}

async fn delete_note(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<Csrf>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core.delete_note(&session.principal, id).await?;
    Ok(see_other("/notes"))
}

// ---------------------------------------------------------------- profile

async fn profile_page(
    s: &WebState,
    session: &WebSession,
    new_secret: Option<String>,
    message: Option<String>,
) -> WebResult<Html<String>> {
    let p = &session.principal;
    let can_tokens = p.has(Permission::TokensManageOwn);
    let tokens = if can_tokens {
        s.core.list_tokens(p).await?
    } else {
        vec![]
    };
    render(ProfilePage {
        layout: session.layout("profile"),
        email: p.email.clone(),
        roles: p.roles.join(", "),
        permissions: p.permissions.iter().map(|x| x.key().to_owned()).collect(),
        tokens: tokens.into_iter().map(TokenView::from).collect(),
        can_tokens,
        new_secret,
        message,
    })
}

async fn profile(
    State(s): State<WebState>,
    session: WebSession,
    Query(m): Query<Msg>,
) -> WebResult<Html<String>> {
    profile_page(&s, &session, None, message(m.msg.as_deref())).await
}

#[derive(Deserialize)]
struct PasswordForm {
    _csrf: String,
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(s): State<WebState>,
    session: WebSession,
    Form(f): Form<PasswordForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core
        .change_own_password(
            &session.principal,
            ChangePassword {
                current_password: f.current_password,
                new_password: f.new_password,
            },
        )
        .await?;
    Ok(see_other("/profile?msg=password"))
}

#[derive(Deserialize)]
struct TokenForm {
    _csrf: String,
    name: String,
    /// Selected permissions; empty means "all of mine".
    #[serde(default)]
    scopes: Vec<String>,
    expires_in_days: Option<String>,
}

async fn create_token(
    State(s): State<WebState>,
    session: WebSession,
    Form(f): Form<TokenForm>,
) -> WebResult<Html<String>> {
    session.check_csrf(&f._csrf)?;
    let expires_in_days = non_empty(f.expires_in_days)
        .map(|d| {
            d.parse::<u32>()
                .map_err(|_| Error::Validation("Laufzeit muss eine Zahl sein".into()))
        })
        .transpose()?;
    let created = s
        .core
        .create_token(
            &session.principal,
            NewToken {
                name: f.name,
                scopes: (!f.scopes.is_empty()).then_some(f.scopes),
                expires_in_days,
            },
        )
        .await?;
    profile_page(&s, &session, Some(created.secret), None).await
}

async fn revoke_token(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<Csrf>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core.revoke_token(&session.principal, id).await?;
    Ok(see_other("/profile?msg=revoked"))
}

// ---------------------------------------------------------------- users

async fn users(State(s): State<WebState>, session: WebSession) -> WebResult<Html<String>> {
    let p = &session.principal;
    let users = s.core.list_users(p).await?;
    let roles = s.core.list_roles(p).await?;
    render(UsersPage {
        layout: session.layout("users"),
        users: users.into_iter().map(UserView::from).collect(),
        roles,
        can_manage: p.has(Permission::UsersManage),
    })
}

#[derive(Deserialize)]
struct NewUserForm {
    _csrf: String,
    email: String,
    display_name: String,
    password: String,
    #[serde(default)]
    roles: Vec<String>,
}

async fn create_user(
    State(s): State<WebState>,
    session: WebSession,
    Form(f): Form<NewUserForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    let u = s
        .core
        .create_user(
            &session.principal,
            NewUser {
                email: f.email,
                display_name: f.display_name,
                password: f.password,
                roles: f.roles,
            },
        )
        .await?;
    Ok(see_other(format!("/admin/users/{}?msg=created", u.id)))
}

async fn user(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Query(m): Query<Msg>,
) -> WebResult<Html<String>> {
    let p = &session.principal;
    p.require(Permission::UsersRead)?;
    let u = s.core.get_user(p, id).await?;
    let roles = s
        .core
        .list_roles(p)
        .await?
        .into_iter()
        .map(|r| RoleChoice {
            checked: u.roles.contains(&r.name),
            name: r.name,
            description: r.description,
        })
        .collect();
    render(UserDetailPage {
        layout: session.layout("users"),
        is_self: u.id == p.user_id,
        user: UserView::from(u),
        roles,
        can_manage: p.has(Permission::UsersManage),
        message: message(m.msg.as_deref()),
    })
}

#[derive(Deserialize)]
struct UserForm {
    _csrf: String,
    display_name: String,
    /// Checkbox: present when checked.
    is_active: Option<String>,
    password: Option<String>,
}

async fn update_user(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<UserForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core
        .update_user(
            &session.principal,
            id,
            UserUpdate {
                display_name: Some(f.display_name),
                is_active: Some(f.is_active.is_some()),
                password: non_empty(f.password),
            },
        )
        .await?;
    Ok(see_other(format!("/admin/users/{id}?msg=saved")))
}

#[derive(Deserialize)]
struct RolesForm {
    _csrf: String,
    #[serde(default)]
    roles: Vec<String>,
}

async fn set_user_roles(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<RolesForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core
        .set_user_roles(&session.principal, id, f.roles)
        .await?;
    Ok(see_other(format!("/admin/users/{id}?msg=saved")))
}

// ---------------------------------------------------------------- roles

async fn roles(State(s): State<WebState>, session: WebSession) -> WebResult<Html<String>> {
    let p = &session.principal;
    p.require(Permission::RolesManage)?;
    render(RolesPage {
        layout: session.layout("roles"),
        roles: s.core.list_roles(p).await?,
        permissions: s.core.list_permissions(p).await?,
    })
}

#[derive(Deserialize)]
struct RoleForm {
    _csrf: String,
    name: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    permissions: Vec<String>,
}

async fn create_role(
    State(s): State<WebState>,
    session: WebSession,
    Form(f): Form<RoleForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    let r = s
        .core
        .create_role(
            &session.principal,
            NewRole {
                name: f.name.unwrap_or_default(),
                description: f.description,
                permissions: f.permissions,
            },
        )
        .await?;
    Ok(see_other(format!("/admin/roles/{}?msg=created", r.id)))
}

async fn role(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Query(m): Query<Msg>,
) -> WebResult<Html<String>> {
    let p = &session.principal;
    p.require(Permission::RolesManage)?;
    let role = s.core.get_role(p, id).await?;
    let permissions = s
        .core
        .list_permissions(p)
        .await?
        .into_iter()
        .map(|perm| PermChoice {
            checked: role.permissions.contains(&perm.key),
            key: perm.key,
            description: perm.description,
        })
        .collect();
    render(RoleDetailPage {
        layout: session.layout("roles"),
        role,
        permissions,
        message: message(m.msg.as_deref()),
    })
}

async fn update_role(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<RoleForm>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core
        .update_role(
            &session.principal,
            id,
            RoleUpdate {
                description: Some(f.description),
                permissions: Some(f.permissions),
            },
        )
        .await?;
    Ok(see_other(format!("/admin/roles/{id}?msg=saved")))
}

async fn delete_role(
    State(s): State<WebState>,
    session: WebSession,
    Path(id): Path<Uuid>,
    Form(f): Form<Csrf>,
) -> WebResult<Response> {
    session.check_csrf(&f._csrf)?;
    s.core.delete_role(&session.principal, id).await?;
    Ok(see_other("/admin/roles"))
}

// ---------------------------------------------------------------- audit

#[derive(Deserialize, Default)]
struct AuditParams {
    before: Option<i64>,
}

async fn audit(
    State(s): State<WebState>,
    session: WebSession,
    Query(q): Query<AuditParams>,
) -> WebResult<Html<String>> {
    let page = s
        .core
        .list_audit(
            &session.principal,
            AuditQuery {
                limit: Some(50),
                before: q.before,
            },
        )
        .await?;
    render(AuditLogPage {
        layout: session.layout("audit"),
        entries: page.items.into_iter().map(AuditView::from).collect(),
        next_cursor: page.next_cursor,
    })
}
