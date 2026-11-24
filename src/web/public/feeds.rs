//! RSS, Atom, sitemap and robots.txt.

use super::*;

pub async fn feed_rss(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        limit: 20,
        ..Default::default()
    };
    let (posts, _) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let mut items = String::new();

    for post in &posts {
        let view =
            content::get_view(&state.db, post.content.id, site.timezone_offset, false).await?;
        let description = if site.feed_full_text {
            view.html
        } else {
            view.excerpt_text
        };
        let link = format!(
            "{}/archives/{}",
            site.site_url,
            path_segment(&post.content.slug)
        );
        let published = post.content.published_at.unwrap_or(post.content.created_at);
        items.push_str(&format!(
            "<item><title>{}</title><link>{}</link><guid isPermaLink=\"true\">{}</guid>\
             <pubDate>{}</pubDate><description>{}</description></item>",
            xml_escape(&post.content.title),
            xml_escape(&link),
            xml_escape(&link),
            format_date(published, site.timezone_offset, "%a, %d %b %Y %H:%M:%S %z"),
            xml_escape(&description),
        ));
    }

    let last_build = posts
        .first()
        .map(|post| post.content.updated_at)
        .unwrap_or_else(now_ts);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <rss version=\"2.0\"><channel><title>{}</title><link>{}/</link>\
         <description>{}</description><language>zh-cn</language>\
         <lastBuildDate>{}</lastBuildDate>{}</channel></rss>",
        xml_escape(&site.title),
        xml_escape(&site.site_url),
        xml_escape(&site.description),
        format_date(last_build, site.timezone_offset, "%a, %d %b %Y %H:%M:%S %z"),
        items,
    );

    Ok(([(CONTENT_TYPE, "application/rss+xml; charset=utf-8")], xml).into_response())
}

pub async fn feed_atom(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let filter = content::ContentFilter {
        kind: Some("post".to_string()),
        status: Some("published".to_string()),
        limit: 20,
        ..Default::default()
    };
    let (posts, _) = content::list(&state.db, &filter, site.timezone_offset).await?;
    let mut entries = String::new();

    for post in &posts {
        let view =
            content::get_view(&state.db, post.content.id, site.timezone_offset, false).await?;
        let content = if site.feed_full_text {
            view.html
        } else {
            view.excerpt_text
        };
        let link = format!(
            "{}/archives/{}",
            site.site_url,
            path_segment(&post.content.slug)
        );
        let published = post.content.published_at.unwrap_or(post.content.created_at);
        entries.push_str(&format!(
            "<entry><title>{}</title><id>{}</id><link href=\"{}\"/>\
             <updated>{}</updated><published>{}</published>\
             <content type=\"html\">{}</content></entry>",
            xml_escape(&post.content.title),
            xml_escape(&link),
            xml_escape(&link),
            format_iso(post.content.updated_at, site.timezone_offset),
            format_iso(published, site.timezone_offset),
            xml_escape(&content),
        ));
    }

    let updated = posts
        .first()
        .map(|post| post.content.updated_at)
        .unwrap_or_else(now_ts);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <feed xmlns=\"http://www.w3.org/2005/Atom\"><title>{}</title>\
         <id>{}/</id><link href=\"{}/\"/><updated>{}</updated>\
         <subtitle>{}</subtitle>{}</feed>",
        xml_escape(&site.title),
        xml_escape(&site.site_url),
        xml_escape(&site.site_url),
        format_iso(updated, site.timezone_offset),
        xml_escape(&site.description),
        entries,
    );

    Ok(([(CONTENT_TYPE, "application/atom+xml; charset=utf-8")], xml).into_response())
}

pub async fn sitemap(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let mut urls = vec![format!(
        "<url><loc>{}/</loc><changefreq>daily</changefreq></url>",
        xml_escape(&site.site_url)
    )];

    let (posts, _) = content::list(
        &state.db,
        &content::ContentFilter {
            kind: Some("post".to_string()),
            status: Some("published".to_string()),
            limit: 500,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    for post in posts {
        urls.push(format!(
            "<url><loc>{}/archives/{}</loc><lastmod>{}</lastmod></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&post.content.slug)),
            format_iso(post.content.updated_at, site.timezone_offset)
        ));
    }

    let (pages, _) = content::list(
        &state.db,
        &content::ContentFilter {
            kind: Some("page".to_string()),
            status: Some("published".to_string()),
            limit: 500,
            ..Default::default()
        },
        site.timezone_offset,
    )
    .await?;
    for page in pages {
        urls.push(format!(
            "<url><loc>{}/pages/{}</loc><lastmod>{}</lastmod></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&page.content.slug)),
            format_iso(page.content.updated_at, site.timezone_offset)
        ));
    }

    for term in term::list_public(&state.db, "category").await? {
        urls.push(format!(
            "<url><loc>{}/categories/{}</loc></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&term.term.slug))
        ));
    }
    for term in term::list_public(&state.db, "tag").await? {
        urls.push(format!(
            "<url><loc>{}/tags/{}</loc></url>",
            xml_escape(&site.site_url),
            xml_escape(&path_segment(&term.term.slug))
        ));
    }

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">{}</urlset>",
        urls.join("")
    );
    Ok(([(CONTENT_TYPE, "application/xml; charset=utf-8")], xml).into_response())
}

pub async fn robots(State(state): State<AppState>) -> AppResult<Response> {
    let site = crate::settings::load(&state.db, &state.config.site_url).await?;
    let body = format!(
        "User-agent: *\nAllow: /\nDisallow: /admin\n\nSitemap: {}/sitemap.xml\n",
        site.site_url
    );
    Ok(([(CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response())
}
