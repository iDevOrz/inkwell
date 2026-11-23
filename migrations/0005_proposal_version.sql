ALTER TABLE content_proposals
    ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE TABLE content_proposal_versions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    proposal_id INTEGER NOT NULL REFERENCES content_proposals(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    excerpt     TEXT,
    summary     TEXT,
    author      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    UNIQUE(proposal_id, version)
);

CREATE INDEX content_proposal_versions_proposal
    ON content_proposal_versions(proposal_id, version);

INSERT INTO content_proposal_versions(
    proposal_id, version, title, body, excerpt, summary, author, created_at
)
SELECT id, version, title, body, excerpt, summary, author, created_at
FROM content_proposals;
