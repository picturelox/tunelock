#!/usr/bin/env python3
"""Unit tests for the three-model selector's invariant feature contract."""

from __future__ import annotations

import unittest
from pathlib import Path

import numpy as np

from train_three_model_selector import (
    KEY_COUNT,
    build_features,
    candidate_features,
    normalized,
    split_spec,
)


class ThreeModelSelectorTests(unittest.TestCase):
    def test_requires_named_source(self) -> None:
        self.assertEqual(
            split_spec("model=folder/file.jsonl"),
            ("model", Path("folder/file.jsonl")),
        )
        for invalid in ("file.jsonl", "=file.jsonl", "model="):
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                split_spec(invalid)

    def test_rejects_invalid_posterior(self) -> None:
        with self.assertRaises(ValueError):
            normalized(np.ones(KEY_COUNT - 1))
        with self.assertRaises(ValueError):
            normalized(np.zeros(KEY_COUNT))

    def test_candidate_features_are_transposition_invariant(self) -> None:
        rng = np.random.default_rng(42)
        posteriors = rng.random((3, KEY_COUNT))
        posteriors /= posteriors.sum(axis=1, keepdims=True)
        candidate = 7
        shift = 5
        shifted = np.roll(posteriors, shift, axis=1)
        np.testing.assert_allclose(
            candidate_features(posteriors, candidate),
            candidate_features(shifted, (candidate + shift) % KEY_COUNT),
            atol=1e-12,
        )

    def test_diagnostic_features_rotate_with_candidate(self) -> None:
        rng = np.random.default_rng(123)
        posteriors = rng.random((3, KEY_COUNT))
        posteriors /= posteriors.sum(axis=1, keepdims=True)
        candidate = 11
        shift = 4
        candidate_fields = {
            field: rng.random(KEY_COUNT).tolist()
            for field in (
                "candidate_std",
                "candidate_min",
                "candidate_max",
                "candidate_top1_rate",
            )
        }
        diagnostics = [
            {},
            {
                "tta": {
                    **candidate_fields,
                    "entropy_mean": 1.0,
                    "entropy_std": 0.2,
                    "js_to_mean_mean": 0.1,
                    "js_to_mean_max": 0.3,
                }
            },
            {},
        ]
        shifted_diagnostics = [
            {},
            {
                "tta": {
                    **{
                        field: np.roll(values, shift).tolist()
                        for field, values in candidate_fields.items()
                    },
                    "entropy_mean": 1.0,
                    "entropy_std": 0.2,
                    "js_to_mean_mean": 0.1,
                    "js_to_mean_max": 0.3,
                }
            },
            {},
        ]
        np.testing.assert_allclose(
            candidate_features(posteriors, candidate, diagnostics),
            candidate_features(
                np.roll(posteriors, shift, axis=1),
                (candidate + shift) % KEY_COUNT,
                shifted_diagnostics,
            ),
            atol=1e-12,
        )

    def test_three_models_produce_one_shared_row_per_candidate(self) -> None:
        rng = np.random.default_rng(7)
        posteriors = rng.random((2, 3, KEY_COUNT))
        posteriors /= posteriors.sum(axis=2, keepdims=True)
        features = build_features(posteriors)
        self.assertEqual(features.shape[0], 2 * KEY_COUNT)
        self.assertGreater(features.shape[1], 0)
        self.assertTrue(np.isfinite(features).all())


if __name__ == "__main__":
    unittest.main()
