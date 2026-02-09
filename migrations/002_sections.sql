-- Migration 002: Add structured sections to pages
ALTER TABLE pages ADD COLUMN sections TEXT DEFAULT NULL;
UPDATE schema_meta SET version = 2, updated_at = datetime('now');
