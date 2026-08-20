#!/usr/bin/env python3
"""Extract CQT, Mel, and HPCP features from GiantSteps audio for CNN training.

Reads the actual GiantSteps annotation format:
  - Audio: giantsteps-key/audio/*.mp3
  - Labels: giantsteps-key/annotations/key/*.key (contains e.g. "C minor")
  - Genre: giantsteps-key/annotations/genre/*.genre (contains e.g. "tech-house")

Usage:
    python extract_features.py --dataset_dir D:/tunelock-ml/giantsteps-key --output D:/tunelock-ml/data/features.h5
    python extract_features.py --dataset_dir D:/tunelock-ml/giantsteps-mtg-key --output D:/tunelock-ml/data/features_mtg.h5
"""

import argparse
import os
import sys
from pathlib import Path

import h5py
import librosa
import numpy as np
from tqdm import tqdm

# Key to index mapping (24 keys: 12 major + 12 minor)
# Using pitch class + mode (0=major, 1=minor)
KEY_TO_IDX = {
    'C major': 0, 'C# major': 1, 'Db major': 1, 'D major': 2, 'D# major': 3,
    'Eb major': 3, 'E major': 4, 'F major': 5, 'F# major': 6, 'Gb major': 6,
    'G major': 7, 'G# major': 8, 'Ab major': 8, 'A major': 9, 'A# major': 10,
    'Bb major': 10, 'B major': 11,
    'C minor': 12, 'C# minor': 13, 'Db minor': 13, 'D minor': 14, 'D# minor': 15,
    'Eb minor': 15, 'E minor': 16, 'F minor': 17, 'F# minor': 18, 'Gb minor': 18,
    'G minor': 19, 'G# minor': 20, 'Ab minor': 20, 'A minor': 21, 'A# minor': 22,
    'Bb minor': 22, 'B minor': 23,
}


def extract_cqt(y, sr=22050, hop_length=512, n_bins=80, bins_per_octave=12):
    """Extract Constant-Q Transform features."""
    cqt = np.abs(librosa.cqt(
        y, sr=sr, hop_length=hop_length,
        n_bins=n_bins, bins_per_octave=bins_per_octave,
    ))
    cqt = librosa.amplitude_to_db(cqt, ref=np.max)
    cqt = (cqt - cqt.mean()) / (cqt.std() + 1e-8)
    return cqt.T  # (frames, bins)


def extract_mel(y, sr=22050, hop_length=512, n_mels=80):
    """Extract Mel spectrogram features."""
    mel = librosa.feature.melspectrogram(
        y=y, sr=sr, hop_length=hop_length, n_mels=n_mels,
    )
    mel = librosa.power_to_db(mel, ref=np.max)
    mel = (mel - mel.mean()) / (mel.std() + 1e-8)
    return mel.T  # (frames, n_mels)


def extract_hpcp(y, sr=22050, hop_length=512, n_bins=12):
    """Extract Harmonic Pitch Class Profile (chroma)."""
    chroma = librosa.feature.chroma_cqt(
        y=y, sr=sr, hop_length=hop_length, n_chroma=n_bins,
    )
    chroma = (chroma - chroma.mean()) / (chroma.std() + 1e-8)
    return chroma.T  # (frames, 12)


def load_audio(filepath, sr=22050, duration=30.0):
    """Load audio file, center `duration` seconds.

    Bug fix (Step 7): Previously, `duration=30.0` was passed to
    `librosa.load`, which truncates the audio to the first 30 seconds
    BEFORE centering. The centering branch was dead code because
    `len(y)` could never exceed `sr * duration`.

    Now we load the full audio (or a generous prefix) and center
    properly. This ensures we capture the middle of the track, not
    the intro.
    """
    # Load the full audio, or up to duration*3 samples (enough to center)
    max_samples = int(sr * duration * 3)
    y, _ = librosa.load(filepath, sr=sr, mono=True)
    target_len = int(sr * duration)
    if len(y) > target_len:
        start = (len(y) - target_len) // 2
        y = y[start:start + target_len]
    elif len(y) < target_len:
        y = np.pad(y, (0, target_len - len(y)))
    return y


def parse_key_file(key_path):
    """Parse a GiantSteps .key annotation file.
    
    Format: "C minor" or "d minor\t2\t" (MTG format with confidence).
    Returns the key index (0-23) or None.
    """
    try:
        content = open(key_path, 'r', encoding='utf-8', errors='ignore').read().strip()
        # MTG format: "d minor\t2\t" — lowercase + tab + confidence
        # Standard format: "C minor"
        # Take the first token group before any tab
        key_str = content.split('\t')[0].strip()
        
        # Normalize: capitalize first letter, keep rest lowercase
        parts = key_str.split()
        if len(parts) < 2:
            return None
        
        root = parts[0].capitalize()
        mode = parts[1].lower()
        
        # Handle sharps/flats
        if '#' in root:
            root = root.replace('#', '#')
        if 'b' in root and len(root) == 2:
            root = root[0] + 'b'
        
        key_normalized = f"{root} {mode}"
        
        return KEY_TO_IDX.get(key_normalized)
    except Exception:
        return None


