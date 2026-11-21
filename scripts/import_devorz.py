#!/usr/bin/env python3
"""
Import public articles from a Typecho/Cactus blog into inkwell.

Only use this for content you own or have permission to migrate.

Dependencies:
    pip install requests beautifulsoup4 html2text
"""

from __future__ import annotations

import argparse
import hashlib
import json
import mimetypes
import os
import re
import sqlite3
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from urllib.parse import unquote, urljoin, urlparse

import html2text
import requests
from bs4 import BeautifulSoup, Tag

USER_AGENT = (
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/124.0 Safari/537.36 inkwell-importer/0.1"
)
TZ = timezone(timedelta(hours=8))
ALLOWED_IMAGE_EXT = {"png", "jpg", "jpeg", "gif", "webp", "avif"}


def slugify(value: str) -> str:
    out: list[str] = []
    last_dash = False
    for ch in value.strip():
        if ch.isalnum():
            out.append(ch.lower())
            last_dash = False
        elif ch.isspace() or ch in "-_/.:":
            if not last_dash and out:
                out.append("-")
                last_dash = True
    result = "".join(out).strip("-")[:120]
    return result or "item"


def fetch(session: requests.Session, url: str) -> requests.Response:
    response = session.get(url, timeout=45)
    response.raise_for_status()
    return response


def discover_articles(session: requests.Session, site: str) -> list[str]:
    """Walk the theme's paginated #post-list and collect article URLs."""
    site = site.rstrip("/") + "/"
    urls: list[str] = []
    seen: set[str] = set()

    for page in range(1, 50):
        page_url = site if page == 1 else urljoin(site, f"index.php/page/{page}/")
        response = session.get(page_url, timeout=45)
        if response.status_code != 200:
            break
        soup = BeautifulSoup(response.text, "html.parser")
        links = [
            urljoin(page_url, anchor.get("href", ""))
            for anchor in soup.select("#post-list a[href]")
        ]
        new_links = [
            url
            for url in links
            if "/archives/" in url and url.endswith(".html") and url not in seen
        ]
        if not new_links:
            break
        for url in new_links:
            seen.add(url)
            urls.append(url)
        print(f"  page {page}: {len(new_links)} articles")

    return urls


def text_to_markdown(node: Tag) -> str:
    converter = html2text.HTML2Text()
    converter.body_width = 0
    converter.ignore_images = False
    converter.ignore_links = False
    converter.ignore_emphasis = False
    converter.unicode_snob = True
    return converter.handle(str(node)).strip()


def table_to_markdown(table: Tag) -> str:
    rows: list[list[str]] = []
    for tr in table.find_all("tr"):
        cells = [
            cell_to_markdown(cell)
            for cell in tr.find_all(["th", "td"], recursive=False)
        ]
        if cells:
            rows.append(cells)

    if not rows:
        return ""

    width = max(len(row) for row in rows)
    rows = [row + [""] * (width - len(row)) for row in rows]
    header = rows[0]
    body = rows[1:]
    lines = [
        "| " + " | ".join(header) + " |",
        "| " + " | ".join(["---"] * width) + " |",
    ]
    for row in body:
        lines.append("| " + " | ".join(row) + " |")
    return "\n".join(lines)


def cell_to_markdown(cell: Tag) -> str:
    text = text_to_markdown(cell)
    text = re.sub(r"\s+", " ", text).strip()
    return text.replace("|", r"\|")


def code_language(pre: Tag) -> str:
    code = pre.find("code")
    if not code:
        return ""
    for class_name in code.get("class", []):
        if class_name.startswith("lang-"):
            return normalize_language(class_name[5:])
        if class_name.startswith("language-"):
            return normalize_language(class_name[9:])
    return ""


def normalize_language(language: str) -> str:
    language = language.strip().lower()
    return {
        "js": "javascript",
        "ts": "typescript",
        "sh": "bash",
        "shell": "bash",
        "yml": "yaml",
        "objc": "objectivec",
    }.get(language, language)


def fence_for(code: str) -> str:
    return "````" if "```" in code else "```"


