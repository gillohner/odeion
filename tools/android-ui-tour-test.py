#!/usr/bin/env python3
"""Finding Play to on real Zones screenshots of the CI emulators. Needs tesseract (the
phone-as-zone job installs it), no device.

zones-api30.png and zones-api35.png are ui-0-zones.png of main's phone-as-zone runs: API 30's
WebView wraps the card's Play to under the transport row, API 35's keeps it on the volume line.
zones-api35-selected.png is API 35's with shorter phone cards and five zones: the phone's card (the
first) says selected and the next card's Play to is in view; that one is not the phone's.
"""
import importlib.util
import subprocess
import unittest
from unittest import mock
from pathlib import Path

spec = importlib.util.spec_from_file_location("ui_tour", Path(__file__).with_name("android-ui-tour.py"))
tour = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tour)

FIX = Path(__file__).resolve().parent / "fixtures/android-ui-tour"


class PlayToSpotTests(unittest.TestCase):
    def check(self, name, box):
        spot = tour.play_to_spot(FIX / name)
        self.assertIsNotNone(spot, f"Play to not found on {name}")
        left, top, right, bottom = box
        self.assertTrue(left <= spot[0] <= right and top <= spot[1] <= bottom,
                        f"{name}: {spot} is outside the Play to button {box}")

    def test_api30_wrapped_card(self):
        # the fixed spot the tour tapped before, (206, 356), misses this button
        self.check("zones-api30.png", (34, 308, 95, 346))

    def test_api35_volume_line(self):
        self.check("zones-api35.png", (180, 343, 235, 371))

    def test_api35_first_card_selected(self):
        pick = tour.zone_pick(FIX / "zones-api35-selected.png")
        self.assertIsNotNone(pick, "neither Play to nor selected read on zones-api35-selected.png")
        kind, (x, y) = pick
        self.assertEqual(kind, "selected", f"the first card is the selected one, not {kind} at {(x, y)}")
        self.assertTrue(220 <= x <= 292 and 244 <= y <= 266, f"{(x, y)} is outside the selected pill")
        # (the Null output card's Play to, at about (264, 475), is not the phone's)
        self.assertIsNone(tour.play_to_spot(FIX / "zones-api35-selected.png"))

    def test_api35_volume_line_is_play_to(self):
        pick = tour.zone_pick(FIX / "zones-api35.png")
        self.assertIsNotNone(pick, "nothing read on zones-api35.png")
        self.assertEqual(pick[0], "play to")

    def test_next_card_play_to_is_not_the_first_cards(self):
        # When OCR misses the first card's "selected", the next card's Play to (below the second
        # state pill) is still not taken; nor is "Nothing playing" a card's top
        with self.ocr_edited(lambda ln: None if ln.rstrip().upper().endswith("SELECTED") else ln):
            self.assertIsNone(tour.zone_pick(FIX / "zones-api35-selected.png"))

    def ocr_edited(self, edit):
        """subprocess.run with tesseract's TSV lines changed by edit (a line in, a line or None out)."""
        real = subprocess.run

        def run(*a, **k):
            r = real(*a, **k)
            keep = [e for e in (edit(ln) for ln in r.stdout.splitlines()) if e is not None]
            return subprocess.CompletedProcess(r.args, r.returncode, "\n".join(keep) + "\n", r.stderr)

        return mock.patch.object(tour.subprocess, "run", run)

    def test_second_card_pill_missed_too(self):
        # OCR misses the first card's "selected" and the second card's state pill: where the first
        # card ends is not known, so the next card's Play to (about (264, 475)) is not taken
        def drop(ln):
            word = ln.rstrip().rsplit("\t", 1)[-1].upper()
            return None if word in ("SELECTED", "STOPPED") else ln

        with self.ocr_edited(drop):
            self.assertIsNone(tour.zone_pick(FIX / "zones-api35-selected.png"))
            self.assertIsNone(tour.play_to_spot(FIX / "zones-api35-selected.png"))

    def test_selected_works_is_not_the_pill(self):
        # A word "Selected" (a song called Selected Works) on the first card is not its pill
        def title_case(ln):
            return ln[:-len("SELECTED")] + "Selected" if ln.rstrip().endswith("SELECTED") else ln

        with self.ocr_edited(title_case):
            self.assertIsNone(tour.zone_pick(FIX / "zones-api35-selected.png"))

    def test_no_play_to(self):
        self.assertIsNone(tour.play_to_spot(FIX / "missing.png"))


if __name__ == "__main__":
    unittest.main()
