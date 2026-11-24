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
import ipaddress
import json
import mimetypes
import os
import re
import secrets
import socket
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
IMPORT_PROBE_SETTING = "__inkwell_import_target_probe__"
REQUEST_TIMEOUT_SECONDS = 45
MAX_PAGE_BYTES = 5 * 1024 * 1024
MAX_IMAGE_BYTES = 20 * 1024 * 1024
MAX_REDIRECTS = 5
DOWNLOAD_CHUNK_BYTES = 64 * 1024
ALLOWED_FETCH_SCHEMES = {"http", "https"}


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


def ensure_public_url(url: str) -> None:
    """Reject non-HTTP(S) URLs and hosts that resolve to non-public addresses."""
    parsed = urlparse(url)
    if parsed.scheme not in ALLOWED_FETCH_SCHEMES or not parsed.hostname:
        raise ValueError(f"refusing to fetch non-http(s) URL: {url}")
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    try:
        infos = socket.getaddrinfo(parsed.hostname, port, type=socket.SOCK_STREAM)
    except socket.gaierror as error:
        raise ValueError(f"cannot resolve {parsed.hostname}: {error}") from error
    for info in infos:
        address = ipaddress.ip_address(info[4][0].split("%")[0])
        if (
            not address.is_global
            or address.is_private
            or address.is_loopback
            or address.is_link_local
            or address.is_multicast
            or address.is_reserved
            or address.is_unspecified
        ):
            raise ValueError(
                f"refusing to fetch {url}: {parsed.hostname} resolves to non-public {address}"
            )


def read_limited(response: requests.Response, max_bytes: int) -> bytes:
    declared = response.headers.get("content-length", "")
    if declared.isdigit() and int(declared) > max_bytes:
        raise ValueError(f"response too large ({declared} bytes > {max_bytes})")
    chunks: list[bytes] = []
    total = 0
    for chunk in response.iter_content(chunk_size=DOWNLOAD_CHUNK_BYTES):
        total += len(chunk)
        if total > max_bytes:
            raise ValueError(f"response exceeds {max_bytes} bytes")
        chunks.append(chunk)
    return b"".join(chunks)


def fetch_limited(
    session: requests.Session, url: str, max_bytes: int = MAX_PAGE_BYTES
) -> requests.Response:
    """GET a public URL, validating every redirect hop and capping the body size.

    The returned response has its body fully read (``.content``/``.text`` work)
    and is not checked for HTTP errors.
    """
    for _ in range(MAX_REDIRECTS + 1):
        ensure_public_url(url)
        response = session.get(
            url, timeout=REQUEST_TIMEOUT_SECONDS, stream=True, allow_redirects=False
        )
        try:
            if response.is_redirect:
                url = urljoin(url, response.headers.get("location", ""))
                continue
            data = read_limited(response, max_bytes)
        finally:
            response.close()
        # Hand back a regular response whose body is already buffered.
        response._content = data  # noqa: SLF001
        response._content_consumed = True  # noqa: SLF001
        return response
    raise ValueError(f"too many redirects (> {MAX_REDIRECTS})")


def fetch(
    session: requests.Session, url: str, max_bytes: int = MAX_PAGE_BYTES
) -> requests.Response:
    response = fetch_limited(session, url, max_bytes)
    response.raise_for_status()
    return response


def discover_articles(session: requests.Session, site: str) -> list[str]:
    """Walk the theme's paginated #post-list and collect article URLs."""
    site = site.rstrip("/") + "/"
    urls: list[str] = []
    seen: set[str] = set()

    for page in range(1, 50):
        page_url = site if page == 1 else urljoin(site, f"index.php/page/{page}/")
        response = fetch_limited(session, page_url)
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
    longest = max((len(run) for run in re.findall(r"`+", code)), default=0)
    return "`" * max(3, longest + 1)


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
        response = fetch(session, url, MAX_IMAGE_BYTES)
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
    dry_run: bool,
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
            local = (
                absolute
                if dry_run
                else download_image(session, absolute, uploads_dir, conn, admin_id)
            )
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


class AlreadyImported(Exception):
    def __init__(self, content_id: int) -> None:
        super().__init__(f"already exists (id={content_id})")
        self.content_id = content_id


def slug_from_url(url: str) -> str:
    return slugify(Path(urlparse(url).path).stem)


def find_existing(conn: sqlite3.Connection, title: str | None, slug: str) -> int | None:
    row = conn.execute(
        "SELECT id FROM contents WHERE title = ? OR slug = ?",
        (title, slug),
    ).fetchone()
    return row[0] if row else None


def parse_article(
    session: requests.Session,
    url: str,
    uploads_dir: Path,
    conn: sqlite3.Connection,
    admin_id: int,
    no_images: bool,
    dry_run: bool,
) -> dict:
    response = fetch(session, url)
    soup = BeautifulSoup(response.text, "html.parser")

    title_tag = soup.select_one("h1.posttitle")
    if not title_tag:
        raise RuntimeError("missing h1.posttitle")
    title = title_tag.get_text(" ", strip=True)
    slug = slug_from_url(url)
    # Check before converting the body so reruns never re-download images
    # or leave media rows behind for skipped articles.
    existing_id = find_existing(conn, title, slug)
    if existing_id is not None:
        raise AlreadyImported(existing_id)

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
        content, url, session, uploads_dir, conn, admin_id, no_images, dry_run
    )
    if not markdown:
        raise RuntimeError("empty article body")

    return {
        "url": url,
        "title": title,
        "slug": slug,
        "date": published,
        "category": category,
        "tags": tags,
        "markdown": markdown,
    }


