import io
import sqlite3
import sys
import tempfile
import unittest
from contextlib import contextmanager, redirect_stderr, redirect_stdout
from datetime import datetime
from pathlib import Path
from unittest.mock import Mock, patch

import requests
from bs4 import BeautifulSoup
from requests.structures import CaseInsensitiveDict

import import_devorz


@contextmanager
def open_db(*args, **kwargs):
    """Like ``with sqlite3.connect(...)`` (commit on success) but also closes."""
    conn = sqlite3.connect(*args, **kwargs)
    try:
        with conn:
            yield conn
    finally:
        conn.close()


class ImporterSafetyTests(unittest.TestCase):
    def test_dry_run_keeps_remote_image_without_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            uploads_dir = Path(directory) / "uploads"
            conn = sqlite3.connect(":memory:")
            content = BeautifulSoup(
                '<div><p>Article</p><img src="/images/chart.png" alt="chart"></div>',
                "html.parser",
            ).div
            session = Mock()

            markdown = import_devorz.article_to_markdown(
                content,
                "https://source.example/articles/1.html",
                session,
                uploads_dir,
                conn,
                admin_id=1,
                no_images=False,
                dry_run=True,
            )

            self.assertIn(
                "![chart](https://source.example/images/chart.png)",
                markdown,
            )
            self.assertFalse(uploads_dir.exists())
            self.assertEqual(conn.total_changes, 0)
            session.get.assert_not_called()
            conn.close()

    def test_dry_run_database_connection_is_read_only(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            with open_db(db_path) as setup:
                setup.execute("CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT)")

            conn = import_devorz.connect_database(db_path, read_only=True)
            with self.assertRaises(sqlite3.OperationalError):
                conn.execute("INSERT INTO settings(name, value) VALUES('x', 'y')")
            conn.close()

    def test_import_rejects_a_different_mcp_instance_before_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            with open_db(db_path) as setup:
                setup.execute("CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT)")
                setup.executemany(
                    "INSERT INTO settings(name, value) VALUES(?, ?)",
                    [
                        ("installation_id", "local-installation"),
                        ("runtime_id", "local-runtime"),
                    ],
                )
            conn = import_devorz.connect_database(db_path, read_only=True)
            session = Mock()
            with patch.object(
                import_devorz,
                "mcp_tool",
                return_value={
                    "installation_id": "remote-installation",
                    "runtime_id": "remote-runtime",
                },
            ):
                with self.assertRaisesRegex(RuntimeError, "different Inkwell instances"):
                    import_devorz.verify_instance_target(
                        conn, db_path, session, "http://target.example/mcp", "token"
                    )
            self.assertEqual(
                conn.execute("SELECT COUNT(*) FROM settings").fetchone()[0],
                2,
            )
            conn.close()

    def test_import_rejects_a_cloned_database_and_cleans_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            with open_db(db_path) as setup:
                setup.execute("CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT)")
                setup.executemany(
                    "INSERT INTO settings(name, value) VALUES(?, ?)",
                    [
                        ("installation_id", "copied-installation"),
                        ("runtime_id", "copied-runtime"),
                    ],
                )
            conn = import_devorz.connect_database(db_path, read_only=True)
            session = Mock()
            with patch.object(
                import_devorz,
                "mcp_tool",
                return_value={
                    "installation_id": "copied-installation",
                    "runtime_id": "copied-runtime",
                    "probe_matches": False,
                },
            ):
                with self.assertRaisesRegex(RuntimeError, "database probe"):
                    import_devorz.verify_instance_target(
                        conn, db_path, session, "http://target.example/mcp", "token"
                    )
            self.assertIsNone(
                conn.execute(
                    "SELECT value FROM settings WHERE name = ?",
                    (import_devorz.IMPORT_PROBE_SETTING,),
                ).fetchone()
            )
            conn.close()

    def test_import_verifies_the_exact_shared_database_file(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            with open_db(db_path) as setup:
                setup.execute("CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT)")
                setup.executemany(
                    "INSERT INTO settings(name, value) VALUES(?, ?)",
                    [
                        ("installation_id", "same-installation"),
                        ("runtime_id", "same-runtime"),
                    ],
                )
            conn = import_devorz.connect_database(db_path, read_only=True)

            def mock_mcp_tool(_session, _url, _token, _name, arguments):
                probe_matches = False
                if "probe" in arguments:
                    with open_db(db_path) as remote_db:
                        row = remote_db.execute(
                            "SELECT value FROM settings WHERE name = ?",
                            (import_devorz.IMPORT_PROBE_SETTING,),
                        ).fetchone()
                    probe_matches = row == (arguments["probe"],)
                return {
                    "installation_id": "same-installation",
                    "runtime_id": "same-runtime",
                    "probe_matches": probe_matches,
                }

            with patch.object(import_devorz, "mcp_tool", side_effect=mock_mcp_tool):
                import_devorz.verify_instance_target(
                    conn, db_path, Mock(), "http://target.example/mcp", "token"
                )

            self.assertIsNone(
                conn.execute(
                    "SELECT value FROM settings WHERE name = ?",
                    (import_devorz.IMPORT_PROBE_SETTING,),
                ).fetchone()
            )
            conn.close()

    def test_main_stops_before_discovery_when_target_mismatches(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            uploads_dir = Path(directory) / "uploads"
            with open_db(db_path) as setup:
                setup.executescript(
                    "CREATE TABLE users(id INTEGER PRIMARY KEY);"
                    "CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT);"
                    "INSERT INTO users VALUES(1);"
                    "INSERT INTO settings VALUES('installation_id', 'local-installation');"
                    "INSERT INTO settings VALUES('runtime_id', 'local-runtime');"
                )

            argv = [
                "import_devorz.py",
                "--token",
                "test-token",
                "--db",
                str(db_path),
                "--uploads",
                str(uploads_dir),
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(
                    import_devorz,
                    "mcp_tool",
                    return_value={
                        "installation_id": "remote-installation",
                        "runtime_id": "remote-runtime",
                    },
                ),
                patch.object(import_devorz, "discover_articles") as discover,
                redirect_stdout(io.StringIO()),
                redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                import_devorz.main()

            discover.assert_not_called()
            self.assertFalse(uploads_dir.exists())
            with open_db(f"file:{db_path}?mode=ro", uri=True) as check:
                self.assertEqual(
                    check.execute("SELECT COUNT(*) FROM users").fetchone()[0],
                    1,
                )

    def test_dry_run_main_performs_no_database_or_upload_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            uploads_dir = Path(directory) / "uploads"
            with open_db(db_path) as setup:
                setup.executescript(
                    "CREATE TABLE users(id INTEGER PRIMARY KEY);"
                    "CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT);"
                    "CREATE TABLE contents(id INTEGER PRIMARY KEY, title TEXT, slug TEXT);"
                    "INSERT INTO users VALUES(1);"
                )
            article = {
                "title": "Preview only",
                "slug": "preview-only",
                "date": datetime(2026, 1, 1),
                "markdown": "Body with a remote image URL.",
                "tags": [],
            }
            argv = [
                "import_devorz.py",
                "--dry-run",
                "--db",
                str(db_path),
                "--uploads",
                str(uploads_dir),
            ]
            with (
                patch.object(sys, "argv", argv),
                patch.object(
                    import_devorz,
                    "discover_articles",
                    return_value=["https://source.example/articles/1.html"],
                ),
                patch.object(import_devorz, "parse_article", return_value=article),
                patch.object(import_devorz, "mcp_tool") as mcp,
                redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(import_devorz.main(), 0)

            mcp.assert_not_called()
            self.assertFalse(uploads_dir.exists())
            with open_db(f"file:{db_path}?mode=ro", uri=True) as check:
                self.assertEqual(
                    check.execute("SELECT COUNT(*) FROM contents").fetchone()[0],
                    0,
                )

    def test_publish_finalize_requires_the_created_draft_to_match(self):
        conn = sqlite3.connect(":memory:")
        conn.execute(
            "CREATE TABLE contents( \
                 id INTEGER PRIMARY KEY, title TEXT, slug TEXT, body TEXT, status TEXT, visibility TEXT \
             )"
        )
        conn.execute(
            "INSERT INTO contents VALUES(7, 'Imported', 'imported', 'Body', 'draft', 'public')"
        )
        self.assertTrue(
            import_devorz.verify_created_draft(
                conn, 7, "Imported", "imported", "Body"
            )
        )
        self.assertFalse(
            import_devorz.verify_created_draft(
                conn, 7, "Unrelated", "other", "Different body"
            )
        )
        conn.close()


PUBLIC_ADDRINFO = [(2, 1, 6, "", ("93.184.216.34", 443))]


def fake_response(status=200, body=b"", headers=None, location=None):
    response = requests.Response()
    response.status_code = status
    response.headers = CaseInsensitiveDict(headers or {})
    if location is not None:
        response.headers["location"] = location
    response.raw = io.BytesIO(body)
    return response


class FenceTests(unittest.TestCase):
    def test_fence_is_longer_than_longest_backtick_run(self):
        self.assertEqual(import_devorz.fence_for("print(1)"), "```")
        self.assertEqual(import_devorz.fence_for("use `x` here"), "```")
        self.assertEqual(import_devorz.fence_for("```py\ncode\n```"), "````")
        self.assertEqual(import_devorz.fence_for("a ```` b ``` c"), "`````")


class SafeFetchTests(unittest.TestCase):
    def test_rejects_non_http_schemes(self):
        for url in ("file:///etc/passwd", "ftp://example.com/x", "gopher://example.com"):
            with self.assertRaisesRegex(ValueError, "non-http"):
                import_devorz.ensure_public_url(url)

    def test_rejects_private_loopback_and_link_local_addresses(self):
        for address in ("127.0.0.1", "10.0.0.5", "192.168.1.1", "169.254.169.254", "::1"):
            info = [(2, 1, 6, "", (address, 80))]
            with patch.object(import_devorz.socket, "getaddrinfo", return_value=info):
                with self.assertRaisesRegex(ValueError, "non-public"):
                    import_devorz.ensure_public_url("http://internal.example/")

    def test_allows_public_address(self):
        with patch.object(
            import_devorz.socket, "getaddrinfo", return_value=PUBLIC_ADDRINFO
        ):
            import_devorz.ensure_public_url("https://source.example/a.html")

    def test_redirect_to_private_host_is_rejected_before_request(self):
        session = Mock()
        session.get.return_value = fake_response(
            status=302, location="http://metadata.internal/latest"
        )

        def resolve(host, *_args, **_kwargs):
            address = "169.254.169.254" if host == "metadata.internal" else "93.184.216.34"
            return [(2, 1, 6, "", (address, 80))]

        with patch.object(import_devorz.socket, "getaddrinfo", side_effect=resolve):
            with self.assertRaisesRegex(ValueError, "non-public"):
                import_devorz.fetch(session, "https://source.example/a.html")
        self.assertEqual(session.get.call_count, 1)
        _, kwargs = session.get.call_args
        self.assertTrue(kwargs["stream"])
        self.assertFalse(kwargs["allow_redirects"])

    def test_follows_public_redirect_and_buffers_body(self):
        session = Mock()
        final = fake_response(body=b"<html>ok</html>", headers={"content-type": "text/html"})
        session.get.side_effect = [
            fake_response(status=301, location="/moved.html"),
            final,
        ]
        with patch.object(
            import_devorz.socket, "getaddrinfo", return_value=PUBLIC_ADDRINFO
        ):
            response = import_devorz.fetch(session, "https://source.example/a.html")
        self.assertEqual(response.content, b"<html>ok</html>")
        self.assertEqual(
            session.get.call_args_list[1].args[0], "https://source.example/moved.html"
        )

    def test_body_over_limit_is_rejected(self):
        session = Mock()
        session.get.return_value = fake_response(body=b"x" * 20)
        with patch.object(
            import_devorz.socket, "getaddrinfo", return_value=PUBLIC_ADDRINFO
        ):
            with self.assertRaisesRegex(ValueError, "exceeds"):
                import_devorz.fetch(session, "https://source.example/a.png", max_bytes=10)

    def test_declared_length_over_limit_is_rejected_without_reading(self):
        # Empty body: only the Content-Length check can reject this response.
        session = Mock()
        response = fake_response(headers={"content-length": "999"})
        session.get.return_value = response
        with patch.object(
            import_devorz.socket, "getaddrinfo", return_value=PUBLIC_ADDRINFO
        ):
            with self.assertRaisesRegex(ValueError, "too large"):
                import_devorz.fetch(session, "https://source.example/a.png", max_bytes=10)


ARTICLE_HTML = """
<html><head><meta name="keywords" content="a,b"></head><body>
<h1 class="posttitle">Existing Title</h1>
<time datetime="2024-01-02"></time>
<div id="post-content"><p>Body</p><img src="/img/one.png"></div>
</body></html>
"""


class ExistingArticleTests(unittest.TestCase):
    def make_conn(self):
        conn = sqlite3.connect(":memory:")
        conn.executescript(
            "CREATE TABLE contents(id INTEGER PRIMARY KEY, title TEXT, slug TEXT);"
            "CREATE TABLE media(id INTEGER PRIMARY KEY, storage_key TEXT, original_name TEXT, "
            "mime TEXT, size INTEGER, width INTEGER, height INTEGER, uploader_id INTEGER, "
            "created_at INTEGER);"
        )
        return conn

    def test_parse_article_skips_existing_title_before_downloading_images(self):
        conn = self.make_conn()
        conn.execute("INSERT INTO contents VALUES(3, 'Existing Title', 'other')")
        page = Mock(text=ARTICLE_HTML)
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(import_devorz, "fetch", return_value=page),
            patch.object(import_devorz, "download_image") as download,
        ):
            with self.assertRaises(import_devorz.AlreadyImported) as caught:
                import_devorz.parse_article(
                    Mock(),
                    "https://source.example/archives/42.html",
                    Path(directory),
                    conn,
                    admin_id=1,
                    no_images=False,
                    dry_run=False,
                )
        self.assertEqual(caught.exception.content_id, 3)
        download.assert_not_called()
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM media").fetchone()[0], 0)
        conn.close()

    def test_main_skips_existing_slug_without_fetching(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            with open_db(db_path) as setup:
                setup.executescript(
                    "CREATE TABLE users(id INTEGER PRIMARY KEY);"
                    "CREATE TABLE contents(id INTEGER PRIMARY KEY, title TEXT, slug TEXT);"
                    "INSERT INTO users VALUES(1);"
                    "INSERT INTO contents VALUES(5, 'Old', '42');"
                )
            argv = [
                "import_devorz.py",
                "--dry-run",
                "--db",
                str(db_path),
                "--uploads",
                str(Path(directory) / "uploads"),
            ]
            output = io.StringIO()
            with (
                patch.object(sys, "argv", argv),
                patch.object(
                    import_devorz,
                    "discover_articles",
                    return_value=["https://source.example/archives/42.html"],
                ),
                patch.object(import_devorz, "parse_article") as parse,
                redirect_stdout(output),
            ):
                self.assertEqual(import_devorz.main(), 0)
            parse.assert_not_called()
            self.assertIn("skipped=1", output.getvalue())


if __name__ == "__main__":
    unittest.main()
