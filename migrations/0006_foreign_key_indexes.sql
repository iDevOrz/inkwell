-- Foreign-key columns without an index force a full scan of the child table
-- for every parent row deleted (e.g. each revision removed by a cascading
-- content delete scans content_proposals twice).
CREATE INDEX contents_promoted_to ON contents(promoted_to_id)
    WHERE promoted_to_id IS NOT NULL;
CREATE INDEX content_proposals_base_revision ON content_proposals(base_revision_id)
    WHERE base_revision_id IS NOT NULL;
CREATE INDEX content_proposals_applied_revision ON content_proposals(applied_revision_id)
    WHERE applied_revision_id IS NOT NULL;

-- Pending wiki links are re-targeted by case-insensitive title match.
CREATE INDEX links_pending_title ON links(target_title COLLATE NOCASE)
    WHERE target_id IS NULL;
