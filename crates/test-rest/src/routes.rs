use axum::extract::{FromRequest, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use axum_extra::extract::CookieJar;
use test_core::SESSION_COOKIE;
use test_core::dto::*;
use utoipa::OpenApi;
use uuid::Uuid;

use crate::auth::{ApiPrincipal, ClientMeta, removal_cookie, session_cookie};
use crate::error::{ApiError, ApiResult, Problem};
use crate::{ApiDoc, RestState};

/// JSON body extractor with problem+json rejections.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
pub struct Body<T>(pub T);

pub fn routes() -> Router<RestState> {
    Router::new()
        .route("/openapi.json", get(|| async { Json(ApiDoc::openapi()) }))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/me", get(me))
        .route("/me/password", post(change_password))
        .route("/me/tokens", get(list_tokens).post(create_token))
        .route("/me/tokens/{id}", axum::routing::delete(revoke_token))
        .route("/notes", get(list_notes).post(create_note))
        .route(
            "/notes/{id}",
            get(get_note).patch(update_note).delete(delete_note),
        )
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", get(get_user).patch(update_user))
        .route("/users/{id}/roles", put(set_user_roles))
        .route("/roles", get(list_roles).post(create_role))
        .route(
            "/roles/{id}",
            get(get_role).patch(update_role).delete(delete_role),
        )
        .route("/permissions", get(list_permissions))
        .route("/audit", get(list_audit))
}

// ---------------------------------------------------------------- auth

/// Log in with e-mail and password. Returns an API token (default) or sets a
/// session cookie (`mode = session`).
#[utoipa::path(post, path = "/auth/login", tag = "auth", security(()),
    request_body = LoginRequest,
    responses((status = 200, body = LoginResponse), (status = 401, body = Problem), (status = 429, body = Problem)))]
pub async fn login(
    State(s): State<RestState>,
    meta: ClientMeta,
    headers: HeaderMap,
    jar: CookieJar,
    Body(req): Body<LoginRequest>,
) -> ApiResult<impl IntoResponse> {
    let out = s
        .core
        .login(
            req,
            crate::auth::api_channel(&headers),
            meta.ip.as_deref(),
            meta.user_agent.as_deref(),
        )
        .await?;
    let me = s.core.me(&out.principal);
    let (jar, csrf_token) = match &out.session {
        Some(info) => (
            jar.add(session_cookie(info, s.auth.cookie_secure)),
            Some(info.csrf_token.clone()),
        ),
        None => (jar, None),
    };
    Ok((
        jar,
        Json(LoginResponse {
            me,
            token: out.token,
            csrf_token,
        }),
    ))
}

/// End the current session, or revoke the token used for this request.
#[utoipa::path(post, path = "/auth/logout", tag = "auth", responses((status = 204)))]
pub async fn logout(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    headers: HeaderMap,
    jar: CookieJar,
) -> ApiResult<impl IntoResponse> {
    if crate::auth::bearer(&headers).is_some() {
        s.core.logout_token(&p).await?;
        return Ok((jar, StatusCode::NO_CONTENT));
    }
    if let Some(c) = jar.get(SESSION_COOKIE) {
        s.core.logout_session(&p, c.value()).await?;
    }
    Ok((jar.remove(removal_cookie()), StatusCode::NO_CONTENT))
}

/// The caller with effective permissions.
#[utoipa::path(get, path = "/me", tag = "auth", responses((status = 200, body = Me), (status = 401, body = Problem)))]
pub async fn me(State(s): State<RestState>, ApiPrincipal(p): ApiPrincipal) -> Json<Me> {
    Json(s.core.me(&p))
}

#[utoipa::path(post, path = "/me/password", tag = "auth", request_body = ChangePassword,
    responses((status = 204), (status = 422, body = Problem)))]
pub async fn change_password(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Body(req): Body<ChangePassword>,
) -> ApiResult<StatusCode> {
    s.core.change_own_password(&p, req).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/me/tokens", tag = "auth", responses((status = 200, body = Vec<ApiToken>)))]
pub async fn list_tokens(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
) -> ApiResult<Json<Vec<ApiToken>>> {
    Ok(Json(s.core.list_tokens(&p).await?))
}

#[utoipa::path(post, path = "/me/tokens", tag = "auth", request_body = NewToken,
    responses((status = 201, body = CreatedToken), (status = 403, body = Problem)))]
pub async fn create_token(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Body(req): Body<NewToken>,
) -> ApiResult<(StatusCode, Json<CreatedToken>)> {
    Ok((
        StatusCode::CREATED,
        Json(s.core.create_token(&p, req).await?),
    ))
}

#[utoipa::path(delete, path = "/me/tokens/{id}", tag = "auth", responses((status = 204), (status = 404, body = Problem)))]
pub async fn revoke_token(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    s.core.revoke_token(&p, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- notes

#[utoipa::path(get, path = "/notes", tag = "notes", params(NoteQuery),
    responses((status = 200, body = NotePage), (status = 403, body = Problem)))]
pub async fn list_notes(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Query(q): Query<NoteQuery>,
) -> ApiResult<Json<NotePage>> {
    Ok(Json(s.core.list_notes(&p, q).await?))
}

