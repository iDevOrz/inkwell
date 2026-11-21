CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    email         TEXT,
    role          TEXT NOT NULL DEFAULT 'admin',
    created_at    INTEGER NOT NULL
);

CREATE TABLE contents (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    kind          TEXT NOT NULL DEFAULT 'post',
    title         TEXT NOT NULL,
    slug          TEXT NOT NULL UNIQUE,
    body          TEXT NOT NULL DEFAULT '',
    excerpt       TEXT,
    status        TEXT NOT NULL DEFAULT 'draft',
    visibility    TEXT NOT NULL DEFAULT 'public',
    author_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    published_at  INTEGER,
    review_status TEXT NOT NULL DEFAULT 'none',
    review_notes  TEXT,
    allow_comments INTEGER NOT NULL DEFAULT 1,
    source_note_id INTEGER REFERENCES contents(id) ON DELETE SET NULL,
    promoted_to_id INTEGER REFERENCES contents(id) ON DELETE SET NULL,
    metadata      TEXT NOT NULL DEFAULT '{}',
    deleted_at    INTEGER
);

CREATE INDEX contents_kind_status ON contents(kind, status);
CREATE INDEX contents_published_at ON contents(published_at);
CREATE INDEX contents_updated_at ON contents(updated_at);
CREATE UNIQUE INDEX contents_source_note_unique
    ON contents(source_note_id) WHERE source_note_id IS NOT NULL;

CREATE TABLE terms (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    parent_id   INTEGER REFERENCES terms(id) ON DELETE SET NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE(kind, slug)
);

CREATE INDEX terms_kind ON terms(kind);

CREATE TABLE content_terms (
    content_id INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
    term_id    INTEGER NOT NULL REFERENCES terms(id) ON DELETE CASCADE,
    PRIMARY KEY(content_id, term_id)
);

CREATE INDEX content_terms_term ON content_terms(term_id);

CREATE TABLE comments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
    parent_id  INTEGER NOT NULL DEFAULT 0,
    author     TEXT NOT NULL,
    email      TEXT,
    url        TEXT,
    body       TEXT NOT NULL,
    status     TEXT NOT NULL DEFAULT 'pending',
    ip         TEXT,
    user_agent TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX comments_content_status ON comments(content_id, status);
CREATE INDEX comments_status_created ON comments(status, created_at);

CREATE TABLE revisions (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL,
    status     TEXT NOT NULL,
    actor      TEXT NOT NULL,
    note       TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX revisions_content_created ON revisions(content_id, created_at);

CREATE TABLE links (
    source_id    INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
    target_title TEXT NOT NULL,
    target_id    INTEGER REFERENCES contents(id) ON DELETE SET NULL,
    PRIMARY KEY(source_id, target_title)
);

CREATE INDEX links_target ON links(target_id);

CREATE TABLE media (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    storage_key   TEXT NOT NULL UNIQUE,
    original_name TEXT NOT NULL,
    mime          TEXT NOT NULL,
    size          INTEGER NOT NULL,
    width         INTEGER,
    height        INTEGER,
    uploader_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    ip         TEXT,
    user_agent TEXT
);

CREATE INDEX sessions_expires ON sessions(expires_at);

CREATE TABLE api_tokens (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    scopes       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at   INTEGER
);

CREATE TABLE settings (
    name  TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