def parse_genre_file(genre_path):
    """Parse a GiantSteps .genre annotation file."""
    try:
        content = open(genre_path, 'r', encoding='utf-8', errors='ignore').read().strip()
        return content.lower()
    except Exception:
        return None


def pad_or_truncate(arr, target_len):
    """Pad or truncate the first dimension to target_len."""
    if arr.shape[0] >= target_len:
        return arr[:target_len]
    padding = np.zeros((target_len - arr.shape[0],) + arr.shape[1:], dtype=arr.dtype)
    return np.vstack([arr, padding])


def main():
    parser = argparse.ArgumentParser(description='Extract features for CNN training')
    parser.add_argument('--dataset_dir', required=True, help='GiantSteps dataset root directory')
    parser.add_argument('--output', required=True, help='Output HDF5 file')
    parser.add_argument('--sr', type=int, default=22050, help='Sample rate')
    parser.add_argument('--duration', type=float, default=30.0, help='Clip duration in seconds')
    parser.add_argument('--target_frames', type=int, default=252, help='Target number of frames')
    args = parser.parse_args()

    dataset_dir = Path(args.dataset_dir)
    audio_dir = dataset_dir / 'audio'
    key_dir = dataset_dir / 'annotations' / 'key'
    genre_dir = dataset_dir / 'annotations' / 'genre'

    if not audio_dir.exists():
        print(f'Audio directory not found: {audio_dir}')
        sys.exit(1)
    if not key_dir.exists():
        print(f'Key annotations directory not found: {key_dir}')
        sys.exit(1)

    # Find all audio files
    audio_files = sorted(list(audio_dir.glob('*.mp3')) + list(audio_dir.glob('*.flac')) + list(audio_dir.glob('*.wav')))
    if not audio_files:
        print(f'No audio files found in {audio_dir}')
        sys.exit(1)

    print(f'Found {len(audio_files)} audio files')

    features = []
    labels = []
    genres = []
    filenames = []
    skipped = 0
    label_dist = {}

    for filepath in tqdm(audio_files, desc='Extracting'):
        # Find the corresponding .key file
        stem = filepath.stem  # e.g., "1004923.LOFI"
        key_path = key_dir / f"{stem}.key"
        genre_path = genre_dir / f"{stem}.genre"

        if not key_path.exists():
            skipped += 1
            continue

        label = parse_key_file(key_path)
        if label is None:
            print(f'  Could not parse key: {key_path}')
            skipped += 1
            continue

        genre = parse_genre_file(genre_path) if genre_path.exists() else None

        try:
            y = load_audio(str(filepath), sr=args.sr, duration=args.duration)
            cqt = extract_cqt(y, sr=args.sr)
            mel = extract_mel(y, sr=args.sr)
            hpcp = extract_hpcp(y, sr=args.sr)

            # Pad or truncate to target frames
            cqt = pad_or_truncate(cqt, args.target_frames)
            mel = pad_or_truncate(mel, args.target_frames)
            hpcp = pad_or_truncate(hpcp, args.target_frames)

            features.append({
                'cqt': cqt.astype(np.float32),
                'mel': mel.astype(np.float32),
                'hpcp': hpcp.astype(np.float32),
            })
            labels.append(label)
            genres.append(genre.encode('utf-8') if genre else b'')
            filenames.append(filepath.name.encode('utf-8'))
            label_dist[label] = label_dist.get(label, 0) + 1

        except Exception as e:
            print(f'  Error processing {filepath.name}: {e}')
            skipped += 1

    print(f'\nExtracted: {len(features)}, Skipped: {skipped}')
    print(f'Label distribution ({len(label_dist)} classes):')
    idx_to_key = {v: k for k, v in KEY_TO_IDX.items()}
    for idx in sorted(label_dist.keys()):
        print(f'  {idx_to_key.get(idx, idx)}: {label_dist[idx]}')

    if len(features) == 0:
        print('No features extracted — aborting.')
        sys.exit(1)

    # Save to HDF5
    os.makedirs(os.path.dirname(args.output), exist_ok=True)
    with h5py.File(args.output, 'w') as f:
        f.create_dataset('cqt', data=np.array([x['cqt'] for x in features]))
        f.create_dataset('mel', data=np.array([x['mel'] for x in features]))
        f.create_dataset('hpcp', data=np.array([x['hpcp'] for x in features]))
        f.create_dataset('labels', data=np.array(labels))
        f.create_dataset('genres', data=np.array(genres))
        f.create_dataset('filenames', data=np.array(filenames))

    print(f'Saved to {args.output}')


if __name__ == '__main__':
    main()
