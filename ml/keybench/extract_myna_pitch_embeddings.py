#!/usr/bin/env python3
"""Cache Myna embeddings for Rust-defined pitch-shift training targets."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import time
from typing import Any, Callable

import numpy as np
import torch
import torchaudio
import torchaudio.functional as audio_functional
import torch.nn.functional as neural_functional
from transformers import AutoModel


DEFAULT_MODEL = "oriyonay/myna-vertical"
DEFAULT_REVISION = "6b9e1e5aae0832335d61d7a38764114e496824d4"
SAFE_ID = re.compile(r"^[A-Za-z0-9._-]+$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Cache pitch-shifted Myna embeddings")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--cache-dir", required=True, type=Path)
    parser.add_argument("--hf-cache", required=True, type=Path)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--revision", default=DEFAULT_REVISION)
    parser.add_argument("--n-samples", type=int, default=100_000)
    parser.add_argument(
        "--embedding-dim",
        type=int,
        default=384,
        help="Expected backbone output width (384 for Myna-Vertical, 1536 for Myna-85M).",
    )
    parser.add_argument(
        "--pitch-method",
        choices=(
            "phase-vocoder",
            "phase-vocoder-cached",
            "phase-vocoder-sparse-v1",
            "resample-speed",
            "linear-speed",
        ),
        default="phase-vocoder",
        help=(
            "Pitch-only phase vocoder (reference, dense cached, or sparse-v1 equivalent), "
            "or pitch+speed ablation."
        ),
    )
    parser.add_argument("--model-batch-size", type=int, default=32)
    parser.add_argument(
        "--role",
        choices=("training", "development"),
        default="training",
        help="Manifest role to transform; use a distinct cache directory per role.",
    )
    parser.add_argument(
        "--semitones",
        type=int,
        nargs="+",
        default=[-6, -5, -4, -3, -2, -1, 1, 2, 3, 4, 5, 6],
    )
    parser.add_argument("--device", default="cuda" if torch.cuda.is_available() else "cpu")
    parser.add_argument("--limit", type=int)
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_manifest(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    targets = data.get("pitch_shift_targets", [])
    if data.get("schema_version") != 1 or len(data.get("canonical_labels", [])) != 24:
        raise ValueError("Expected schema-1 Rust key corpus manifest")
    if {int(item["semitones"]) for item in targets} != set(range(-6, 7)):
        raise ValueError("Manifest must contain Rust targets for every shift in [-6, 6]")
    if any(len(item["target_by_source_index"]) != 24 for item in targets):
        raise ValueError("Every Rust pitch-shift target table must contain 24 entries")
    return data


def deduplicate_recordings(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen = set()
    result = []
    for record in records:
        fingerprint = str(record.get("recording_md5") or "")
        if fingerprint and fingerprint in seen:
            continue
        if fingerprint:
            seen.add(fingerprint)
        result.append(record)
    return result


def cache_path(root: Path, semitones: int, record: dict[str, Any]) -> Path:
    track_id = str(record["id"])
    corpus = str(record["corpus"])
    if not SAFE_ID.fullmatch(track_id) or not SAFE_ID.fullmatch(corpus):
        raise ValueError(f"Unsafe manifest identity: {corpus}/{track_id}")
    return root / f"shift_{semitones:+d}" / corpus / f"{track_id}.npy"


def valid_cached(path: Path, embedding_dim: int) -> bool:
    if not path.is_file():
        return False
    try:
        value = np.load(path, mmap_mode="r", allow_pickle=False)
        return value.ndim == 2 and value.shape[0] > 0 and value.shape[1] == embedding_dim
    except (OSError, ValueError):
        return False


def atomic_save(path: Path, embeddings: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.part.{os.getpid()}.npy")
    if temporary.exists():
        raise FileExistsError(f"Refusing to overwrite stale temporary file: {temporary}")
    np.save(temporary, embeddings, allow_pickle=False)
    os.replace(temporary, path)


class SparseSincResampler:
    """Sparse equivalent of torchaudio's default sinc/Hann resampler.

    Torchaudio materializes ``new_freq * (orig_freq + 2 * width)`` weights
    after GCD reduction. The pitch-only rates used here are mostly coprime, so
    that creates hundreds of millions of near-zero weights per shift. This
    implementation stores only the local low-pass support for each output
    phase while preserving torchaudio's rate reduction and float64 kernel
    construction rules.
    """

    def __init__(
        self,
        orig_freq: int,
        new_freq: int,
        device: torch.device | str,
        *,
        lowpass_filter_width: int = 6,
        rolloff: float = 0.99,
        chunk_size: int = 65_536,
    ) -> None:
        if orig_freq <= 0 or new_freq <= 0:
            raise ValueError("Resampling frequencies must be positive")
        if lowpass_filter_width <= 0:
            raise ValueError("Low-pass filter width must be positive")
        if not 0.0 < rolloff <= 1.0:
            raise ValueError("Rolloff must be in (0, 1]")
        if chunk_size < 1:
            raise ValueError("Chunk size must be positive")

        self.identity = orig_freq == new_freq
        divisor = math.gcd(orig_freq, new_freq)
        self.orig_freq = orig_freq // divisor
        self.new_freq = new_freq // divisor
        self.chunk_size = chunk_size
        if self.identity:
            self.offsets = torch.empty(0, dtype=torch.int64, device=device)
            self.weights = torch.empty(0, dtype=torch.float32, device=device)
            return

        base_freq = min(self.orig_freq, self.new_freq) * rolloff
        width = math.ceil(lowpass_filter_width * self.orig_freq / base_freq)
        support = lowpass_filter_width * self.orig_freq / base_freq
        radius = math.ceil(support) + 1

        # Construct in float64 on CPU, as torchaudio does when dtype is omitted,
        # and cast the final kernel weights to float32 before device transfer.
        phases = torch.arange(self.new_freq, dtype=torch.float64)
        centers = phases * self.orig_freq / self.new_freq
        center_floor = torch.floor(centers).to(torch.int64)
        relative = torch.arange(-radius, radius + 1, dtype=torch.int64)
        offsets = center_floor[:, None] + relative[None, :]
        valid_kernel = (offsets >= -width) & (offsets < width + self.orig_freq)

        # Preserve an otherwise non-obvious torchaudio detail: with its default
        # dtype, the phase arange/division is float32 while the sample offsets
        # are float64. This rounding is observable in the resulting weights.
        phase_term = (
            -torch.arange(self.new_freq, dtype=torch.float32) / self.new_freq
        )
        t = (
            phase_term.to(torch.float64)[:, None]
            + offsets.to(torch.float64) / self.orig_freq
        )
        t *= base_freq
        t.clamp_(-lowpass_filter_width, lowpass_filter_width)
        window = torch.cos(t * math.pi / lowpass_filter_width / 2.0) ** 2
        t *= math.pi
        sinc = torch.where(t == 0, torch.ones_like(t), t.sin() / t)
        weights = sinc * window * (base_freq / self.orig_freq)
        weights.masked_fill_(~valid_kernel, 0.0)

        self.offsets = offsets.to(device=device)
        self.weights = weights.to(device=device, dtype=torch.float32)

    def __call__(self, waveform: torch.Tensor) -> torch.Tensor:
        if not waveform.is_floating_point():
            raise TypeError(f"Expected floating point waveform, got {waveform.dtype}")
        if self.identity:
            return waveform
        if waveform.device != self.offsets.device:
            raise ValueError(
                f"Waveform is on {waveform.device}, resampler is on {self.offsets.device}"
            )
        shape = waveform.shape
        length = shape[-1]
        if length < 1:
            raise ValueError("Waveform must contain at least one sample")
        flattened = waveform.reshape(-1, length)
        target_length = (self.new_freq * length + self.orig_freq - 1) // self.orig_freq
        chunks = []

        for start in range(0, target_length, self.chunk_size):
            stop = min(start + self.chunk_size, target_length)
            output_indices = torch.arange(start, stop, device=waveform.device)
            blocks = torch.div(output_indices, self.new_freq, rounding_mode="floor")
            phases = torch.remainder(output_indices, self.new_freq)
            source_indices = blocks[:, None] * self.orig_freq + self.offsets[phases]
            valid_source = (source_indices >= 0) & (source_indices < length)
            gathered = flattened[:, source_indices.clamp(0, length - 1)]
            weights = self.weights[phases].to(dtype=waveform.dtype)
            weighted = gathered * weights.masked_fill(~valid_source, 0.0)
            chunks.append(weighted.sum(dim=-1))

        result = torch.cat(chunks, dim=-1)
        return result.reshape(shape[:-1] + (target_length,))


def cached_phase_vocoder_shift(
    spectrum: torch.Tensor,
    original_length: int,
    semitones: int,
    window: torch.Tensor,
    phase_advance: torch.Tensor,
    resampler: Callable[[torch.Tensor], torch.Tensor],
) -> torch.Tensor:
    """Equivalent pitch-only path with one shared STFT and cached sinc kernels."""
    n_fft = 512
    hop_length = n_fft // 4
    rate = 2.0 ** (-float(semitones) / 12.0)
    stretched_spectrum = audio_functional.phase_vocoder(spectrum, rate, phase_advance)
    stretched_length = int(round(original_length / rate))
    stretched = torch.istft(
        stretched_spectrum,
        n_fft=n_fft,
        hop_length=hop_length,
        win_length=n_fft,
        window=window,
        length=stretched_length,
    )
    shifted = resampler(stretched)
    if shifted.shape[-1] < original_length:
        shifted = neural_functional.pad(shifted, (0, original_length - shifted.shape[-1]))
    return shifted[..., :original_length]


def write_metadata(
    root: Path,
    args: argparse.Namespace,
    manifest_hash: str,
    records: list[dict[str, Any]],
    failures: list[dict[str, str]],
) -> None:
    expected = len(records) * len(args.semitones)
    complete = sum(
        valid_cached(cache_path(root, semitones, record), args.embedding_dim)
        for record in records
        for semitones in args.semitones
    )
    metadata = {
        "schema_version": 1,
        "adapter": "tunelock/myna-pitch-embedding-cache",
        "augmentation": {
            "phase-vocoder": "torchaudio.functional.pitch_shift",
            "phase-vocoder-cached": "torchaudio phase-vocoder pitch shift with shared STFT and cached sinc-resample kernels",
            "phase-vocoder-sparse-v1": "torchaudio-compatible phase-vocoder pitch shift with shared STFT and sparse sinc/Hann resampling (width=6, rolloff=0.99)",
            "resample-speed": "torchaudio.transforms.Resample interpreted at 16kHz (pitch+speed)",
            "linear-speed": "torch linear interpolation interpreted at 16kHz (pitch+speed ablation)",
        }[args.pitch_method],
        "pitch_method": args.pitch_method,
        "model": args.model,
        "model_revision": args.revision,
        "model_license": "MIT",
        "manifest_sha256": manifest_hash,
        "n_samples": args.n_samples,
        "embedding_dim": args.embedding_dim,
        "semitones": args.semitones,
        "role": args.role,
        "unique_records": len(records),
        "expected": expected,
        "complete": complete,
        "failed": failures,
    }
    target = root / "metadata.json"
    temporary = target.with_name(f"{target.name}.part.{os.getpid()}")
    temporary.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, target)


def validate_existing_metadata(
    root: Path,
    args: argparse.Namespace,
    manifest_hash: str,
) -> None:
    path = root / "metadata.json"
    if not path.exists():
        return
    metadata = json.loads(path.read_text(encoding="utf-8"))
    matches = (
        metadata.get("adapter") == "tunelock/myna-pitch-embedding-cache"
        and metadata.get("model") == args.model
        and metadata.get("model_revision") == args.revision
        and metadata.get("manifest_sha256") == manifest_hash
        and int(metadata.get("n_samples", -1)) == args.n_samples
        and int(metadata.get("embedding_dim", 384)) == args.embedding_dim
        and metadata.get("pitch_method") == args.pitch_method
        and metadata.get("role") == args.role
        and sorted(int(value) for value in metadata.get("semitones", []))
        == sorted(args.semitones)
    )
    if not matches:
        raise ValueError(
            f"Pitch cache metadata does not match this run: {path}; use a new cache directory"
        )


def main() -> int:
    args = parse_args()
    if args.embedding_dim < 1:
        raise ValueError("--embedding-dim must be positive")
    manifest = load_manifest(args.manifest)
    available_shifts = {int(item["semitones"]) for item in manifest["pitch_shift_targets"]}
    if 0 in args.semitones or any(value not in available_shifts for value in args.semitones):
        raise ValueError("Requested shifts must be non-zero members of [-6, 6]")
    if len(set(args.semitones)) != len(args.semitones):
        raise ValueError("Duplicate semitone arguments are not allowed")

    records = deduplicate_recordings(
        [record for record in manifest["records"] if record["role"] == args.role]
    )
    if args.limit is not None:
        records = records[: args.limit]
    args.cache_dir.mkdir(parents=True, exist_ok=True)
    args.hf_cache.mkdir(parents=True, exist_ok=True)
    manifest_hash = sha256(args.manifest)
    validate_existing_metadata(args.cache_dir, args, manifest_hash)
    pending_records = [
        record
        for record in records
        if any(
            not valid_cached(
                cache_path(args.cache_dir, semitones, record), args.embedding_dim
            )
            for semitones in args.semitones
        )
    ]
    print(
        f"records={len(records)} shifts={args.semitones} pending_records={len(pending_records)} "
        f"device={args.device}",
        flush=True,
    )
    if not pending_records:
        write_metadata(args.cache_dir, args, manifest_hash, records, [])
        return 0

    model = AutoModel.from_pretrained(
        args.model,
        revision=args.revision,
        trust_remote_code=True,
        cache_dir=args.hf_cache,
        local_files_only=True,
    ).to(args.device)
    model.eval()
    model.config.n_samples = args.n_samples
    model.config.n_frames = model.config._get_n_frames(args.n_samples)
    mel_spectrogram = model.preprocessor.mel_spec.to(args.device)
    resamplers: dict[int, torchaudio.transforms.Resample] = {}
    pitch_resamplers = (
        {
            semitones: torchaudio.transforms.Resample(
                16_000,
                round(16_000 / (2.0 ** (semitones / 12.0))),
            ).to(args.device)
            for semitones in args.semitones
        }
        if args.pitch_method == "resample-speed"
        else {}
    )
    pitch_only_resamplers = (
        {
            semitones: torchaudio.transforms.Resample(
                int(16_000 / (2.0 ** (-float(semitones) / 12.0))),
                16_000,
            ).to(args.device)
            for semitones in args.semitones
        }
        if args.pitch_method == "phase-vocoder-cached"
        else {}
    )
    sparse_pitch_only_resamplers = (
        {
            semitones: SparseSincResampler(
                int(16_000 / (2.0 ** (-float(semitones) / 12.0))),
                16_000,
                args.device,
            )
            for semitones in args.semitones
        }
        if args.pitch_method == "phase-vocoder-sparse-v1"
        else {}
    )
    phase_window = (
        torch.hann_window(512, device=args.device)
        if args.pitch_method
        in ("phase-vocoder-cached", "phase-vocoder-sparse-v1")
        else None
    )
    failures: list[dict[str, str]] = []
    started = time.perf_counter()

    for index, record in enumerate(pending_records, start=1):
        try:
            waveform, sample_rate = torchaudio.load(str(record["audio_path"]))
            waveform = waveform.mean(dim=0, keepdim=True).to(args.device)
            if sample_rate != 16_000:
                if sample_rate not in resamplers:
                    resamplers[sample_rate] = torchaudio.transforms.Resample(
                        sample_rate, 16_000
                    ).to(args.device)
                waveform = resamplers[sample_rate](waveform)

            prepared: list[tuple[Path, int]] = []
            spectrogram_batches = []
            with torch.inference_mode():
                shared_spectrum = None
                phase_advance = None
                if args.pitch_method in (
                    "phase-vocoder-cached",
                    "phase-vocoder-sparse-v1",
                ):
                    shared_spectrum = torch.stft(
                        waveform,
                        n_fft=512,
                        hop_length=128,
                        win_length=512,
                        window=phase_window,
                        center=True,
                        pad_mode="reflect",
                        normalized=False,
                        onesided=True,
                        return_complex=True,
                    )
                    phase_advance = torch.linspace(
                        0,
                        math.pi * 128,
                        shared_spectrum.shape[-2],
                        device=shared_spectrum.device,
                    )[..., None]
                for semitones in args.semitones:
                    destination = cache_path(args.cache_dir, semitones, record)
                    if valid_cached(destination, args.embedding_dim):
                        continue
                    if args.pitch_method == "phase-vocoder":
                        shifted = audio_functional.pitch_shift(
                            waveform, sample_rate=16_000, n_steps=semitones
                        )
                    elif args.pitch_method == "phase-vocoder-cached":
                        shifted = cached_phase_vocoder_shift(
                            shared_spectrum,
                            waveform.shape[-1],
                            semitones,
                            phase_window,
                            phase_advance,
                            pitch_only_resamplers[semitones],
                        )
                    elif args.pitch_method == "phase-vocoder-sparse-v1":
                        shifted = cached_phase_vocoder_shift(
                            shared_spectrum,
                            waveform.shape[-1],
                            semitones,
                            phase_window,
                            phase_advance,
                            sparse_pitch_only_resamplers[semitones],
                        )
                    elif args.pitch_method == "resample-speed":
                        shifted = pitch_resamplers[semitones](waveform)
                    else:
                        rate = 2.0 ** (semitones / 12.0)
                        output_length = max(1, round(waveform.shape[-1] / rate))
                        shifted = neural_functional.interpolate(
                            waveform.unsqueeze(0),
                            size=output_length,
                            mode="linear",
                            align_corners=False,
                        ).squeeze(0)
                    spectrogram = mel_spectrogram(shifted)
                    batched = model.preprocessor._batch_spectrogram(
                        spectrogram, model.config.n_frames
                    )
                    prepared.append((destination, len(batched)))
                    spectrogram_batches.append(batched)
                    del shifted, spectrogram

                if spectrogram_batches:
                    all_spectrograms = torch.cat(spectrogram_batches)
                    output_batches = []
                    for start in range(0, len(all_spectrograms), args.model_batch_size):
                        output_batches.append(
                            model(all_spectrograms[start : start + args.model_batch_size])
                        )
                    all_embeddings = torch.cat(output_batches)

            cursor = 0
            for (destination, count) in prepared:
                embeddings = all_embeddings[cursor : cursor + count]
                cursor += count
                value = embeddings.detach().to(device="cpu", dtype=torch.float32).numpy()
                if (
                    value.ndim != 2
                    or value.shape[0] < 1
                    or value.shape[1] != args.embedding_dim
                ):
                    raise ValueError(f"unexpected embedding shape {value.shape}")
                if not np.isfinite(value).all():
                    raise ValueError("non-finite embedding")
                atomic_save(destination, value)
            if spectrogram_batches:
                del all_spectrograms, all_embeddings, output_batches
        except Exception as error:  # preserve per-shift caches for a resumable retry
            failures.append({"id": str(record["id"]), "error": repr(error)})

        if index == 1 or index % 10 == 0 or index == len(pending_records):
            elapsed = max(time.perf_counter() - started, 1e-9)
            print(
                f"processed_records={index}/{len(pending_records)} failed={len(failures)} "
                f"rate={index / elapsed:.3f} records/s",
                flush=True,
            )
            write_metadata(args.cache_dir, args, manifest_hash, records, failures)

    write_metadata(args.cache_dir, args, manifest_hash, records, failures)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
