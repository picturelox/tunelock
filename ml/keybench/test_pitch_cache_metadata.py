#!/usr/bin/env python3
"""Unit tests for score-bearing pitch-cache provenance gates."""

from __future__ import annotations

import copy
import unittest

from train_myna_head import validate_pitch_cache_metadata


class PitchCacheMetadataTests(unittest.TestCase):
    def setUp(self) -> None:
        self.base = {
            "model": "example/model",
            "model_revision": "revision-1",
            "embedding_dim": 384,
        }
        self.pitch = {
            "adapter": "tunelock/myna-pitch-embedding-cache",
            "manifest_sha256": "manifest-hash",
            "model": "example/model",
            "model_revision": "revision-1",
            "embedding_dim": 384,
            "role": "training",
            "pitch_method": "phase-vocoder-sparse-v1",
            "unique_records": 12,
            "expected": 24,
            "complete": 24,
            "failed": [],
            "semitones": [-1, 1],
        }

    def validate(self, metadata: dict) -> set[int]:
        return validate_pitch_cache_metadata(
            metadata,
            manifest_hash="manifest-hash",
            base_metadata=self.base,
            required_role="training",
        )

    def test_accepts_complete_matching_cache(self) -> None:
        self.assertEqual(self.validate(self.pitch), {-1, 1})

    def test_rejects_incomplete_cache(self) -> None:
        metadata = copy.deepcopy(self.pitch)
        metadata["complete"] = 23
        with self.assertRaisesRegex(ValueError, "incomplete"):
            self.validate(metadata)

    def test_rejects_failed_cache(self) -> None:
        metadata = copy.deepcopy(self.pitch)
        metadata["failed"] = [{"id": "track", "error": "decode"}]
        with self.assertRaisesRegex(ValueError, "incomplete"):
            self.validate(metadata)

    def test_rejects_inconsistent_expected_shape(self) -> None:
        metadata = copy.deepcopy(self.pitch)
        metadata["unique_records"] = 11
        with self.assertRaisesRegex(ValueError, "incomplete"):
            self.validate(metadata)

    def test_rejects_wrong_role_or_provenance(self) -> None:
        for field, value in (
            ("role", "development"),
            ("manifest_sha256", "other-manifest"),
            ("model_revision", "other-revision"),
            ("embedding_dim", 1536),
        ):
            with self.subTest(field=field):
                metadata = copy.deepcopy(self.pitch)
                metadata[field] = value
                with self.assertRaises(ValueError):
                    self.validate(metadata)

    def test_rejects_empty_or_zero_shift_set(self) -> None:
        for shifts in ([], [-1, 0, 1]):
            with self.subTest(shifts=shifts):
                metadata = copy.deepcopy(self.pitch)
                metadata["semitones"] = shifts
                with self.assertRaisesRegex(ValueError, "non-zero"):
                    self.validate(metadata)

    def test_rejects_missing_method(self) -> None:
        metadata = copy.deepcopy(self.pitch)
        metadata.pop("pitch_method")
        with self.assertRaisesRegex(ValueError, "augmentation method"):
            self.validate(metadata)


if __name__ == "__main__":
    unittest.main()
