CREATE TABLE annotations (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id    INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL DEFAULT 'inline',
    quote         TEXT,
    prefix        TEXT,
    suffix        TEXT,
    start_offset  INTEGER,
    end_offset    INTEGER,
    comment       TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'open',
    author        TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    resolved_at   INTEGER
);

CREATE INDEX annotations_content_status ON annotations(content_id, status, created_at);
CREATE INDEX annotations_status_created ON annotations(status, created_at);

CREATE TABLE annotation_replies (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    annotation_id INTEGER NOT NULL REFERENCES annotations(id) ON DELETE CASCADE,
    author        TEXT NOT NULL,
    body          TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE INDEX annotation_replies_annotation ON annotation_replies(annotation_id, created_at);
