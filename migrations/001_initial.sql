-- Migration 001: Initial schema
-- Creates the foundational tables for the knowledge base.

CREATE TABLE schema_meta (
    version    INTEGER NOT NULL,
    updated_at TEXT    NOT NULL
);

INSERT INTO schema_meta (version, updated_at) VALUES (1, datetime('now'));

CREATE TABLE spaces (
    id          TEXT PRIMARY KEY,
    slug        TEXT UNIQUE NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
);

CREATE TABLE pages (
    id               TEXT    PRIMARY KEY,
    space_id         TEXT    NOT NULL REFERENCES spaces(id),
    parent_id        TEXT    REFERENCES pages(id),
    title            TEXT    NOT NULL,
    page_type        TEXT    NOT NULL CHECK(page_type IN (
        'decision', 'architecture', 'session-log',
        'reference', 'troubleshooting', 'runbook'
    )),
    content          TEXT    NOT NULL DEFAULT '',
    created_by_user  TEXT    NOT NULL,
    created_by_agent TEXT    NOT NULL,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    version          INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE labels (
    page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    label   TEXT NOT NULL,
    PRIMARY KEY (page_id, label)
);

CREATE TABLE links (
    source_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    target_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    relation  TEXT NOT NULL DEFAULT 'relates-to' CHECK(relation IN (
        'relates-to', 'supersedes', 'depends-on', 'elaborates'
    )),
    PRIMARY KEY (source_id, target_id)
);

-- Full-text search index on page titles and content.
CREATE VIRTUAL TABLE pages_fts USING fts5(
    title,
    content,
    content='pages',
    content_rowid='rowid'
);

-- Triggers keep the FTS index in sync with the pages table.
-- On INSERT: add the new row to the FTS index.
CREATE TRIGGER pages_fts_insert AFTER INSERT ON pages BEGIN
    INSERT INTO pages_fts(rowid, title, content)
    VALUES (new.rowid, new.title, new.content);
END;

-- On DELETE: remove the row from the FTS index.
CREATE TRIGGER pages_fts_delete AFTER DELETE ON pages BEGIN
    INSERT INTO pages_fts(pages_fts, rowid, title, content)
    VALUES ('delete', old.rowid, old.title, old.content);
END;

-- On UPDATE: remove the old entry and insert the new one.
CREATE TRIGGER pages_fts_update AFTER UPDATE ON pages BEGIN
    INSERT INTO pages_fts(pages_fts, rowid, title, content)
    VALUES ('delete', old.rowid, old.title, old.content);
    INSERT INTO pages_fts(rowid, title, content)
    VALUES (new.rowid, new.title, new.content);
END;

-- Indexes for common query patterns.
CREATE INDEX idx_pages_space    ON pages(space_id);
CREATE INDEX idx_pages_type     ON pages(page_type);
CREATE INDEX idx_pages_parent   ON pages(parent_id);
CREATE INDEX idx_labels_label   ON labels(label);