#[utoipa::path(post, path = "/notes", tag = "notes", request_body = NewNote,
    responses((status = 201, body = Note), (status = 403, body = Problem), (status = 422, body = Problem)))]
pub async fn create_note(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Body(req): Body<NewNote>,
) -> ApiResult<(StatusCode, Json<Note>)> {
    Ok((
        StatusCode::CREATED,
        Json(s.core.create_note(&p, req).await?),
    ))
}

#[utoipa::path(get, path = "/notes/{id}", tag = "notes",
    responses((status = 200, body = Note), (status = 404, body = Problem)))]
pub async fn get_note(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Note>> {
    Ok(Json(s.core.get_note(&p, id).await?))
}

#[utoipa::path(patch, path = "/notes/{id}", tag = "notes", request_body = NoteUpdate,
    responses((status = 200, body = Note), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn update_note(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
    Body(req): Body<NoteUpdate>,
) -> ApiResult<Json<Note>> {
    Ok(Json(s.core.update_note(&p, id, req).await?))
}

#[utoipa::path(delete, path = "/notes/{id}", tag = "notes",
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_note(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    s.core.delete_note(&p, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- users

#[utoipa::path(get, path = "/users", tag = "users", responses((status = 200, body = Vec<User>), (status = 403, body = Problem)))]
pub async fn list_users(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
) -> ApiResult<Json<Vec<User>>> {
    Ok(Json(s.core.list_users(&p).await?))
}

#[utoipa::path(post, path = "/users", tag = "users", request_body = NewUser,
    responses((status = 201, body = User), (status = 403, body = Problem), (status = 409, body = Problem)))]
pub async fn create_user(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Body(req): Body<NewUser>,
) -> ApiResult<(StatusCode, Json<User>)> {
    Ok((
        StatusCode::CREATED,
        Json(s.core.create_user(&p, req).await?),
    ))
}

#[utoipa::path(get, path = "/users/{id}", tag = "users", responses((status = 200, body = User), (status = 404, body = Problem)))]
pub async fn get_user(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<User>> {
    Ok(Json(s.core.get_user(&p, id).await?))
}

#[utoipa::path(patch, path = "/users/{id}", tag = "users", request_body = UserUpdate,
    responses((status = 200, body = User), (status = 403, body = Problem), (status = 409, body = Problem)))]
pub async fn update_user(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
    Body(req): Body<UserUpdate>,
) -> ApiResult<Json<User>> {
    Ok(Json(s.core.update_user(&p, id, req).await?))
}

#[utoipa::path(put, path = "/users/{id}/roles", tag = "users", request_body = SetRoles,
    responses((status = 200, body = User), (status = 403, body = Problem), (status = 409, body = Problem)))]
pub async fn set_user_roles(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
    Body(req): Body<SetRoles>,
) -> ApiResult<Json<User>> {
    Ok(Json(s.core.set_user_roles(&p, id, req.roles).await?))
}

// ---------------------------------------------------------------- roles

#[utoipa::path(get, path = "/roles", tag = "roles", responses((status = 200, body = Vec<Role>)))]
pub async fn list_roles(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
) -> ApiResult<Json<Vec<Role>>> {
    Ok(Json(s.core.list_roles(&p).await?))
}

#[utoipa::path(post, path = "/roles", tag = "roles", request_body = NewRole,
    responses((status = 201, body = Role), (status = 403, body = Problem), (status = 409, body = Problem)))]
pub async fn create_role(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Body(req): Body<NewRole>,
) -> ApiResult<(StatusCode, Json<Role>)> {
    Ok((
        StatusCode::CREATED,
        Json(s.core.create_role(&p, req).await?),
    ))
}

#[utoipa::path(get, path = "/roles/{id}", tag = "roles", responses((status = 200, body = Role), (status = 404, body = Problem)))]
pub async fn get_role(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Role>> {
    Ok(Json(s.core.get_role(&p, id).await?))
}

#[utoipa::path(patch, path = "/roles/{id}", tag = "roles", request_body = RoleUpdate,
    responses((status = 200, body = Role), (status = 403, body = Problem), (status = 409, body = Problem)))]
pub async fn update_role(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
    Body(req): Body<RoleUpdate>,
) -> ApiResult<Json<Role>> {
    Ok(Json(s.core.update_role(&p, id, req).await?))
}

#[utoipa::path(delete, path = "/roles/{id}", tag = "roles",
    responses((status = 204), (status = 403, body = Problem), (status = 409, body = Problem)))]
pub async fn delete_role(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    s.core.delete_role(&p, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/permissions", tag = "roles", responses((status = 200, body = Vec<PermissionInfo>)))]
pub async fn list_permissions(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
) -> ApiResult<Json<Vec<PermissionInfo>>> {
    Ok(Json(s.core.list_permissions(&p).await?))
}

// ---------------------------------------------------------------- audit

#[utoipa::path(get, path = "/audit", tag = "audit", params(AuditQuery),
    responses((status = 200, body = AuditPage), (status = 403, body = Problem)))]
pub async fn list_audit(
    State(s): State<RestState>,
    ApiPrincipal(p): ApiPrincipal,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<AuditPage>> {
    Ok(Json(s.core.list_audit(&p, q).await?))
}
