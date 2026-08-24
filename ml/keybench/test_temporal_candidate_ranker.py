import unittest

import torch

from train_temporal_candidate_ranker import feature_names, record_candidate_features


class TemporalCandidateRankerTests(unittest.TestCase):
    def test_feature_contract_is_finite_and_candidate_shaped(self) -> None:
        logits = torch.randn((19, 24), generator=torch.Generator().manual_seed(7))
        features = record_candidate_features(logits)
        self.assertEqual(tuple(features.shape), (24, len(feature_names())))
        self.assertTrue(torch.isfinite(features).all())

    def test_features_are_equivariant_to_candidate_rotation(self) -> None:
        logits = torch.randn((19, 24), generator=torch.Generator().manual_seed(8))
        shift = 5
        original = record_candidate_features(logits)
        rotated = record_candidate_features(torch.roll(logits, shifts=shift, dims=1))
        self.assertTrue(
            torch.allclose(
                torch.roll(original, shifts=shift, dims=0), rotated, atol=1e-5, rtol=1e-5
            )
        )

    def test_persistence_distinguishes_stable_from_isolated_support(self) -> None:
        logits = torch.zeros((19, 24))
        logits[:10, 2] = 5.0
        logits[10, 9] = 8.0
        features = record_candidate_features(logits)
        persistence = feature_names().index("top1_longest_run")
        self.assertGreater(float(features[2, persistence]), float(features[9, persistence]))


if __name__ == "__main__":
    unittest.main()
