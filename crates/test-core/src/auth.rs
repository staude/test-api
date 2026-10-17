use std::collections::BTreeSet;

use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::dto::{ChangePassword, LoginMode, LoginRequest, Me};
use crate::error::validation;
use crate::{Channel, Core, Error, Permission, Principal, Result, crypto};

const SESSION_TTL: Duration = Duration::days(7);
/// Sessions are extended at most once per day to avoid a write per request.
const SESSION_REFRESH: Duration = Duration::days(6);
const LOGIN_TOKEN_TTL: Duration = Duration::days(90);
pub(crate) const MIN_PASSWORD_LEN: usize = 12;

pub struct SessionInfo {
    /// Plain cookie value; only its hash is stored.
    pub cookie_value: String,
    pub csrf_token: String,
    pub expires_at: OffsetDateTime,
}

pub struct LoginOutcome {
    pub principal: Principal,
    pub token: Option<String>,
    pub session: Option<SessionInfo>,
}

pub(crate) fn check_password_policy(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(validation(format!(
            "Das Passwort muss mindestens {MIN_PASSWORD_LEN} Zeichen lang sein"
        )));
    }
    Ok(())
}

impl Core {
    /// Verifies e-mail and password, with rate limiting and audit.
    pub async fn authenticate_password(
        &self,
        email: &str,
        password: &str,
        channel: Channel,
        ip: Option<&str>,
    ) -> Result<Principal> {
        let email = email.trim();
        if self.inner.limiter.is_blocked(email, ip) {
            return Err(Error::RateLimited);
        }
        let creds = test_db::users::credentials_by_email(&self.pool, email).await?;
        // Always run argon2, so unknown e-mails take as long as wrong passwords.
        let hash = creds
            .as_ref()
            .map(|c| c.password_hash.clone())
            .unwrap_or_else(|| self.inner.dummy_hash.clone());
        let ok = crypto::verify_password(password.to_owned(), hash).await;

        let principal = match creds {
            Some(c) if ok && c.is_active => self.principal_for_user(c.id, channel, None).await?,
            _ => None,
        };
        let Some(principal) = principal else {
            self.inner.limiter.record_failure(email, ip);
            self.audit_raw(
                None,
                channel.as_str(),
                "auth.login_failed",
                None,
                json!({ "email": email, "ip": ip }),
            )
            .await;
            return Err(Error::InvalidCredentials);
        };
        self.inner.limiter.record_success(email);
        test_db::users::touch_login(&self.pool, principal.user_id).await?;
        self.audit(&principal, "auth.login", None, json!({ "ip": ip }))
            .await;
        Ok(principal)
    }

    /// Login endpoint logic for REST: returns a token or a session.
    pub async fn login(
        &self,
        req: LoginRequest,
        channel: Channel,
        ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<LoginOutcome> {
        let principal = self
            .authenticate_password(&req.email, &req.password, channel, ip)
            .await?;
        match req.mode {
            LoginMode::Token => {
                let name = req
                    .token_name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| "Login".into());
                let token = self.issue_login_token(&principal, &name).await?;
                Ok(LoginOutcome {
                    principal,
                    token: Some(token),
                    session: None,
                })
            }
            LoginMode::Session => {
                let session = self
                    .create_session(principal.user_id, user_agent, ip)
                    .await?;
                Ok(LoginOutcome {
                    principal,
                    token: None,
                    session: Some(session),
                })
            }
        }
    }

    async fn issue_login_token(&self, p: &Principal, name: &str) -> Result<String> {
        let secret = crypto::new_api_token();
        let prefix: String = secret.chars().take(12).collect();
        let expires = OffsetDateTime::now_utc() + LOGIN_TOKEN_TTL;
        let id = test_db::tokens::insert(
            &self.pool,
            p.user_id,
            name,
            &prefix,
            &crypto::sha256(&secret),
            None,
            Some(expires),
        )
        .await?;
        self.audit(
            p,
            "token.create",
            Some(("token", id.to_string())),
            json!({ "name": name }),
        )
        .await;
        Ok(secret)
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        user_agent: Option<&str>,
        ip: Option<&str>,
    ) -> Result<SessionInfo> {
        let cookie_value = crypto::random_secret();
        let csrf_token = crypto::random_secret();
        let expires_at = OffsetDateTime::now_utc() + SESSION_TTL;
        test_db::sessions::insert(
            &self.pool,
            &crypto::sha256(&cookie_value),
            user_id,
            &csrf_token,
            expires_at,
            user_agent,
            ip,
        )
        .await?;
        Ok(SessionInfo {
            cookie_value,
            csrf_token,
            expires_at,
        })
    }

