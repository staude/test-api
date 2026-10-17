use serde_json::json;
use uuid::Uuid;

use crate::dto::{NewNote, Note, NotePage, NoteQuery, NoteUpdate};
use crate::error::validation;
use crate::{Core, Error, Permission, Principal, Result};

const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 100_000;

fn check_title(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        return Err(validation("Der Titel darf nicht leer sein"));
    }
    if title.chars().count() > MAX_TITLE {
        return Err(validation(format!(
            "Der Titel darf höchstens {MAX_TITLE} Zeichen haben"
        )));
    }
    Ok(())
}

fn check_body(body: &str) -> Result<()> {
    if body.len() > MAX_BODY {
        return Err(validation("Der Text ist zu lang"));
    }
    Ok(())
}

impl Core {
    pub async fn list_notes(&self, p: &Principal, q: NoteQuery) -> Result<NotePage> {
        let owner = if p.has(Permission::NotesRead) {
            None
        } else if p.has(Permission::NotesReadOwn) {
            Some(p.user_id)
        } else {
            return Err(Error::Forbidden(Permission::NotesRead.key().into()));
        };
        let limit = q.limit.unwrap_or(20).clamp(1, 100);
        let search = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let items = test_db::notes::list(
            &self.pool,
            test_db::notes::ListFilter {
                owner,
                search,
                after: q.cursor,
                limit,
            },
        )
        .await?;
        let next_cursor = (items.len() as i64 == limit)
            .then(|| items.last().map(|n| n.id))
            .flatten();
        self.audit_read(
            p,
            "note.list",
            None,
            json!({
                "query": search,
                "scope": if owner.is_some() { "own" } else { "all" },
                "count": items.len(),
                "cursor": q.cursor,
            }),
        )
        .await;
        Ok(NotePage { items, next_cursor })
    }

    /// Notes the principal may not read are reported as not found, so their
    /// existence is not leaked.
    pub async fn get_note(&self, p: &Principal, id: Uuid) -> Result<Note> {
        if !p.has_any_scope(Permission::NotesRead) {
            return Err(Error::Forbidden(Permission::NotesRead.key().into()));
        }
        let note = test_db::notes::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)?;
        if !p.can_on(Permission::NotesRead, note.owner_id) {
            return Err(Error::NotFound);
        }
        self.audit_read(
            p,
            "note.read",
            Some(("note", id.to_string())),
            json!({ "title": note.title }),
        )
        .await;
        Ok(note)
    }

    pub async fn create_note(&self, p: &Principal, req: NewNote) -> Result<Note> {
        if !p.has_any_scope(Permission::NotesWrite) {
            return Err(Error::Forbidden(Permission::NotesWrite.key().into()));
        }
        check_title(&req.title)?;
        check_body(&req.body)?;
        let id = test_db::notes::insert(&self.pool, p.user_id, req.title.trim(), &req.body).await?;
        self.audit(
            p,
            "note.create",
            Some(("note", id.to_string())),
            json!({ "title": req.title.trim(), "body_length": req.body.chars().count() }),
        )
        .await;
        test_db::notes::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn update_note(&self, p: &Principal, id: Uuid, req: NoteUpdate) -> Result<Note> {
        let note = self.writable_note(p, id).await?;
        if let Some(t) = &req.title {
            check_title(t)?;
        }
        if let Some(b) = &req.body {
            check_body(b)?;
        }
        test_db::notes::update(
            &self.pool,
            note.id,
            req.title.as_deref().map(str::trim),
            req.body.as_deref(),
        )
        .await?;
        let mut changed = Vec::new();
        if req.title.as_deref().is_some_and(|t| t.trim() != note.title) {
            changed.push("title");
        }
        if req.body.as_deref().is_some_and(|b| b != note.body) {
            changed.push("body");
        }
        self.audit(
            p,
            "note.update",
            Some(("note", id.to_string())),
            json!({
                "title_before": note.title,
                "title_after": req.title.as_deref().map(str::trim).unwrap_or(&note.title),
                "changed": changed,
            }),
        )
        .await;
        test_db::notes::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn delete_note(&self, p: &Principal, id: Uuid) -> Result<()> {
        let note = self.writable_note(p, id).await?;
        test_db::notes::delete(&self.pool, note.id).await?;
        self.audit(
            p,
            "note.delete",
            Some(("note", id.to_string())),
            json!({ "title": note.title }),
        )
        .await;
        Ok(())
    }

    async fn writable_note(&self, p: &Principal, id: Uuid) -> Result<Note> {
        if !p.has_any_scope(Permission::NotesWrite) {
            return Err(Error::Forbidden(Permission::NotesWrite.key().into()));
        }
        let note = test_db::notes::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound)?;
        if p.can_on(Permission::NotesWrite, note.owner_id) {
            Ok(note)
        } else if p.can_on(Permission::NotesRead, note.owner_id) {
            Err(Error::Forbidden(Permission::NotesWrite.key().into()))
        } else {
            Err(Error::NotFound)
        }
    }
}
