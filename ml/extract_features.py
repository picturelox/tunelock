#!/usr/bin/env python3
"""Extract CQT, Mel, and HPCP features from audio files for CNN training.

Usage:
    python extract_features.py --audio_dir ../ground-truth/giantsteps/audio --output data/features.h5
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
KEY_TO_IDX = {
    'C major': 0, 'C# major': 1, 'D major': 2, 'D# major': 3,
    'E major': 4, 'F major': 5, 'F# major': 6, 'G major': 7,
    'G# major': 8, 'A major': 9, 'A# major': 10, 'B major': 11,
    'C minor': 12, 'C# minor': 13, 'D minor': 14, 'D# minor': 15,
    'E minor': 16, 'F minor': 17, 'F# minor': 18, 'G minor': 19,
    'G# minor': 20, 'A minor': 21, 'A# minor': 22, 'B minor': 23,
}

# Camelot to standard key mapping
CAMELOT_TO_KEY = {
    '1A': 'Ab minor', '1B': 'B major',
    '2A': 'Eb minor', '2B': 'F# major',
    '3A': 'Bb minor', '3B': 'Db major',
    '4A': 'F minor', '4B': 'Ab major',
    '5A': 'C minor', '5B': 'Eb major',
    '6A': 'G minor', '6B': 'Bb major',
    '7A': 'D minor', '7B': 'F major',
    '8A': 'A minor', '8B': 'C major',
    '9A': 'E minor', '9B': 'G major',
    '10A': 'B minor', '10B': 'D major',
    '11A': 'F# minor', '11B': 'A major',
    '12A': 'Db minor', '12B': 'E major',
}


def extract_cqt(y, sr=22050, hop_length=512, n_bins=80, bins_per_octave=12):
    """Extract Constant-Q Transform features."""
    cqt = np.abs(librosa.cqt(
        y, sr=sr, hop_length=hop_length,
        n_bins=n_bins, bins_per_octave=bins_per_octave,
    ))
    # Log-scale
    cqt = librosa.amplitude_to_db(cqt, ref=np.max)
    # Normalize
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
    """Extract Harmonic Pitch Class Profile (chroma-like)."""
    chroma = librosa.feature.chroma_cqt(
        y=y, sr=sr, hop_length=hop_length, n_chroma=n_bins,
    )
    chroma = (chroma - chroma.mean()) / (chroma.std() + 1e-8)
    return chroma.T  # (frames, 12)


def load_audio(filepath, sr=22050, duration=30.0):
    """Load audio file, center 30 seconds."""
    y, _ = librosa.load(filepath, sr=sr, mono=True)
    if len(y) > sr * duration:
        start = (len(y) - int(sr * duration)) // 2
        y = y[start:start + int(sr * duration)]
    return y


def parse_label(filename):
    """Parse key label from filename (GiantSteps format: 'artist - title - KEY')."""
    # Try to extract Camelot or standard key from filename
    parts = filename.rsplit('.', 1)[0].split(' - ')
    for part in parts:
        part = part.strip()
        if part in CAMELOT_TO_KEY:
            key = CAMELOT_TO_KEY[part]
            # Normalize to our 24-key space
            return normalize_key(key)
        if part in KEY_TO_IDX:
            return KEY_TO_IDX[part]
    return None


def normalize_key(key):
    """Normalize a key string to our 24-key index."""
    key = key.replace('m ', ' minor ').replace('M ', ' major ')
    # Handle enharmonic equivalents
    equivalents = {
        'Db major': 'C# major', 'Eb major': 'D# major',
        'Gb major': 'F# major', 'Ab major': 'G# major', 'Bb major': 'A# major',
        'Db minor': 'C# minor', 'Eb minor': 'D# minor',
        'Gb minor': 'F# minor', 'Ab minor': 'G# minor', 'Bb minor': 'A# minor',
        'C# minor': 'Db minor',  # prefer flat for minor
    }
    key = equivalents.get(key, key)
    return KEY_TO_IDX.get(key)


def main():
    parser = argparse.ArgumentParser(description='Extract features for CNN training')
    parser.add_argument('--audio_dir', required=True, help='Directory of audio files')
    parser.add_argument('--output', required=True, help='Output HDF5 file')
    parser.add_argument('--labels', help='Optional CSV with labels (filepath,key)')
    parser.add_argument('--sr', type=int, default=22050, help='Sample rate')
    parser.add_argument('--duration', type=float, default=30.0, help='Clip duration in seconds')
    args = parser.parse_args()

    audio_dir = Path(args.audio_dir)
    audio_files = sorted(audio_dir.glob('*.mp3')) + sorted(audio_dir.glob('*.flac')) + sorted(audio_dir.glob('*.wav'))

    if not audio_files:
        print(f'No audio files found in {audio_dir}')
        sys.exit(1)

    print(f'Found {len(audio_files)} audio files')

    features = []
    labels = []
    skipped = 0

    for filepath in tqdm(audio_files, desc='Extracting'):
        try:
            y = load_audio(str(filepath), sr=args.sr, duration=args.duration)
            cqt = extract_cqt(y, sr=args.sr)
            mel = extract_mel(y, sr=args.sr)
            hpcp = extract_hpcp(y, sr=args.sr)

            # Pad or truncate to 252 frames
            target_frames = 252
            cqt = pad_or_truncate(cqt, target_frames)
            mel = pad_or_truncate(mel, target_frames)
            hpcp = pad_or_truncate(hpcp, target_frames)

            label = parse_label(filepath.name)
            if label is None:
                skipped += 1
                continue

            features.append({
                'cqt': cqt.astype(np.float32),
                'mel': mel.astype(np.float32),
                'hpcp': hpcp.astype(np.float32),
            })
            labels.append(label)

        except Exception as e:
            print(f'Error processing {filepath.name}: {e}')
            skipped += 1

    print(f'Extracted: {len(features)}, Skipped: {skipped}')

    # Save to HDF5
    with h5py.File(args.output, 'w') as f:
        f.create_dataset('cqt', data=np.array([x['cqt'] for x in features]))
        f.create_dataset('mel', data=np.array([x['mel'] for x in features]))
        f.create_dataset('hpcp', data=np.array([x['hpcp'] for x in features]))
        f.create_dataset('labels', data=np.array(labels))

    print(f'Saved to {args.output}')


def pad_or_truncate(arr, target_len):
    """Pad or truncate the first dimension to target_len."""
    if arr.shape[0] >= target_len:
        return arr[:target_len]
    padding = np.zeros((target_len - arr.shape[0],) + arr.shape[1:], dtype=arr.dtype)
    return np.vstack([arr, padding])


if __name__ == '__main__':
    main()
