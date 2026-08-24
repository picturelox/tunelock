#!/usr/bin/env python3
"""Unit tests for FMAK label and license normalization."""

import unittest

from export_fmak_manifest import CANONICAL_LABELS, canonical_index, license_class


class CanonicalIndexTests(unittest.TestCase):
    def test_all_24_canonical_labels_round_trip(self) -> None:
        for index, label in enumerate(CANONICAL_LABELS):
            tonic, mode = label.split(" ")
            self.assertEqual(canonical_index(f"{tonic} {mode.capitalize()}"), index)

    def test_flats_map_to_sharp_canonical(self) -> None:
        self.assertEqual(canonical_index("Bb Major"), CANONICAL_LABELS.index("A# major"))
        self.assertEqual(canonical_index("Eb minor"), CANONICAL_LABELS.index("D# minor"))
        self.assertEqual(canonical_index("Ab Major"), CANONICAL_LABELS.index("G# major"))
        self.assertEqual(canonical_index("Db minor"), CANONICAL_LABELS.index("C# minor"))

    def test_rejects_malformed_labels(self) -> None:
        self.assertIsNone(canonical_index("C"))
        self.assertIsNone(canonical_index("C dorian"))
        self.assertIsNone(canonical_index("H Major"))
        self.assertIsNone(canonical_index(""))


class LicenseClassTests(unittest.TestCase):
    def test_noncommercial_detected(self) -> None:
        self.assertEqual(
            license_class(
                "Attribution-NonCommercial-ShareAlike 4.0",
                "https://creativecommons.org/licenses/by-nc-sa/4.0/",
            ),
            "noncommercial",
        )
        self.assertEqual(
            license_class("Attribution-NonCommercial 3.0", ""), "noncommercial"
        )

    def test_commercial_friendly_classes(self) -> None:
        self.assertEqual(
            license_class("Attribution 4.0", "https://creativecommons.org/licenses/by/4.0/"),
            "cc_by",
        )
        self.assertEqual(
            license_class(
                "Attribution-ShareAlike 4.0",
                "https://creativecommons.org/licenses/by-sa/4.0/",
            ),
            "cc_by_sa",
        )
        self.assertEqual(license_class("Public Domain", ""), "public_domain")

    def test_unknown_when_unrecognized(self) -> None:
        self.assertEqual(license_class("All Rights Reserved", ""), "unknown")


if __name__ == "__main__":
    unittest.main()
