CREATE INDEX contents_timeline
    ON contents(kind, status, visibility, COALESCE(published_at, created_at) DESC, id DESC)
    WHERE deleted_at IS NULL;
