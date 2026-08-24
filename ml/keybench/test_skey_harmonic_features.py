import unittest

import numpy as np

from extract_skey_harmonic_features import atomic_feature, valid_feature


class SKeyHarmonicFeatureTests(unittest.TestCase):
    def test_cache_contract_round_trip(self) -> None:
        from tempfile import TemporaryDirectory
        from pathlib import Path

        with TemporaryDirectory() as directory:
            path = Path(directory) / "feature.npz"
            atomic_feature(
                path,
                np.arange(36, dtype=np.float32).reshape(3, 12),
                np.full(24, 1.0 / 24.0, dtype=np.float32),
            )
            self.assertTrue(valid_feature(path))

    def test_rejects_wrong_shape_or_probability_mass(self) -> None:
        from tempfile import TemporaryDirectory
        from pathlib import Path

        with TemporaryDirectory() as directory:
            path = Path(directory) / "feature.npz"
            np.savez(path, feature=np.zeros((2, 12)), posterior=np.ones(24))
            self.assertFalse(valid_feature(path))


if __name__ == "__main__":
    unittest.main()