def download_image(
    session: requests.Session,
    url: str,
    uploads_dir: Path,
    conn: sqlite3.Connection,
    admin_id: int,
) -> str:
    parsed = urlparse(url)
    suffix = Path(parsed.path).suffix.lower().lstrip(".")
    if suffix == "svg":
        # Do not host author-supplied SVG: it can execute script when opened directly.
        return url

    try:
        response = session.get(url, timeout=45)
        response.raise_for_status()
        content_type = response.headers.get("content-type", "").split(";")[0].strip()
        if not content_type.startswith("image/"):
            return url
        if not suffix:
            guessed = mimetypes.guess_extension(content_type) or ".bin"
            suffix = guessed.lstrip(".")
        if suffix not in ALLOWED_IMAGE_EXT:
            return url
        data = response.content
    except Exception as error:  # noqa: BLE001
        print(f"    image skipped: {url} ({error})")
        return url

    digest = hashlib.sha256(data).hexdigest()[:20]
    storage_key = f"imported/devorz/{digest}.{suffix}"
    target = uploads_dir / storage_key
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists():
        target.write_bytes(data)

    row = conn.execute(
        "SELECT id FROM media WHERE storage_key = ?", (storage_key,)
    ).fetchone()
    if not row:
        conn.execute(
            "INSERT INTO media(storage_key, original_name, mime, size, width, height, uploader_id, created_at) "
            "VALUES(?, ?, ?, ?, NULL, NULL, ?, ?)",
            (
                storage_key,
                unquote(Path(parsed.path).name) or f"{digest}.{suffix}",
                content_type or "application/octet-stream",
                len(data),
                admin_id,
                int(time.time()),
            ),
        )
    return f"/media/{storage_key}"


def article_to_markdown(
    content: Tag,
    article_url: str,
    session: requests.Session,
    uploads_dir: Path,
    conn: sqlite3.Connection,
    admin_id: int,
    no_images: bool,
) -> str:
    # Remove the theme's appended "本文链接" block.
    for heading in content.find_all(["h1", "h2", "h3"]):
        if "本文链接" in heading.get_text():
            for sibling in list(heading.next_siblings):
                sibling.extract()
            heading.extract()
            break

    image_tokens: dict[str, str] = {}
    if not no_images:
        for index, image in enumerate(list(content.find_all("img"))):
            src = image.get("src", "").strip()
            if not src or src.startswith("data:"):
                continue
            absolute = urljoin(article_url, src)
            local = download_image(session, absolute, uploads_dir, conn, admin_id)
            alt = image.get("alt", "").strip()
            token = f"IMAGEBLOCKTOKEN{index}XYZ"
            image_tokens[token] = f"![{alt}]({local})"
            image.replace_with(token)

    table_tokens: dict[str, str] = {}
    for index, table in enumerate(list(content.find_all("table"))):
        token = f"TABLEBLOCKTOKEN{index}XYZ"
        table_tokens[token] = table_to_markdown(table)
        table.replace_with(token)

    code_tokens: dict[str, str] = {}
    for index, pre in enumerate(list(content.find_all("pre"))):
        code = pre.find("code")
        code_text = (code or pre).get_text()
        language = code_language(pre)
        fence = fence_for(code_text)
        token = f"CODEBLOCKTOKEN{index}XYZ"
        code_tokens[token] = f"{fence}{language}\n{code_text.rstrip()}\n{fence}"
        pre.replace_with(token)

    converter = html2text.HTML2Text()
    converter.body_width = 0
    converter.ignore_images = False
    converter.ignore_links = False
    converter.ignore_emphasis = False
    converter.unicode_snob = True
    markdown = converter.handle(str(content))

    for token, replacement in {**image_tokens, **table_tokens, **code_tokens}.items():
        markdown = markdown.replace(token, replacement)

    markdown = re.split(r"\n#{1,6}\s*本文链接", markdown)[0]
    markdown = re.sub(r"\n{3,}", "\n\n", markdown)
    return markdown.strip()


def mcp_tool(
    session: requests.Session, mcp_url: str, token: str, name: str, arguments: dict
) -> dict:
    response = session.post(
        mcp_url,
        json={
            "jsonrpc": "2.0",
            "id": int(time.time() * 1000),
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        },
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        timeout=60,
    )
    response.raise_for_status()
    payload = response.json()
    if "error" in payload:
        raise RuntimeError(payload["error"])
    result = payload["result"]
    if result.get("isError"):
        raise RuntimeError(result.get("content", [{}])[0].get("text", "MCP tool failed"))
    return result["structuredContent"]


