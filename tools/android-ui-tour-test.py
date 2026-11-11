#!/usr/bin/env python3
"""Finding Play to on real Zones screenshots of the CI emulators. Needs tesseract (the
phone-as-zone job installs it), no device.

zones-api30.png and zones-api35.png are ui-0-zones.png of main's phone-as-zone runs: API 30's
WebView wraps the card's Play to under the transport row, API 35's keeps it on the volume line.
"""
import importlib.util
import unittest
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

    def test_no_play_to(self):
        self.assertIsNone(tour.play_to_spot(FIX / "missing.png"))


if __name__ == "__main__":
    unittest.main()
