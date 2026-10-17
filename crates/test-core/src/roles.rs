use serde_json::json;
use uuid::Uuid;

use crate::dto::{NewRole, PermissionInfo, Role, RoleUpdate};
use crate::error::validation;
use crate::{Core, Error, Permission, Principal, Result};

fn check_role_name(name: &str) -> Result<()> {
    let ok = (2..=40).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(validation(
            "Rollennamen: 2–40 Zeichen, nur a–z, 0–9, - und _",
        ))
    }
}

/// Parses permission keys and checks that the actor holds each of them.
fn grantable(p: &Principal, keys: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        let perm = Permission::from_key(key)
            .ok_or_else(|| validation(format!("Unbekanntes Recht: {key}")))?;
        p.require(perm)?;
        if !out.contains(key) {
            out.push(key.clone());
        }
    }
    Ok(out)
}

impl Core {
    /// Every signed-in user may see roles, so the UI can explain rights.
    pub async fn list_roles(&self, _p: &Principal) -> Result<Vec<Role>> {
        Ok(test_db::roles::list(&self.pool).await?)
    }

    pub async fn list_permissions(&self, _p: &Principal) -> Result<Vec<PermissionInfo>> {
        Ok(test_db::roles::list_permissions(&self.pool).await?)
    }

    pub async fn get_role(&self, _p: &Principal, id: Uuid) -> Result<Role> {
        test_db::roles::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn create_role(&self, p: &Principal, req: NewRole) -> Result<Role> {
        p.require(Permission::RolesManage)?;
        check_role_name(&req.name)?;
        let perms = grantable(p, &req.permissions)?;
        let mut tx = self.pool.begin().await?;
        let id = test_db::roles::insert(&mut *tx, &req.name, req.description.trim(), false)
            .await
            .map_err(|e| {
                if test_db::is_unique_violation(&e) {
                    Error::Conflict("Eine Rolle mit diesem Namen existiert bereits".into())
                } else {
                    e.into()
                }
            })?;
        test_db::roles::set_permissions(&mut tx, id, &perms).await?;
        tx.commit().await?;
        self.audit(
            p,
            "role.create",
            Some(("role", id.to_string())),
            json!({ "name": req.name, "permissions": perms }),
        )
        .await;
        self.get_role(p, id).await
    }

    pub async fn update_role(&self, p: &Principal, id: Uuid, req: RoleUpdate) -> Result<Role> {
        p.require(Permission::RolesManage)?;
        let role = self.get_role(p, id).await?;
        if role.is_system {
            return Err(Error::Conflict(
                "Systemrollen können nicht geändert werden".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        if let Some(desc) = &req.description {
            test_db::roles::update_description(&mut *tx, id, desc.trim()).await?;
        }
        if let Some(keys) = &req.permissions {
            // Removing a permission also requires holding it.
            let mut touched = keys.clone();
            touched.extend(
                role.permissions
                    .iter()
                    .filter(|k| !keys.contains(k))
                    .cloned(),
            );
            grantable(p, &touched)?;
            let perms = grantable(p, keys)?;
            test_db::roles::set_permissions(&mut tx, id, &perms).await?;
        }
        tx.commit().await?;
        self.audit(
            p,
            "role.update",
            Some(("role", id.to_string())),
            json!({ "before": role.permissions, "after": req.permissions }),
        )
        .await;
        self.get_role(p, id).await
    }

    pub async fn delete_role(&self, p: &Principal, id: Uuid) -> Result<()> {
        p.require(Permission::RolesManage)?;
        let role = self.get_role(p, id).await?;
        if role.is_system {
            return Err(Error::Conflict(
                "Systemrollen können nicht gelöscht werden".into(),
            ));
        }
        grantable(p, &role.permissions)?;
        test_db::roles::delete(&self.pool, id).await?;
        self.audit(
            p,
            "role.delete",
            Some(("role", id.to_string())),
            json!({ "name": role.name }),
        )
        .await;
        Ok(())
    }

    pub async fn role_id_by_name(&self, p: &Principal, name: &str) -> Result<Uuid> {
        let _ = p;
        test_db::roles::get_by_name(&self.pool, name)
            .await?
            .map(|r| r.id)
            .ok_or(Error::NotFound)
    }
}
