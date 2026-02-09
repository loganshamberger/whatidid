-- Migration 003: Add timestamps to spaces and links

-- Add updated_at to spaces, default to created_at for existing rows
ALTER TABLE spaces ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE spaces SET updated_at = created_at WHERE updated_at = '';

-- Add timestamps to links
ALTER TABLE links ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE links ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
UPDATE links SET created_at = datetime('now'), updated_at = datetime('now') WHERE created_at = '';

UPDATE schema_meta SET version = 3, updated_at = datetime('now');
