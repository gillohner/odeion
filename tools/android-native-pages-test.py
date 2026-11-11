#!/usr/bin/env python3
"""Deterministic controls for native page evidence; no device or build required."""
import importlib.util
import contextlib
import sqlite3
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("native_pages", Path(__file__).with_name("android-native-pages.py"))
pages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pages)


class NativePagesTests(unittest.TestCase):
    def test_albums_tap_waits_for_rendered_library_heading(self):
        class AlbumsTapped(Exception):
            pass
        for rendered_at in (5, None):
            with self.subTest(rendered_at=rendered_at):
                root = Path(tempfile.mkdtemp(prefix="phone328-render-"))
                state = {"time": 0, "taps": 0}
                def words_at(top):
                    return [dict(text=text, left=30 + n * 70, top=top, width=60,
                                 height=30, line=(1, 1, 1)) for n, text in enumerate(("Your", "library"))]
                def words(_):
                    # A chip/footer label is not evidence that the page body is ready.
                    return words_at(370 if rendered_at is not None and state["time"] >= rendered_at else 200)
                def shot(name):
                    (root / f"{name}.png").write_bytes(b"\x89PNG\r\n\x1a\n" + bytes(8) + struct.pack(">II", 720, 1600))
                def until(what, fn, timeout, every):
                    end = state["time"] + timeout
                    while state["time"] < end:
                        result = fn()
                        if result:
                            return result
                        state["time"] += every
                    raise RuntimeError(f"timeout {what}")
                def adb(*_):
                    state["taps"] += 1
                    self.assertIsNotNone(rendered_at, "must not tap an unrendered page")
                    self.assertGreaterEqual(state["time"], rendered_at, "Albums tap raced Library rendering")
                    raise AlbumsTapped()
                responses = [{}, {"title": "Native Fixture", "songs": [1, 2, 3]},
                             {"albums": [{"songs": [{"track": {"id": 2}, "options": [
                                 {"title": "Studio Candidate"}, {"title": "Radio Candidate"}]}]}]}]
                h = dict(adb=adb, api=Mock(side_effect=responses), shot=shot, until=until, log=Mock(), out=root)
                with patch.object(pages, "seed_fixture", return_value={"album_id": 1, "track_ids": [2]}), \
                        patch.object(pages, "read_screen", side_effect=words):
                    if rendered_at is None:
                        with self.assertRaisesRegex(RuntimeError, "timeout.*Library"):
                            pages.capture_pages(h)
                        self.assertEqual(state["taps"], 0)
                    else:
                        with self.assertRaises(AlbumsTapped):
                            pages.capture_pages(h)
                        self.assertEqual(state["taps"], 1)

    def test_navigation_is_established_after_density_recreates_activity(self):
        state = {"page": "Home"}
        events = []
        @contextlib.contextmanager
        def display(_):
            state["page"] = "Home"  # Android recreates the WebView on density changes
            events.append("display")
            try:
                yield
            finally:
                events.append("restored")
        def adb(*args):
            if args[:3] == ("shell", "input", "tap"):
                state["page"] = "Library"
            if "force-stop" in args:
                state["page"] = "Home"
                events.append("restart")
            return ""
        def tap(*_):
            state["page"] = "Library"
            events.append("fresh Library")
        def capture(_):
            self.assertEqual(state["page"], "Library", "resize must not lose the page before capture")
            events.append("capture")
        h = {"adb": adb, "tap": tap, "package": "fixture.app"}
        with patch.object(pages, "capture_display", display), patch.object(pages, "capture_pages", capture):
            pages.run(h)
        self.assertEqual(events, ["display", "restart", "fresh Library", "capture", "restored"])

    def test_fresh_navigation_failure_is_fatal_and_restores_display(self):
        adb = Mock(return_value="")
        h = {"adb": adb, "tap": Mock(side_effect=RuntimeError("Library missing")), "package": "fixture.app"}
        with patch.object(pages.time, "sleep"), patch.object(pages, "capture_pages") as capture:
            with self.assertRaisesRegex(RuntimeError, "Library missing"):
                pages.run(h)
            capture.assert_not_called()
        self.assertEqual(adb.call_args_list[-2].args, ("shell", "wm", "density", "reset"))
        self.assertEqual(adb.call_args_list[-1].args, ("shell", "wm", "size", "reset"))

    def test_display_is_restored_when_page_acceptance_fails(self):
        adb = Mock(side_effect=["Physical size: 320x640\nOverride size: 412x915", "Physical density: 160\nOverride density: 200", "", "", "", ""])
        with patch.object(pages.time, "sleep"), self.assertRaisesRegex(AssertionError, "missing page"):
            with pages.capture_display(adb):
                raise AssertionError("missing page")
        self.assertEqual(adb.call_args_list[-2].args, ("shell", "wm", "density", "200"))
        self.assertEqual(adb.call_args_list[-1].args, ("shell", "wm", "size", "412x915"))

    def test_partial_display_setup_restores_only_changed_size(self):
        adb = Mock(side_effect=["Physical size: 320x640", "Physical density: 160", "", RuntimeError("density refused"), ""])
        with self.assertRaisesRegex(RuntimeError, "density refused"):
            with pages.capture_display(adb):
                self.fail("must not capture after setup failure")
        self.assertEqual(adb.call_count, 5)
        self.assertEqual(adb.call_args_list[-1].args, ("shell", "wm", "size", "reset"))

    def test_empty_or_wrong_page_is_not_album_evidence(self):
        for text in ("", "Native Fixture Albums", "Native Fixture Morning Light"):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                pages.require_text(text, pages.ALBUM_REQUIRED)
        pages.require_text("Native Fixture Play Morning Light Evening Glow", pages.ALBUM_REQUIRED)

    def test_header_or_empty_state_is_not_populated_matches_evidence(self):
        for text in ("Confirm matches Nothing to confirm", "Confirm matches Morning Light",
                     "Native Fixture Morning Light Studio Candidate"):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                pages.require_text(text, pages.MATCHES_REQUIRED)
        pages.require_text("Morning Light Studio Candidate Radio Candidate", pages.MATCHES_REQUIRED)

    def test_phrase_target_needs_adjacent_words_on_the_same_line(self):
        words = [dict(text="Native", left=10, top=20, width=40, height=16, line=(1, 1, 1)),
                 dict(text="Fixture", left=55, top=20, width=45, height=16, line=(1, 1, 1))]
        self.assertEqual(pages.phrase_center(words, "Native Fixture"), (55, 28))
        words[1]["line"] = (1, 1, 2)
        self.assertIsNone(pages.phrase_center(words, "Native Fixture"))

    def test_fixture_uses_real_schema_and_preserves_original_tone(self):
        # Keep this small test's files as evidence; the task forbids cleanup.
        root = Path(tempfile.mkdtemp(prefix="phone328-seed-"))
        (root / "data").mkdir()
        (root / "music").mkdir()
        tone = root / "music" / "tone.wav"
        tone.write_bytes(b"synthetic fixture tone")
        db = root / "data" / "library.db"
        with sqlite3.connect(db) as c:
            for migration in sorted((Path(__file__).resolve().parents[1] / "crates/pm-library/src/migrations").glob("*.sql")):
                c.executescript(migration.read_text())
            c.execute("INSERT INTO tracks(id,title,duration_ms) VALUES(1,'tone',120000)")
            c.execute("INSERT INTO files(track_id,path,size_bytes,mtime_unix,container,sample_kind,rate_hz,bits,channels,duration_ms,missing) VALUES(1,?,22,1,'wav','int',44100,16,2,120000,0)", (str(tone),))
        fixture = pages.seed_fixture(root)
        with sqlite3.connect(db) as c:
            self.assertEqual(c.execute("SELECT title,album_id FROM tracks WHERE id=1").fetchone(), ("tone", None))
            self.assertEqual(c.execute("SELECT count(*) FROM tracks WHERE album_id=?", (fixture["album_id"],)).fetchone()[0], 3)
            self.assertEqual(c.execute("SELECT count(*) FROM identify_suggestions WHERE track_id=?", (fixture["track_ids"][0],)).fetchone()[0], 2)
            self.assertEqual(c.execute("SELECT count(*) FROM tracks WHERE canonical_uri IS NOT NULL").fetchone()[0], 0)
            self.assertEqual(c.execute("PRAGMA foreign_key_check").fetchall(), [])
            for (path,) in c.execute("SELECT path FROM files"):
                self.assertEqual(Path(path).read_bytes(), tone.read_bytes())
        with self.assertRaisesRegex(RuntimeError, "fresh.*fixture"):
            pages.seed_fixture(root)

    def test_non_fixture_database_is_refused_without_changes(self):
        root = Path(tempfile.mkdtemp(prefix="phone328-refuse-"))
        (root / "data").mkdir()
        with sqlite3.connect(root / "data" / "library.db") as c:
            c.execute("CREATE TABLE tracks(id INTEGER)")
            c.execute("INSERT INTO tracks VALUES(99)")
            c.execute("CREATE TABLE files(track_id INTEGER,path TEXT)")
            c.execute("INSERT INTO files VALUES(99,'/real/music/song.flac')")
        with self.assertRaisesRegex(RuntimeError, "fresh.*fixture"):
            pages.seed_fixture(root)
        with sqlite3.connect(root / "data" / "library.db") as c:
            self.assertEqual(c.execute("SELECT * FROM tracks").fetchall(), [(99,)])


if __name__ == "__main__":
    unittest.main(verbosity=2)
