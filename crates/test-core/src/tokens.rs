use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::dto::{ApiToken, CreatedToken, NewToken};
use crate::error::validation;
use crate::{Core, Error, Permission, Principal, Result, crypto};

impl Core {
    pub async fn list_tokens(&self, p: &Principal) -> Result<Vec<ApiToken>> {
        p.require(Permission::TokensManageOwn)?;
        Ok(test_db::tokens::list_for_user(&self.pool, p.user_id).await?)
    }

    pub async fn create_token(&self, p: &Principal, req: NewToken) -> Result<CreatedToken> {
        p.require(Permission::TokensManageOwn)?;
        let name = req.name.trim();
        if name.is_empty() || name.chars().count() > 100 {
            return Err(validation(
                "Der Token-Name muss 1 bis 100 Zeichen lang sein",
            ));
        }
        // A token can never carry more than the principal creating it. A
        // scoped token therefore can only create equally or less scoped ones.
        let scopes = match req.scopes {
            Some(keys) => {
                let mut out = Vec::new();
                for key in keys {
                    let perm = Permission::from_key(&key)
                        .ok_or_else(|| validation(format!("Unbekanntes Recht: {key}")))?;
                    p.require(perm)?;
                    if !out.contains(&key) {
                        out.push(key);
                    }
                }
                Some(out)
            }
            None if p.token_scoped => Some(
                p.permissions
                    .iter()
                    .map(|perm| perm.key().to_owned())
                    .collect(),
            ),
            None => None,
        };
        let expires_at = match req.expires_in_days {
            Some(0) => return Err(validation("Die Laufzeit muss mindestens 1 Tag betragen")),
            Some(days) => {
                Some(OffsetDateTime::now_utc() + Duration::days(i64::from(days.min(3650))))
            }
            None => None,
        };

        let secret = crypto::new_api_token();
        let prefix: String = secret.chars().take(12).collect();
        let id = test_db::tokens::insert(
            &self.pool,
            p.user_id,
            name,
            &prefix,
            &crypto::sha256(&secret),
            scopes.as_deref(),
            expires_at,
        )
        .await?;
        self.audit(
            p,
            "token.create",
            Some(("token", id.to_string())),
            json!({ "name": name, "scopes": scopes }),
        )
        .await;
        let token = test_db::tokens::get_for_user(&self.pool, p.user_id, id)
            .await?
            .ok_or(Error::NotFound)?;
        Ok(CreatedToken { token, secret })
    }

    pub async fn revoke_token(&self, p: &Principal, id: Uuid) -> Result<()> {
        p.require(Permission::TokensManageOwn)?;
        if !test_db::tokens::revoke(&self.pool, p.user_id, id).await? {
            return Err(Error::NotFound);
        }
        self.audit(
            p,
            "token.revoke",
            Some(("token", id.to_string())),
            json!({}),
        )
        .await;
        Ok(())
    }
}
