CREATE TABLE content_proposals (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    content_id          INTEGER NOT NULL REFERENCES contents(id) ON DELETE CASCADE,
    base_revision_id    INTEGER REFERENCES revisions(id) ON DELETE SET NULL,
    base_updated_at     INTEGER NOT NULL,
    title               TEXT NOT NULL,
    body                TEXT NOT NULL,
    excerpt             TEXT,
    summary             TEXT,
    status              TEXT NOT NULL DEFAULT 'proposed',
    author              TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    resolved_at         INTEGER,
    decision_note       TEXT,
    applied_revision_id INTEGER REFERENCES revisions(id) ON DELETE SET NULL
);

CREATE INDEX content_proposals_content_status
    ON content_proposals(content_id, status, created_at);
CREATE INDEX content_proposals_status_created
    ON content_proposals(status, created_at);
