#!/usr/bin/env python3
"""Unit tests for the Rekordbox opinion importer's vocabulary mirrors."""

import unittest

from import_rekordbox_opinions import (
    CANONICAL_LABELS,
    camelot_index,
    location_to_path,
    mirex_category,
    rekordbox_tonality,
)


class RekordboxTonalityTests(unittest.TestCase):
    def test_minor_suffix_and_bare_major(self) -> None:
        self.assertEqual(rekordbox_tonality("Am"), CANONICAL_LABELS.index("A minor"))
        self.assertEqual(rekordbox_tonality("A"), CANONICAL_LABELS.index("A major"))
        self.assertEqual(rekordbox_tonality("F#m"), CANONICAL_LABELS.index("F# minor"))

    def test_flats_map_to_sharp_canonical(self) -> None:
        self.assertEqual(rekordbox_tonality("Bbm"), CANONICAL_LABELS.index("A# minor"))
        self.assertEqual(rekordbox_tonality("Eb"), CANONICAL_LABELS.index("D# major"))
        self.assertEqual(rekordbox_tonality("Abm"), CANONICAL_LABELS.index("G# minor"))

    def test_rejects_empty_and_unknown(self) -> None:
        self.assertIsNone(rekordbox_tonality(""))
        self.assertIsNone(rekordbox_tonality("Hm"))
        self.assertIsNone(rekordbox_tonality("Cdim"))


class CamelotMirrorTests(unittest.TestCase):
    def test_matches_rust_corpus_semantics(self) -> None:
        self.assertEqual(camelot_index("7A"), CANONICAL_LABELS.index("D minor"))
        self.assertEqual(camelot_index("8A"), CANONICAL_LABELS.index("A minor"))
        self.assertEqual(camelot_index("8B"), CANONICAL_LABELS.index("C major"))
        self.assertEqual(camelot_index("1B"), CANONICAL_LABELS.index("B major"))

    def test_rejects_garbage(self) -> None:
        self.assertIsNone(camelot_index(""))
        self.assertIsNone(camelot_index("13A"))
        self.assertIsNone(camelot_index("0B"))
        self.assertIsNone(camelot_index("8C"))


class MirexCategoryTests(unittest.TestCase):
    def test_categories(self) -> None:
        c_major = CANONICAL_LABELS.index("C major")
        a_minor = CANONICAL_LABELS.index("A minor")
        g_major = CANONICAL_LABELS.index("G major")
        c_minor = CANONICAL_LABELS.index("C minor")
        b_major = CANONICAL_LABELS.index("B major")
        self.assertEqual(mirex_category(c_major, c_major), "correct")
        self.assertEqual(mirex_category(c_major, g_major), "fifth")
        self.assertEqual(mirex_category(c_major, a_minor), "relative")
        self.assertEqual(mirex_category(a_minor, c_major), "relative")
        self.assertEqual(mirex_category(c_major, c_minor), "parallel")
        self.assertEqual(mirex_category(c_major, b_major), "semitone")
        self.assertEqual(mirex_category(c_major, CANONICAL_LABELS.index("D major")), "other")


class LocationTests(unittest.TestCase):
    def test_file_uri_decoding(self) -> None:
        self.assertEqual(
            location_to_path("file://localhost/C:/Users/DJ/My%20Music/track%20one.mp3"),
            "C:\\Users\\DJ\\My Music\\track one.mp3",
        )


if __name__ == "__main__":
    unittest.main()