    /// Resolves a session cookie into a principal and its CSRF token.
    pub async fn resolve_session(
        &self,
        cookie_value: &str,
        channel: Channel,
    ) -> Result<Option<(Principal, String)>> {
        let hash = crypto::sha256(cookie_value);
        let Some(session) = test_db::sessions::find_valid(&self.pool, &hash).await? else {
            return Ok(None);
        };
        if session.expires_at - OffsetDateTime::now_utc() < SESSION_REFRESH {
            test_db::sessions::extend(&self.pool, &hash, OffsetDateTime::now_utc() + SESSION_TTL)
                .await?;
        }
        Ok(self
            .principal_for_user(session.user_id, channel, None)
            .await?
            .map(|p| (p, session.csrf_token)))
    }

    pub async fn logout_session(&self, p: &Principal, cookie_value: &str) -> Result<()> {
        test_db::sessions::delete(&self.pool, &crypto::sha256(cookie_value)).await?;
        self.audit(p, "auth.logout", None, json!({})).await;
        Ok(())
    }

    /// Resolves a bearer token into a principal.
    pub async fn resolve_token(&self, token: &str, channel: Channel) -> Result<Option<Principal>> {
        if !token.starts_with(crypto::TOKEN_PREFIX) {
            return Ok(None);
        }
        let Some(found) = test_db::tokens::use_token(&self.pool, &crypto::sha256(token)).await?
        else {
            return Ok(None);
        };
        let principal = self
            .principal_for_user(found.user_id, channel, found.scopes.as_deref())
            .await?;
        Ok(principal.map(|p| Principal {
            token_id: Some(found.id),
            ..p
        }))
    }

    /// Revokes the token the principal is authenticated with.
    pub async fn logout_token(&self, p: &Principal) -> Result<()> {
        if let Some(id) = p.token_id {
            test_db::tokens::revoke(&self.pool, p.user_id, id).await?;
            self.audit(p, "auth.logout", Some(("token", id.to_string())), json!({}))
                .await;
        }
        Ok(())
    }

    /// Loads the current permissions of a user. `None` if the user does not
    /// exist or is deactivated.
    pub async fn principal_for_user(
        &self,
        user_id: Uuid,
        channel: Channel,
        scopes: Option<&[String]>,
    ) -> Result<Option<Principal>> {
        let Some(identity) = test_db::users::identity(&self.pool, user_id).await? else {
            return Ok(None);
        };
        if !identity.is_active {
            return Ok(None);
        }
        let mut permissions: BTreeSet<Permission> = identity
            .permissions
            .iter()
            .filter_map(|k| Permission::from_key(k))
            .collect();
        if let Some(scopes) = scopes {
            permissions.retain(|p| scopes.iter().any(|s| s == p.key()));
        }
        Ok(Some(Principal {
            user_id: identity.id,
            email: identity.email,
            display_name: identity.display_name,
            roles: identity.roles,
            permissions,
            channel,
            token_id: None,
            token_scoped: scopes.is_some(),
            system: false,
        }))
    }

    pub fn me(&self, p: &Principal) -> Me {
        Me {
            id: p.user_id,
            email: p.email.clone(),
            display_name: p.display_name.clone(),
            roles: p.roles.clone(),
            permissions: p.permissions.iter().map(|p| p.key().to_owned()).collect(),
        }
    }

    pub async fn change_own_password(&self, p: &Principal, req: ChangePassword) -> Result<()> {
        if p.token_scoped {
            return Err(Error::Forbidden(
                "Passwortänderung mit eingeschränktem Token".into(),
            ));
        }
        let creds = test_db::users::credentials_by_id(&self.pool, p.user_id)
            .await?
            .ok_or(Error::NotFound)?;
        if !crypto::verify_password(req.current_password, creds.password_hash).await {
            return Err(validation("Das aktuelle Passwort ist falsch"));
        }
        check_password_policy(&req.new_password)?;
        let hash = crypto::hash_password(req.new_password).await?;
        test_db::users::set_password(&self.pool, p.user_id, &hash).await?;
        self.audit(
            p,
            "user.password_change",
            Some(("user", p.user_id.to_string())),
            json!({}),
        )
        .await;
        Ok(())
    }
}
