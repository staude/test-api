use serde_json::json;
use uuid::Uuid;

use crate::auth::check_password_policy;
use crate::dto::{NewUser, User, UserUpdate};
use crate::error::validation;
use crate::{Core, Error, Permission, Principal, Result, crypto};

fn check_email(email: &str) -> Result<()> {
    let ok = email.len() <= 254
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'));
    if ok {
        Ok(())
    } else {
        Err(validation("Ungültige E-Mail-Adresse"))
    }
}

fn check_display_name(name: &str) -> Result<()> {
    let len = name.trim().chars().count();
    if (1..=100).contains(&len) {
        Ok(())
    } else {
        Err(validation("Der Name muss 1 bis 100 Zeichen lang sein"))
    }
}

impl Core {
    pub async fn list_users(&self, p: &Principal) -> Result<Vec<User>> {
        p.require(Permission::UsersRead)?;
        let users = test_db::users::list(&self.pool).await?;
        self.audit_read(p, "user.list", None, json!({ "count": users.len() }))
            .await;
        Ok(users)
    }

    pub async fn get_user(&self, p: &Principal, id: Uuid) -> Result<User> {
        if id != p.user_id {
            p.require(Permission::UsersRead)?;
        }
        let user = test_db::users::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)?;
        self.audit_read(
            p,
            "user.read",
            Some(("user", id.to_string())),
            json!({ "email": user.email }),
        )
        .await;
        Ok(user)
    }

    pub async fn find_user_id_by_email(&self, p: &Principal, email: &str) -> Result<Uuid> {
        p.require(Permission::UsersRead)?;
        test_db::users::find_id_by_email(&self.pool, email.trim())
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn create_user(&self, p: &Principal, req: NewUser) -> Result<User> {
        p.require(Permission::UsersManage)?;
        let email = req.email.trim();
        check_email(email)?;
        check_display_name(&req.display_name)?;
        check_password_policy(&req.password)?;
        self.ensure_roles_assignable(p, &req.roles).await?;

        let hash = crypto::hash_password(req.password).await?;
        let mut tx = self.pool.begin().await?;
        let id = test_db::users::insert(&mut *tx, email, req.display_name.trim(), &hash)
            .await
            .map_err(|e| {
                if test_db::is_unique_violation(&e) {
                    Error::Conflict("Diese E-Mail-Adresse ist bereits vergeben".into())
                } else {
                    e.into()
                }
            })?;
        test_db::users::set_roles(&mut tx, id, &req.roles).await?;
        tx.commit().await?;

        self.audit(
            p,
            "user.create",
            Some(("user", id.to_string())),
            json!({ "email": email, "roles": req.roles }),
        )
        .await;
        test_db::users::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn update_user(&self, p: &Principal, id: Uuid, req: UserUpdate) -> Result<User> {
        p.require(Permission::UsersManage)?;
        if let Some(name) = &req.display_name {
            check_display_name(name)?;
        }
        if req.is_active == Some(false) && id == p.user_id {
            return Err(Error::Conflict(
                "Du kannst dich nicht selbst sperren".into(),
            ));
        }
        let hash = match req.password {
            Some(pw) => {
                check_password_policy(&pw)?;
                Some(crypto::hash_password(pw).await?)
            }
            None => None,
        };

        let mut tx = self.pool.begin().await?;
        let found = test_db::users::update_profile(
            &mut *tx,
            id,
            req.display_name.as_deref().map(str::trim),
            req.is_active,
        )
        .await?;
        if !found {
            return Err(Error::NotFound);
        }
        if let Some(hash) = &hash {
            test_db::users::set_password(&mut *tx, id, hash).await?;
            test_db::sessions::delete_for_user(&mut *tx, id).await?;
        }
        if req.is_active == Some(false) {
            test_db::sessions::delete_for_user(&mut *tx, id).await?;
            test_db::tokens::revoke_all_for_user(&mut *tx, id).await?;
        }
        ensure_admin_remains(&mut tx).await?;
        tx.commit().await?;

        self.audit(
            p,
            "user.update",
            Some(("user", id.to_string())),
            json!({
                "display_name": req.display_name,
                "is_active": req.is_active,
                "password_changed": hash.is_some(),
            }),
        )
        .await;
        test_db::users::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn set_user_roles(
        &self,
        p: &Principal,
        id: Uuid,
        roles: Vec<String>,
    ) -> Result<User> {
        p.require(Permission::UsersManage)?;
        let current = test_db::users::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)?;
        // Taking a role away requires the same rights as granting it.
        let mut touched = roles.clone();
        touched.extend(current.roles.iter().filter(|r| !roles.contains(r)).cloned());
        self.ensure_roles_assignable(p, &touched).await?;

        let mut tx = self.pool.begin().await?;
        test_db::users::set_roles(&mut tx, id, &roles).await?;
        ensure_admin_remains(&mut tx).await?;
        tx.commit().await?;

        self.audit(
            p,
            "user.set_roles",
            Some(("user", id.to_string())),
            json!({ "before": current.roles, "after": roles }),
        )
        .await;
        test_db::users::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)
    }

    /// Admin CLI: set a password without knowing the old one.
    pub async fn admin_reset_password(&self, email: &str, password: String) -> Result<()> {
        let p = Principal::system();
        let id = self.find_user_id_by_email(&p, email).await?;
        self.update_user(
            &p,
            id,
            UserUpdate {
                password: Some(password),
                is_active: Some(true),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
    }

    /// Roles must exist, and the actor may only hand out permissions they
    /// hold themselves. This prevents privilege escalation via role
    /// assignment.
    async fn ensure_roles_assignable(&self, p: &Principal, names: &[String]) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let existing = test_db::roles::existing_names(&self.pool, names).await?;
        if let Some(missing) = names.iter().find(|n| !existing.contains(n)) {
            return Err(validation(format!("Unbekannte Rolle: {missing}")));
        }
        for name in names {
            let Some(role) = test_db::roles::get_by_name(&self.pool, name).await? else {
                continue;
            };
            if let Some(missing) = role
                .permissions
                .iter()
                .find(|k| !Permission::from_key(k).is_some_and(|perm| p.has(perm)))
            {
                return Err(Error::Forbidden(format!(
                    "{missing} (benötigt, um Rolle {name} zu vergeben)"
                )));
            }
        }
        Ok(())
    }
}

/// At least one active user must keep the right to manage users.
async fn ensure_admin_remains(tx: &mut sqlx::PgConnection) -> Result<()> {
    let n = test_db::users::count_active_with_permission(&mut *tx, Permission::UsersManage.key())
        .await?;
    if n == 0 {
        return Err(Error::Conflict(
            "Mindestens ein aktiver Benutzer muss Benutzer verwalten dürfen".into(),
        ));
    }
    Ok(())
}