def parse_article(
    session: requests.Session,
    url: str,
    uploads_dir: Path,
    conn: sqlite3.Connection,
    admin_id: int,
    no_images: bool,
) -> dict:
    response = fetch(session, url)
    soup = BeautifulSoup(response.text, "html.parser")

    title_tag = soup.select_one("h1.posttitle")
    if not title_tag:
        raise RuntimeError("missing h1.posttitle")
    title = title_tag.get_text(" ", strip=True)

    time_tag = soup.select_one("time[datetime]")
    if not time_tag:
        raise RuntimeError("missing time[datetime]")
    date = time_tag["datetime"].strip()
    published = datetime.strptime(date, "%Y-%m-%d").replace(
        hour=10, minute=0, second=0, tzinfo=TZ
    )

    category_tag = soup.select_one('.article-tag a[href*="/category/"]')
    category = category_tag.get_text(strip=True) if category_tag else "默认分类"

    tags: list[str] = []
    keywords_tag = soup.select_one('meta[name="keywords"]')
    if keywords_tag:
        tags.extend(
            tag.strip()
            for tag in re.split(r"[,，]", keywords_tag.get("content", ""))
            if tag.strip()
        )
    for tag_link in soup.select(".article-tag-box a"):
        tag = tag_link.get_text(strip=True)
        if tag and tag not in tags:
            tags.append(tag)

    content = soup.select_one("#post-content")
    if not content:
        raise RuntimeError("missing #post-content")
    markdown = article_to_markdown(
        content, url, session, uploads_dir, conn, admin_id, no_images
    )
    if not markdown:
        raise RuntimeError("empty article body")

    slug = slugify(Path(urlparse(url).path).stem)
    return {
        "url": url,
        "title": title,
        "slug": slug,
        "date": published,
        "category": category,
        "tags": tags,
        "markdown": markdown,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site", default="https://www.devorz.com/")
    parser.add_argument("--mcp-url", default="http://127.0.0.1:3000/mcp")
    parser.add_argument("--token", default=os.environ.get("MCP_TOKEN", ""))
    parser.add_argument("--db", default="data/typecho.db")
    parser.add_argument("--uploads", default="data/uploads")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-images", action="store_true")
    args = parser.parse_args()

    if not args.dry_run and not args.token:
        parser.error("--token or MCP_TOKEN is required")

    db_path = Path(args.db).resolve()
    uploads_dir = Path(args.uploads).resolve()
    if not db_path.exists():
        parser.error(f"database not found: {db_path}")

    conn = sqlite3.connect(db_path, timeout=30)
    conn.execute("PRAGMA busy_timeout = 30000")
    admin_row = conn.execute("SELECT id FROM users ORDER BY id LIMIT 1").fetchone()
    if not admin_row:
        parser.error("no admin user found")
    admin_id = admin_row[0]

    session = requests.Session()
    session.headers.update({"User-Agent": USER_AGENT})

    print(f"Discovering articles from {args.site}")
    urls = discover_articles(session, args.site)
    if args.limit:
        urls = urls[: args.limit]
    print(f"Found {len(urls)} articles")

    imported = 0
    skipped = 0
    failed = 0

    for index, url in enumerate(urls, start=1):
        print(f"[{index}/{len(urls)}] {url}")
        try:
            article = parse_article(
                session,
                url,
                uploads_dir,
                conn,
                admin_id,
                args.no_images,
            )
        except Exception as error:  # noqa: BLE001
            failed += 1
            print(f"  failed to parse: {error}")
            continue

        existing = conn.execute(
            "SELECT id FROM contents WHERE title = ? OR slug = ?",
            (article["title"], article["slug"]),
        ).fetchone()
        if existing:
            skipped += 1
            print(f"  skipped, already exists (id={existing[0]})")
            continue

        print(
            f"  {article['date'].date()} | {article['title']} | "
            f"{len(article['markdown'])} chars | tags={','.join(article['tags'])}"
        )

        if args.dry_run:
            imported += 1
            continue

        # parse_article may have inserted media rows while downloading images.
        # Release the SQLite write lock before the app handles content_create.
        conn.commit()

        try:
            result = mcp_tool(
                session,
                args.mcp_url,
                args.token,
                "content_create",
                {
                    "kind": "post",
                    "title": article["title"],
                    "slug": article["slug"],
                    "body": article["markdown"],
                    "status": "draft",
                    "visibility": "public",
                    "category": article["category"],
                    "tags": article["tags"],
                    "allow_comments": True,
                },
            )
            content_id = int(result["id"])
        except Exception as error:  # noqa: BLE001
            failed += 1
            conn.rollback()
            print(f"  MCP create failed: {error}")
            continue

        timestamp = int(article["date"].timestamp())
        metadata = json.dumps(
            {"source_url": article["url"], "source_site": "devorz.com"},
            ensure_ascii=False,
        )
        conn.execute(
            "UPDATE contents SET status = 'published', visibility = 'public', "
            "published_at = ?, created_at = ?, updated_at = ?, review_status = 'none', "
            "metadata = ? WHERE id = ?",
            (timestamp, timestamp, timestamp, metadata, content_id),
        )
        conn.execute(
            "INSERT INTO revisions(content_id, title, body, status, actor, note, created_at) "
            "VALUES(?, ?, ?, 'published', 'import:devorz', '导入自 devorz.com', ?)",
            (content_id, article["title"], article["markdown"], timestamp),
        )
        conn.commit()
        imported += 1
        print(f"  imported id={content_id}, slug={result.get('slug')}")

    conn.commit()
    conn.close()
    print(
        f"Done. imported={imported}, skipped={skipped}, failed={failed}, "
        f"dry_run={args.dry_run}"
    )
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
