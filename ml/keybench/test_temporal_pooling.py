import unittest

import torch

from evaluate_myna_temporal_pooling import canonical_candidates, pool_record_logits


class TemporalPoolingTests(unittest.TestCase):
    def test_every_candidate_emits_a_finite_posterior(self) -> None:
        generator = torch.Generator().manual_seed(42)
        logits = torch.randn((19, 24), generator=generator)
        for candidate in canonical_candidates():
            with self.subTest(candidate=candidate["id"]):
                posterior = pool_record_logits(logits, candidate)
                self.assertEqual(tuple(posterior.shape), (24,))
                self.assertTrue(torch.isfinite(posterior).all())
                self.assertAlmostEqual(float(posterior.sum()), 1.0, places=5)

    def test_consistent_sections_keep_the_same_winner(self) -> None:
        logits = torch.zeros((19, 24))
        logits[:, 7] = 4.0
        for candidate in canonical_candidates():
            with self.subTest(candidate=candidate["id"]):
                self.assertEqual(int(pool_record_logits(logits, candidate).argmax()), 7)

    def test_central_crop_rejects_edge_only_outliers(self) -> None:
        logits = torch.zeros((19, 24))
        logits[:, 3] = 3.0
        logits[0, 11] = 100.0
        logits[-1, 11] = 100.0
        config = {"id": "central-minus-1", "kind": "central_logits", "edge_chunks": 1}
        self.assertEqual(int(pool_record_logits(logits, config).argmax()), 3)


if __name__ == "__main__":
    unittest.main()