def connect_database(db_path: Path, read_only: bool) -> sqlite3.Connection:
    if read_only:
        conn = sqlite3.connect(f"{db_path.as_uri()}?mode=ro", uri=True, timeout=30)
        conn.execute("PRAGMA query_only = ON")
    else:
        conn = sqlite3.connect(db_path, timeout=30)
    conn.execute("PRAGMA busy_timeout = 30000")
    return conn


def verify_instance_target(
    conn: sqlite3.Connection,
    db_path: Path,
    session: requests.Session,
    mcp_url: str,
    token: str,
) -> None:
    local_instance = conn.execute(
        "SELECT value FROM settings WHERE name = 'installation_id'"
    ).fetchone()
    local_runtime = conn.execute(
        "SELECT value FROM settings WHERE name = 'runtime_id'"
    ).fetchone()
    if not local_instance or not local_runtime:
        raise RuntimeError(
            "database has no instance identity; start a current Inkwell instance and retry"
        )
    remote = mcp_tool(
        session, mcp_url, token, "instance_info", {}
    )
    if (
        remote.get("installation_id") != local_instance[0]
        or remote.get("runtime_id") != local_runtime[0]
    ):
        raise RuntimeError(
            "MCP endpoint and --db are different Inkwell instances; no import was started"
        )

    probe = secrets.token_urlsafe(24)
    probe_conn = connect_database(db_path, read_only=False)
    try:
        probe_conn.execute(
            "INSERT INTO settings(name, value) VALUES(?, ?) \
             ON CONFLICT(name) DO UPDATE SET value = excluded.value",
            (IMPORT_PROBE_SETTING, probe),
        )
        probe_conn.commit()
        remote_probe = mcp_tool(
            session, mcp_url, token, "instance_info", {"probe": probe}
        )
        if not remote_probe.get("probe_matches"):
            raise RuntimeError(
                "MCP endpoint cannot see the local database probe; refusing a different database file"
            )
    finally:
        try:
            probe_conn.execute(
                "DELETE FROM settings WHERE name = ? AND value = ?",
                (IMPORT_PROBE_SETTING, probe),
            )
            probe_conn.commit()
        finally:
            probe_conn.close()


def verify_created_draft(
    conn: sqlite3.Connection,
    content_id: int,
    title: str,
    slug: str,
    body: str,
) -> bool:
    row = conn.execute(
        "SELECT title, slug, body, status, visibility FROM contents WHERE id = ?",
        (content_id,),
    ).fetchone()
    return row == (title, slug, body, "draft", "public")


DATABASE_FILE = "inkwell.db"
# Default name before the rename; existing installs keep it (see the naming ADR).
LEGACY_DATABASE_FILE = "typecho.db"


def default_db_path(data_dir: str) -> str:
    """Mirror the server's default: prefer inkwell.db, fall back to typecho.db."""
    current = Path(data_dir) / DATABASE_FILE
    legacy = Path(data_dir) / LEGACY_DATABASE_FILE
    if not current.exists() and legacy.exists():
        return str(legacy)
    return str(current)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site", default="https://www.devorz.com/")
    parser.add_argument("--mcp-url", default="http://127.0.0.1:3000/mcp")
    parser.add_argument("--token", default=os.environ.get("MCP_TOKEN", ""))
    parser.add_argument("--db", default=default_db_path("data"))
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

    # Keep the local database read-only until the MCP endpoint is verified.
    conn = connect_database(db_path, read_only=True)
    admin_row = conn.execute("SELECT id FROM users ORDER BY id LIMIT 1").fetchone()
    if not admin_row:
        parser.error("no admin user found")
    admin_id = admin_row[0]

    session = requests.Session()
    session.headers.update({"User-Agent": USER_AGENT})

    if not args.dry_run:
        try:
            verify_instance_target(conn, db_path, session, args.mcp_url, args.token)
        except Exception as error:  # noqa: BLE001
            conn.close()
            parser.error(f"target verification failed before writing: {error}")
        conn.close()
        conn = connect_database(db_path, read_only=False)
        admin_row = conn.execute("SELECT id FROM users ORDER BY id LIMIT 1").fetchone()
        if not admin_row:
            parser.error("no admin user found")
        admin_id = admin_row[0]

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
        existing_id = find_existing(conn, None, slug_from_url(url))
        if existing_id is not None:
            skipped += 1
            print(f"  skipped, already exists (id={existing_id})")
            continue
        try:
            article = parse_article(
                session,
                url,
                uploads_dir,
                conn,
                admin_id,
                args.no_images,
                args.dry_run,
            )
        except AlreadyImported as error:
            skipped += 1
            print(f"  skipped, {error}")
            continue
        except Exception as error:  # noqa: BLE001
            failed += 1
            # Drop media rows inserted for an article that will not be imported.
            conn.rollback()
            print(f"  failed to parse: {error}")
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
            if not verify_created_draft(
                conn,
                content_id,
                article["title"],
                result["slug"],
                article["markdown"],
            ):
                try:
                    mcp_tool(
                        session,
                        args.mcp_url,
                        args.token,
                        "content_delete",
                        {"id": content_id},
                    )
                except Exception as cleanup_error:  # noqa: BLE001
                    print(f"  warning: could not trash unverified remote draft: {cleanup_error}")
                raise RuntimeError(
                    "the created draft does not match --db; refusing to publish by ID"
                )
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

    if not args.dry_run:
        conn.commit()
    conn.close()
    print(
        f"Done. imported={imported}, skipped={skipped}, failed={failed}, "
        f"dry_run={args.dry_run}"
    )
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
