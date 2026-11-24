//! Wiki links: syncing the links table and resolving [[titles]] to targets.

use super::*;

pub(super) async fn build_wiki_map(
    pool: &SqlitePool,
    source_id: i64,
    for_admin: bool,
) -> AppResult<HashMap<String, WikiTarget>> {
    let links: Vec<(String, Option<i64>)> =
        sqlx::query_as("SELECT target_title, target_id FROM links WHERE source_id = ?")
            .bind(source_id)
            .fetch_all(pool)
            .await?;
    wiki_map_for_links(pool, links, for_admin).await
}

pub(super) async fn wiki_map_for_links(
    pool: &SqlitePool,
    links: Vec<(String, Option<i64>)>,
    for_admin: bool,
) -> AppResult<HashMap<String, WikiTarget>> {
    let target_ids: Vec<i64> = links.iter().filter_map(|(_, id)| *id).collect();
    let targets = load_by_ids(pool, &target_ids).await?;
    let promoted_ids: Vec<i64> = targets
        .values()
        .filter_map(|target| target.promoted_to_id)
        .collect();
    let promoted = load_by_ids(pool, &promoted_ids).await?;

    let resolved: Vec<(String, WikiTarget)> = links
        .into_iter()
        .map(|(title, target_id)| {
            let target = target_id.and_then(|id| targets.get(&id));
            let wiki = WikiTarget {
                label: target.map_or_else(|| title.clone(), |target| target.title.clone()),
                href: target.and_then(|target| target_href(target, &promoted, for_admin)),
            };
            (title, wiki)
        })
        .collect();

    // Exact titles win over the case-insensitive fallback keys, so [[Rust]]
    // and [[rust]] in one body each resolve to their own target.
    let mut map: HashMap<String, WikiTarget> = resolved.iter().cloned().collect();
    for (title, wiki) in resolved {
        map.entry(title.to_lowercase()).or_insert(wiki);
    }
    Ok(map)
}

pub(super) fn target_href(
    target: &Content,
    promoted: &HashMap<i64, Content>,
    for_admin: bool,
) -> Option<String> {
    if for_admin {
        return Some(format!(
            "/admin/{}/{}/edit",
            crate::models::kind_to_path(&target.kind),
            target.id
        ));
    }

    if target.is_public() {
        return Some(match target.kind.as_str() {
            "page" => format!("/pages/{}", target.slug),
            _ => format!("/archives/{}", target.slug),
        });
    }

    target
        .promoted_to_id
        .and_then(|post_id| promoted.get(&post_id))
        .filter(|post| post.is_public())
        .map(|post| format!("/archives/{}", post.slug))
}

pub(super) async fn sync_links_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    source_id: i64,
    body: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM links WHERE source_id = ?")
        .bind(source_id)
        .execute(&mut **transaction)
        .await?;

    for title in markdown::wiki_titles(body) {
        let target_id = resolve_note_id(&mut **transaction, &title).await?;

        sqlx::query(
            "INSERT OR REPLACE INTO links(source_id, target_title, target_id) VALUES(?, ?, ?)",
        )
        .bind(source_id)
        .bind(&title)
        .bind(target_id)
        .execute(&mut **transaction)
        .await?;

        if let Some(target_id) = target_id {
            sqlx::query(
                "UPDATE links SET target_id = ? WHERE target_id IS NULL AND target_title = ? COLLATE NOCASE",
            )
            .bind(target_id)
            .bind(&title)
            .execute(&mut **transaction)
            .await?;
        }
    }

    Ok(())
}

/// The note a `[[title]]` points to: matched by title or slug, newest first.
pub(super) async fn resolve_note_id<'e, E>(executor: E, title: &str) -> AppResult<Option<i64>>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    Ok(sqlx::query_scalar(
        "SELECT id FROM contents WHERE kind = 'note' AND deleted_at IS NULL \
         AND (title = ? COLLATE NOCASE OR slug = ? COLLATE NOCASE) \
         ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(title)
    .bind(title)
    .fetch_optional(executor)
    .await?)
}

pub(in crate::repo) async fn resolve_pending_links(
    pool: &SqlitePool,
    target_id: i64,
    title: &str,
    slug: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE links SET target_id = ? WHERE target_id IS NULL \
         AND (target_title = ? COLLATE NOCASE OR target_title = ? COLLATE NOCASE)",
    )
    .bind(target_id)
    .bind(title)
    .bind(slug)
    .execute(pool)
    .await?;
    Ok(())
}
