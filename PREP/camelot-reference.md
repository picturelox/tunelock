# Camelot Wheel Reference & Harmonic Mixing Rules

> This document is the implementation reference for all Camelot wheel logic in NotMixedInKey.  
> Every mapping, rule, and formula a developer needs to code the harmonic engine.

---

## 1. Complete Key ↔ Camelot Mapping Table

| Camelot Code | Musical Key | Circle of Fifths Position | Pitch Class Root |
|:---:|:---|:---:|:---:|
| **1A** | A♭ minor (G# minor) | 1 | 8 |
| **1B** | B major | 1 | 11 |
| **2A** | E♭ minor (D# minor) | 2 | 3 |
| **2B** | F# major (G♭ major) | 2 | 6 |
| **3A** | B♭ minor (A# minor) | 3 | 10 |
| **3B** | D♭ major (C# major) | 3 | 1 |
| **4A** | F minor | 4 | 5 |
| **4B** | A♭ major (G# major) | 4 | 8 |
| **5A** | C minor | 5 | 0 |
| **5B** | E♭ major (D# major) | 5 | 3 |
| **6A** | G minor | 6 | 7 |
| **6B** | B♭ major (A# major) | 6 | 10 |
| **7A** | D minor | 7 | 2 |
| **7B** | F major | 7 | 5 |
| **8A** | A minor | 8 | 9 |
| **8B** | C major | 8 | 0 |
| **9A** | E minor | 9 | 4 |
| **9B** | G major | 9 | 7 |
| **10A** | B minor | 10 | 11 |
| **10B** | D major | 10 | 2 |
| **11A** | F# minor (G♭ minor) | 11 | 6 |
| **11B** | A major | 11 | 9 |
| **12A** | D♭ minor (C# minor) | 12 | 1 |
| **12B** | E major | 12 | 4 |

> **Note:** The Camelot number wraps — after 12 comes 1 (it's a wheel/clock).  
> **Note:** Each number has an A (minor) and B (major) version. A and B at the same number are **relative major/minor** pairs.

---

## 2. Reverse Lookup: Musical Key → Camelot Code

Use this for the implementation — after detecting a key in standard notation, map it to Camelot.

```
KEY_TO_CAMELOT = {
    // Minor keys (A ring)
    "Ab minor":  "1A",   "G# minor":  "1A",
    "Eb minor":  "2A",   "D# minor":  "2A",
    "Bb minor":  "3A",   "A# minor":  "3A",
    "F minor":   "4A",
    "C minor":   "5A",
    "G minor":   "6A",
    "D minor":   "7A",
    "A minor":   "8A",
    "E minor":   "9A",
    "B minor":   "10A",
    "F# minor":  "11A",  "Gb minor":  "11A",
    "Db minor":  "12A",  "C# minor":  "12A",

    // Major keys (B ring)
    "B major":   "1B",
    "F# major":  "2B",   "Gb major":  "2B",
    "Db major":  "3B",   "C# major":  "3B",
    "Ab major":  "4B",   "G# major":  "4B",
    "Eb major":  "5B",   "D# major":  "5B",
    "Bb major":  "6B",   "A# major":  "6B",
    "F major":   "7B",
    "C major":   "8B",
    "G major":   "9B",
    "D major":   "10B",
    "A major":   "11B",
    "E major":   "12B",
}
```

---

## 3. Harmonic Compatibility Rules

### 3.1 Core Rules (Always Compatible)

| Rule Name | Move | Example | Musical Relationship | DJ Effect |
|:---|:---|:---|:---|:---|
| **Same Key** | Same code | 8A → 8A | Identical key | Perfect harmonic blend |
| **Adjacent Up** | +1 number, same letter | 8A → 9A | Perfect fifth up | Subtle energy lift |
| **Adjacent Down** | −1 number, same letter | 8A → 7A | Perfect fourth up | Subtle energy drop |
| **Relative Switch** | Same number, A↔B | 8A → 8B | Relative major/minor | Mood shift (minor→major = brighter) |

### 3.2 Extended Rules (Advanced / Situational)

| Rule Name | Move | Example | Musical Relationship | DJ Effect |
|:---|:---|:---|:---|:---|
| **Energy Boost** | +2 numbers, same letter | 8A → 10A | Whole step up (2 fifths) | Noticeable energy increase |
| **Energy Drop** | −2 numbers, same letter | 8A → 6A | Whole step down | Energy decrease |
| **Mood Boost** | +1 number, A→B | 8A → 9B | Dominant resolution feel | Uplifting transition |
| **Mood Drop** | −1 number, B→A | 8B → 7A | Subdominant darkening | Darkening transition |
| **Parallel Switch** | +3 numbers, A↔B | 8B → 5A (C maj → C min) | Parallel major/minor | Dramatic mood shift |
| **Diagonal Up** | +1 number, A→B | 8A → 9B | | Bright energy push |
| **Diagonal Down** | −1 number, B→A | 8B → 7A | | Dark energy pull |

### 3.3 Compatibility Score Formula

For implementing a compatibility score between two Camelot codes:

```
function compatibility_score(code_a, code_b):
    num_a = number part of code_a    // e.g., 8
    let_a = letter part of code_a    // e.g., "A"
    num_b = number part of code_b
    let_b = letter part of code_b

    // Calculate circular distance (wheel wraps at 12)
    distance = min(abs(num_a - num_b), 12 - abs(num_a - num_b))

    same_ring = (let_a == let_b)

    // Scoring
    if distance == 0 and same_ring:     return 1.00  // Same key
    if distance == 0 and not same_ring: return 0.90  // Relative major/minor
    if distance == 1 and same_ring:     return 0.85  // Adjacent (±1)
    if distance == 1 and not same_ring: return 0.70  // Diagonal (±1 cross-ring)
    if distance == 2 and same_ring:     return 0.60  // Energy boost/drop (±2)
    if distance == 2 and not same_ring: return 0.50  // ±2 cross-ring
    if distance == 3:                   return 0.30  // Parallel switch territory
    if distance <= 5:                   return 0.15  // Risky but sometimes works
    return 0.00  // Clashing — avoid
```

### 3.4 Circular Arithmetic Helper

The Camelot wheel is a clock (1–12). Arithmetic must wrap:

```
function camelot_add(number, offset):
    result = ((number - 1 + offset) % 12) + 1
    if result <= 0: result += 12
    return result

// Examples:
// camelot_add(11, +2) = 1    (wraps around)
// camelot_add(1, -1)  = 12   (wraps backward)
// camelot_add(8, +1)  = 9
```

---

## 4. Playlist Generation Algorithm

### 4.1 Greedy Harmonic Path

Given a starting track and a set of rules, build a playlist:

```
function generate_playlist(start_track, all_tracks, rules, max_length):
    playlist = [start_track]
    used = {start_track.id}
    current = start_track

    while len(playlist) < max_length:
        candidates = []

        for track in all_tracks:
            if track.id in used: continue

            score = compatibility_score(current.camelot, track.camelot)

            // Check if this move matches any of the user's selected rules
            if not matches_any_rule(current.camelot, track.camelot, rules):
                continue

            // Optional: BPM compatibility bonus
            bpm_diff = abs(current.bpm - track.bpm)
            bpm_penalty = min(bpm_diff / 20.0, 1.0)  // penalize >20 BPM difference
            score = score * (1.0 - bpm_penalty * 0.3)

            candidates.append((track, score))

        if not candidates: break

        // Sort by score descending, pick best (or random from top-3 for variety)
        candidates.sort(by=score, descending)
        next_track = candidates[0].track  // or random.choice(candidates[:3])

        playlist.append(next_track)
        used.add(next_track.id)
        current = next_track

    return playlist
```

### 4.2 Energy Curve Shaping

When the user selects an energy curve preference:

| Curve | Strategy |
|---|---|
| **Build Up** | Prefer +1, +2 moves. Sort candidates by ascending energy. Increase preference for higher-energy tracks as playlist progresses. |
| **Wind Down** | Prefer −1, −2 moves. Sort by descending energy. Prefer lower-energy tracks as playlist progresses. |
| **Peak & Valley** | Build up for first half, wind down for second half. |
| **Flat / Steady** | Prefer same-key and ±1 only. Minimize energy variance. |

---

## 5. Visual Representation: Camelot Wheel SVG Structure

The interactive wheel is an SVG with 24 segments (12 inner, 12 outer):

```
Outer Ring (Major / B keys):
  12 arc segments, each spanning 30° (360°/12)
  Labeled: 1B, 2B, 3B, ..., 12B
  Starting angle: 1B at 12 o'clock position (top), clockwise

Inner Ring (Minor / A keys):
  12 arc segments, each spanning 30°
  Labeled: 1A, 2A, 3A, ..., 12A
  Same angular positions as outer ring

Colors:
  Each number (1-12) gets a unique hue, rotated evenly around HSL color wheel:
  position_n → hsl((n-1) * 30, 70%, 50%)

  1  → hsl(0,   70%, 50%)  // red
  2  → hsl(30,  70%, 50%)  // orange
  3  → hsl(60,  70%, 50%)  // yellow
  4  → hsl(90,  70%, 50%)  // yellow-green
  5  → hsl(120, 70%, 50%)  // green
  6  → hsl(150, 70%, 50%)  // teal
  7  → hsl(180, 70%, 50%)  // cyan
  8  → hsl(210, 70%, 50%)  // blue
  9  → hsl(240, 70%, 50%)  // indigo
  10 → hsl(270, 70%, 50%)  // purple
  11 → hsl(300, 70%, 50%)  // magenta
  12 → hsl(330, 70%, 50%)  // pink

Inner ring (A/minor): slightly darker (lightness 40%)
Outer ring (B/major): standard (lightness 50%)
```

### Track Indicators on the Wheel

- Each analyzed track gets a small dot/circle placed on its Camelot segment
- Dot count per segment shows distribution of library across keys
- Clicking a segment selects it and highlights compatible segments (per rules)
- Clicking a dot selects that specific track

---

## 6. Key Profile Vectors (Implementation Ready)

These are the exact numeric arrays to use in `key_profiles.rs`:

### Krumhansl-Kessler Profiles

```
KRUMHANSL_MAJOR = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88]
                //  C     C#    D     D#    E     F     F#    G     G#    A     A#    B

KRUMHANSL_MINOR = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17]
                //  C     C#    D     D#    E     F     F#    G     G#    A     A#    B
```

### Temperley Profiles (Better for pop/rock/electronic)

```
TEMPERLEY_MAJOR = [5.0, 2.0, 3.5, 2.0, 4.5, 4.0, 2.0, 4.5, 2.0, 3.5, 1.5, 4.0]
TEMPERLEY_MINOR = [5.0, 2.0, 3.5, 4.5, 2.0, 4.0, 2.0, 4.5, 3.5, 2.0, 1.5, 4.0]
```

### Sha'ath Profiles (Tuned for electronic/dance — used in libkeyfinder)

```
SHAATH_MAJOR = [6.6, 2.0, 3.5, 2.3, 4.6, 4.0, 2.5, 5.2, 2.4, 3.7, 2.3, 3.4]
SHAATH_MINOR = [6.5, 2.7, 3.5, 5.4, 2.6, 3.5, 2.5, 5.2, 4.0, 2.7, 4.3, 3.2]
```

### How to generate all 24 profiles from a base pair:

```rust
fn generate_all_profiles(major_base: &[f64; 12], minor_base: &[f64; 12]) -> Vec<([f64; 12], String)> {
    let pitch_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let mut profiles = Vec::with_capacity(24);

    for shift in 0..12 {
        // Major profile for this key
        let mut major = [0.0; 12];
        for i in 0..12 {
            major[i] = major_base[(i + 12 - shift) % 12];
        }
        let key_name = format!("{} major", pitch_names[shift]);
        profiles.push((major, key_name));

        // Minor profile for this key
        let mut minor = [0.0; 12];
        for i in 0..12 {
            minor[i] = minor_base[(i + 12 - shift) % 12];
        }
        let key_name = format!("{} minor", pitch_names[shift]);
        profiles.push((minor, key_name));
    }

    profiles
}
```

---

## 7. Enharmonic Equivalents

When displaying keys, normalize enharmonic spellings for consistency:

| Detected | Display As | Camelot |
|---|---|---|
| G# minor | A♭ minor | 1A |
| D# minor | E♭ minor | 2A |
| A# minor | B♭ minor | 3A |
| G♭ minor | F# minor | 11A |
| C# minor | D♭ minor | 12A |
| G♭ major | F# major | 2B |
| C# major | D♭ major | 3B |
| G# major | A♭ major | 4B |
| D# major | E♭ major | 5B |
| A# major | B♭ major | 6B |

**Rule:** Prefer flats for keys on the left side of the circle of fifths (1-6), sharps for the right side (7-12). But always map to the correct Camelot code regardless of spelling.
