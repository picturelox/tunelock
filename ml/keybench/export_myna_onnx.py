#!/usr/bin/env python3
"""Export a pinned Myna backbone + trained key head as one verified ONNX graph.

The graph accepts the pinned model's mel-spectrogram chunks and emits 24 raw
key logits per chunk. Track aggregation stays explicit in the artifact
manifest: mean chunk logits, then softmax. This script does not parse key names;
the ordered labels come from the Rust-generated corpus manifest.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
from typing import Any

import numpy as np
import nnAudio
import onnx
import onnxruntime
import torch
import torchaudio
from torch import nn
from transformers import AutoModel

from train_myna_head import KeyHead


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Export and verify a Myna key ONNX artifact")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--checkpoint", required=True, type=Path)
    parser.add_argument("--hf-cache", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--state-index", type=int, default=0)
    parser.add_argument("--opset", type=int, default=17)
    parser.add_argument("--parity-atol", type=float, default=2e-4)
    parser.add_argument("--parity-rtol", type=float, default=2e-4)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


class ChunkKeyModel(nn.Module):
    def __init__(
        self,
        backbone: nn.Module,
        head: nn.Module,
        embedding_slice: tuple[int, int],
    ) -> None:
        super().__init__()
        self.backbone = backbone
        self.head = head
        self.embedding_start, self.embedding_end = embedding_slice

    def forward(self, spectrogram: torch.Tensor) -> torch.Tensor:
        embedding = self.backbone(spectrogram)
        return self.head(embedding[:, self.embedding_start : self.embedding_end])


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> int:
    args = parse_args()
    if nnAudio.__version__ != "0.3.3":
        raise ValueError(f"Expected pinned nnAudio 0.3.3, found {nnAudio.__version__}")
    torchaudio_version = torchaudio.__version__.split("+", maxsplit=1)[0]
    if torchaudio_version != "2.7.1":
        raise ValueError(
            f"Expected pinned torchaudio 2.7.1, found {torchaudio.__version__}"
        )
    if args.output_dir.exists():
        raise FileExistsError(f"Refusing to overwrite artifact directory: {args.output_dir}")
    if args.opset < 17:
        raise ValueError("The production artifact contract requires ONNX opset 17 or newer")

    manifest = load_json(args.manifest)
    labels = manifest.get("canonical_labels", [])
    if manifest.get("schema_version") != 1 or len(labels) != 24:
        raise ValueError("Expected a schema-1 Rust key manifest with 24 labels")
    checkpoint = torch.load(args.checkpoint, map_location="cpu", weights_only=False)
    if checkpoint.get("manifest_sha256") != sha256(args.manifest):
        raise ValueError("Checkpoint was trained from a different Rust manifest")

    base = checkpoint.get("base_model", {})
    model_name = str(base.get("model", ""))
    model_revision = str(base.get("model_revision", ""))
    model_license = str(base.get("model_license", "unknown"))
    embedding_dim = int(checkpoint.get("embedding_dim", base.get("embedding_dim", 384)))
    source_embedding_dim = int(checkpoint.get("source_embedding_dim", embedding_dim))
    raw_slice = checkpoint.get("embedding_slice", [0, embedding_dim])
    if not isinstance(raw_slice, (list, tuple)) or len(raw_slice) != 2:
        raise ValueError("Checkpoint has an invalid embedding slice")
    embedding_slice = (int(raw_slice[0]), int(raw_slice[1]))
    if (
        not 0 <= embedding_slice[0] < embedding_slice[1] <= source_embedding_dim
        or embedding_slice[1] - embedding_slice[0] != embedding_dim
    ):
        raise ValueError("Checkpoint embedding dimensions are inconsistent")
    n_samples = int(base.get("n_samples", 100_000))
    states = checkpoint.get("state_dicts", [])
    if not model_name or not model_revision:
        raise ValueError("Checkpoint does not pin its backbone and revision")
    if not 0 <= args.state_index < len(states):
        raise ValueError(
            f"--state-index {args.state_index} is outside {len(states)} final state(s)"
        )

    backbone = AutoModel.from_pretrained(
        model_name,
        revision=model_revision,
        trust_remote_code=True,
        cache_dir=args.hf_cache,
        local_files_only=True,
    ).cpu().eval()
    backbone.config.n_samples = n_samples
    backbone.config.n_frames = backbone.config._get_n_frames(n_samples)
    n_mels = int(backbone.config.spec_size[0])
    n_frames = int(backbone.config.n_frames)

    head = KeyHead(
        [int(value) for value in checkpoint["hidden_dims"]],
        float(checkpoint["dropout"]),
        embedding_dim,
    ).cpu().eval()
    head.load_state_dict(states[args.state_index])
    model = ChunkKeyModel(backbone, head, embedding_slice).cpu().eval()

    generator = torch.Generator().manual_seed(20260823)
    example = torch.rand((2, 1, n_mels, n_frames), generator=generator)
    with torch.inference_mode():
        reference = model(example).numpy()
    if reference.shape != (2, 24) or not np.isfinite(reference).all():
        raise ValueError(f"Unexpected PyTorch output shape/value: {reference.shape}")

    temporary = args.output_dir.with_name(f"{args.output_dir.name}.part.{os.getpid()}")
    if temporary.exists():
        raise FileExistsError(f"Refusing to overwrite stale temporary directory: {temporary}")
    temporary.mkdir(parents=True)
    onnx_path = temporary / "key-model.onnx"
    try:
        torch.onnx.export(
            model,
            (example,),
            onnx_path,
            input_names=["mel_spectrogram"],
            output_names=["chunk_logits"],
            dynamic_axes={
                "mel_spectrogram": {0: "chunk_count"},
                "chunk_logits": {0: "chunk_count"},
            },
            opset_version=args.opset,
            do_constant_folding=True,
            dynamo=False,
        )
        graph = onnx.load(onnx_path, load_external_data=False)
        onnx.checker.check_model(graph)

        session = onnxruntime.InferenceSession(
            str(onnx_path), providers=["CPUExecutionProvider"]
        )
        actual = session.run(
            ["chunk_logits"], {"mel_spectrogram": example.numpy()}
        )[0]
        maximum_absolute_error = float(np.max(np.abs(reference - actual)))
        parity = bool(
            np.allclose(
                reference,
                actual,
                atol=args.parity_atol,
                rtol=args.parity_rtol,
            )
        )
        if not parity or not np.array_equal(reference.argmax(axis=1), actual.argmax(axis=1)):
            raise RuntimeError(
                f"ONNX parity failed: max_abs={maximum_absolute_error:.6g}, "
                f"torch={reference.argmax(axis=1)}, ort={actual.argmax(axis=1)}"
            )

        artifact = {
            "schema_version": 3,
            "artifact_kind": "tunelock-neural-key-chunk-v3",
            "status": "research candidate; not production-enabled",
            "model_file": onnx_path.name,
            "model_sha256": sha256(onnx_path),
            "model_bytes": onnx_path.stat().st_size,
            "onnx_opset": args.opset,
            "input": {
                "name": "mel_spectrogram",
                "dtype": "float32",
                "shape": ["chunk_count", 1, n_mels, n_frames],
                "sample_rate_hz": int(backbone.config.sr),
                "audio_samples_per_chunk": n_samples,
                "preprocessor": (
                    "native TuneLock parity implementation of pinned nnAudio MelSpectrogram"
                ),
                "preprocessing": {
                    "implementation": "nnAudio MelSpectrogram",
                    "version": "0.3.3",
                    "n_fft": 2048,
                    "hop_length": 512,
                    "n_mels": n_mels,
                    "window": "periodic Hann",
                    "center": True,
                    "pad_mode": "reflect",
                    "power": 2.0,
                    "mel_scale": "Slaney",
                    "normalization": "area",
                },
                "audio_preprocessing": {
                    "reference_implementation": "torchaudio.load + transforms.Resample",
                    "reference_version": torchaudio_version,
                    "production_implementation": "Symphonia 0.5 + native sinc resampler",
                    "channel_reduction": "arithmetic mean across channels in float32",
                    "amplitude_handling": "preserve decoded amplitude; no normalization",
                    "resampling_method": "sinc_interp_hann",
                    "lowpass_filter_width": 6,
                    "rolloff": 0.99,
                },
            },
            "output": {
                "name": "chunk_logits",
                "dtype": "float32",
                "shape": ["chunk_count", 24],
                "posterior_labels": labels,
                "track_aggregation": "arithmetic mean of chunk logits, then softmax",
            },
            "backbone": {
                "model": model_name,
                "revision": model_revision,
                "license": model_license,
                "embedding_dim": source_embedding_dim,
            },
            "head": {
                "checkpoint_sha256": sha256(args.checkpoint),
                "state_index": args.state_index,
                "hidden_dims": checkpoint["hidden_dims"],
                "dropout_at_training": checkpoint["dropout"],
                "embedding_view": checkpoint.get("embedding_view", "full"),
                "embedding_slice": list(embedding_slice),
                "input_dim": embedding_dim,
            },
            "data_rights_status": "research-only pending commercial training-data review",
            "parity": {
                "provider": "CPUExecutionProvider",
                "test_batch": 2,
                "max_absolute_error": maximum_absolute_error,
                "absolute_tolerance": args.parity_atol,
                "relative_tolerance": args.parity_rtol,
                "argmax_equal": True,
            },
            "source": {
                "rust_manifest_sha256": sha256(args.manifest),
                "export_script_sha256": sha256(Path(__file__)),
                "torch": torch.__version__,
                "onnx": onnx.__version__,
                "onnxruntime": onnxruntime.__version__,
            },
        }
        (temporary / "artifact.json").write_text(
            json.dumps(artifact, indent=2) + "\n", encoding="utf-8"
        )
        args.output_dir.parent.mkdir(parents=True, exist_ok=True)
        os.replace(temporary, args.output_dir)
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise

    print(
        f"artifact={args.output_dir} bytes={artifact['model_bytes']} "
        f"max_abs={artifact['parity']['max_absolute_error']:.6g} parity=ok"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
