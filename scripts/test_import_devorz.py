import io
import sqlite3
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime
from pathlib import Path
from unittest.mock import Mock, patch

from bs4 import BeautifulSoup

import import_devorz


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
            with sqlite3.connect(db_path) as setup:
                setup.execute("CREATE TABLE settings(name TEXT PRIMARY KEY, value TEXT)")

            conn = import_devorz.connect_database(db_path, read_only=True)
            with self.assertRaises(sqlite3.OperationalError):
                conn.execute("INSERT INTO settings(name, value) VALUES('x', 'y')")
            conn.close()

    def test_import_rejects_a_different_mcp_instance_before_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            with sqlite3.connect(db_path) as setup:
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
            with sqlite3.connect(db_path) as setup:
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
            with sqlite3.connect(db_path) as setup:
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
                    with sqlite3.connect(db_path) as remote_db:
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
            with sqlite3.connect(db_path) as setup:
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
            with sqlite3.connect(f"file:{db_path}?mode=ro", uri=True) as check:
                self.assertEqual(
                    check.execute("SELECT COUNT(*) FROM users").fetchone()[0],
                    1,
                )

    def test_dry_run_main_performs_no_database_or_upload_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            db_path = Path(directory) / "typecho.db"
            uploads_dir = Path(directory) / "uploads"
            with sqlite3.connect(db_path) as setup:
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
            with sqlite3.connect(f"file:{db_path}?mode=ro", uri=True) as check:
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


if __name__ == "__main__":
    unittest.main()
