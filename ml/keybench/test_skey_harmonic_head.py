import unittest

import torch

from train_skey_harmonic_head import ARCHITECTURES, HarmonicHead, rotate_batch


class SKeyHarmonicHeadTests(unittest.TestCase):
    def test_every_head_is_pitch_equivariant_without_dropout(self) -> None:
        feature = torch.randn((2, 3, 12), generator=torch.Generator().manual_seed(4))
        for config in ARCHITECTURES:
            with self.subTest(architecture=config["id"]):
                model = HarmonicHead(config).eval()
                original = model(feature).reshape(2, 2, 12)
                rotated = model(torch.roll(feature, 5, dims=2)).reshape(2, 2, 12)
                self.assertTrue(
                    torch.allclose(torch.roll(original, 5, dims=2), rotated, atol=1e-6)
                )

    def test_augmentation_rotates_tonic_and_preserves_mode(self) -> None:
        features = torch.zeros((4, 3, 12))
        labels = torch.tensor([0, 11, 12, 23])
        rotated, targets = rotate_batch(
            features, labels, torch.Generator().manual_seed(10)
        )
        self.assertEqual(tuple(rotated.shape), tuple(features.shape))
        self.assertTrue(torch.equal(labels // 12, targets // 12))


if __name__ == "__main__":
    unittest.main()
