-- Substring search (ILIKE '%term%') on title and body via trigram indexes.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX notes_title_trgm_idx ON notes USING gin (title gin_trgm_ops);
CREATE INDEX notes_body_trgm_idx ON notes USING gin (body gin_trgm_ops);

-- Retention cleanup deletes by age.
CREATE INDEX audit_log_at_idx ON audit_log (at);
