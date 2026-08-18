# TuneLock Key Detection — Test Findings Report

**Date:** 2026-05-29  
**Pipeline:** STFT(16384) → HPSS → 12-bin + 72-band chroma → Dual Ensemble (Krumhansl + Temperley + Sha'ath 72) → 8-segment ranked voting  
**Tracks:** 5 audio files from `C:\Users\louis.media\Music\Tunelock Test Tracks`

---

## 1. Executive Summary

All 5 tracks processed successfully — zero crashes, zero decode failures. The fused KeyFinder + TuneLock pipeline demonstrates **excellent stability** and **fast performance** (~1s per track in release). Key confidence varies by track: two tracks show near-certainty (>0.90), while three tracks exhibit moderate confidence (~0.68) with split temporal votes, suggesting either weaker tonality, key modulation, or genuine ambiguity between related keys.

---

## 2. Per-Track Results

### Track 1 — "01 Perra (Lusho remix).mp3"

| Metric | Value |
|---|---|
| **Detected Key** | **E minor (9A)** |
| Confidence | **0.904** — very strong |
| Agreement | **87.5%** (7/8 segments) |
| Avg Profile Score | 0.948 |
| Runner-up | G# minor (1A), conf=0.451, agree=12.5% |
| Total Time | 878ms |

**Analysis:** Unambiguous E minor. The runner-up (G# minor) is distant on the circle of fifths and carries low confidence — a clean separation. Chroma bars show a clear E minor scale-degree pattern with strong tonic (E/G#) and dominant (B) peaks.

---

### Track 2 — "02 Baja Panty (Instrumental).mp3"

| Metric | Value |
|---|---|
| **Detected Key** | **A minor (8A)** |
| Confidence | **0.682** — moderate |
| Agreement | **50.0%** (4/8 segments) |
| Avg Profile Score | 0.956 |
| Runner-up #1 | E minor (9A), conf=0.535, agree=25%, segs=2/8 |
| Runner-up #2 | D# minor (2A), conf=0.461, agree=12.5% |
| Runner-up #3 | D minor (7A), conf=0.457, agree=12.5% |
| Total Time | 639ms |

**Analysis:** Split vote — only half the segments agree on A minor. The strong runner-up E minor (9A) is the **dominant minor** of A minor (E is the V of A). This pattern is common in tracks where the dominant chord is emphasized rhythmically or melodically. The high avg score (0.956) across segments that *did* pick A minor suggests the profile match is good when the key is present — the ambiguity is temporal, not tonal weakness.

**Hypothesis:** The track may have sections in E minor (dominant emphasis) and sections in A minor, causing the split.

---

### Track 3 — "06 D-Stroy & DJ Tony Touch - Palante Siempre Palante.mp3"

| Metric | Value |
|---|---|
| **Detected Key** | **A minor (8A)** |
| Confidence | **0.684** — moderate |
| Agreement | **50.0%** (4/8 segments) |
| Avg Profile Score | 0.961 |
| Runner-up #1 | E minor (9A), conf=0.610, agree=37.5%, segs=3/8 |
| Runner-up #2 | C minor (5A), conf=0.459, agree=12.5% |
| Total Time | 1354ms |

**Analysis:** Nearly identical pattern to Track 2 — A minor winner with E minor runner-up. However, the E minor runner-up is *stronger* here (conf=0.610 vs 0.535), and captures 3/8 segments vs 2/8. This suggests an even more pronounced dominant (E minor) presence. C minor appearing as #3 may be a chromatic neighbor or a mis-rotation artifact.

**Hypothesis:** Strong dominant-minor presence throughout. The track may genuinely sit between A minor and E minor depending on section.

---

### Track 4 — "06. gotta go home (long 12'' version).mp3"

| Metric | Value |
|---|---|
| **Detected Key** | **D# minor (2A)** |
| Confidence | **0.686** — moderate |
| Agreement | **50.0%** (4/8 segments) |
| Avg Profile Score | 0.966 |
| Runner-up #1 | G# minor (1A), conf=0.461, agree=12.5% |
| Runner-up #2 | G# major (4B), conf=0.461, agree=12.5% |
| Runner-up #3 | F minor (4A), conf=0.460, agree=12.5% |
| Runner-up #4 | C minor (5A), conf=0.458, agree=12.5% |
| Total Time | 1420ms |

**Analysis:** The most fragmented result. The winner (D# minor) only holds 4/8 segments, and the remaining 4 segments split across 4 different keys. Notably, G# minor and G# major both appear — these share the same tonic (G#) but differ in mode. This "tonic clustering" (multiple modes on the same root) is a classic sign of a weakly defined mode — the tonic is clear but major vs minor is ambiguous.

**Hypothesis:** The track may be in a **blues-influenced or modal key** where the 3rd is bent/ambiguous, or it may modulate frequently. The 12" long version format often includes extended instrumental breaks with less tonal stability.

---

### Track 5 — "1 FS Green - Go To Work Ft. Dave Nunes.mp3"

| Metric | Value |
|---|---|
| **Detected Key** | **E minor (9A)** |
| Confidence | **0.911** — very strong |
| Agreement | **87.5%** (7/8 segments) |
| Avg Profile Score | 0.965 |
| Runner-up | B minor (10A), conf=0.441, agree=12.5% |
| Total Time | 864ms |

**Analysis:** Another unambiguous E minor, mirroring Track 1. The runner-up B minor is the supertonic (ii) of A major / relative major of F# minor — not a close neighbor on the circle of fifths, so this is a clean result. The chroma bars show a well-defined minor pattern with strong E/G/B peaks.

---

## 3. Aggregate Statistics

| Metric | Value |
|---|---|
| **Avg Confidence** | **0.774** |
| **Avg Agreement** | **65.0%** |
| **Avg Time** | **1031ms** |
| **Total Time (5 tracks)** | **5155ms** |

### Stage Breakdown (Average)

| Stage | Time | % of Total |
|---|---|---|
| Decode | ~404ms¹ | 39% |
| Spectrogram | 197ms | 19% |
| HPSS | 110ms | 11% |
| Chromagram | 301ms | 29% |
| Ensemble | <1ms² | <0.1% |

¹ Decode time varies by file size/codec (222–646ms).  
² Ensemble shows 0ms due to millisecond rounding; actual time is sub-millisecond.

---

## 4. Key Observations

### 4.1 Confidence Distribution
- **High confidence (>0.90):** 2/5 tracks (E minor, stable tonality)
- **Moderate confidence (~0.68):** 3/5 tracks (split votes, weaker or shifting tonality)

This distribution is **realistic and healthy**. A key detector that returns >0.90 for every track is either overconfident or processing only simple material. The moderate-confidence tracks genuinely exhibit ambiguity.

### 4.2 Temporal Split Patterns
- **Stable tracks** (1, 5): 87.5% agreement — the key is consistent across the entire track.
- **Ambiguous tracks** (2, 3, 4): 50% agreement — the key changes or is weakly defined in some sections.

Tracks 2 and 3 both show **A minor vs E minor splits**. E minor is the dominant minor of A minor (V in minor). This is a well-known ambiguity in tonal music — the dominant chord is so structurally important that profile-matching methods sometimes confuse tonic and dominant, especially in loop-based or sample-driven tracks where the bass emphasizes the dominant.

### 4.3 Chroma Vector Quality
All tracks show **non-flat chroma distributions** with visible peaks. No track produced a near-uniform chroma (which would indicate atonality or extreme noise). The chroma bars align with the detected keys:
- E minor tracks: peaks at E, G, B (tonic, minor 3rd, perfect 5th)
- A minor tracks: peaks at A, C, E
- D# minor track: peaks at D#, F#, G# (though more diffuse)

### 4.4 Performance
- **1.0s average per track** in release build is excellent for a 16384-point STFT + HPSS + dual chromagram pipeline.
- The longest track (306s) took 1420ms — roughly linear with duration, as expected.
- HPSS is the biggest computational win: without it, percussive transients would pollute the chroma, but the median filter cost is only ~110ms.

---

## 5. Comparison with Original TuneLock (Pre-KeyFinder Integration)

| Aspect | Original TuneLock | Current (KeyFinder-fused) |
|---|---|---|
| Chroma bands | 12 (collapsed) | 72 (6 octaves, DSK) |
| Sha'ath profile | 12-element normalized | 72-element octave-weighted |
| Chroma method | MIDI bin mapping | Cosine-windowed DSK (CQT approx) |
| FFT size | 4096 (~5.4 Hz/bin) | 16384 (~1.35 Hz/bin) |
| Frequency resolution | Adequate | High (resolves C1 = 32.7 Hz) |
| Temporal voting | Single 12-bin chroma | Dual: 12-bin + 72-band ensemble |

The 72-band chroma preserves octave information, allowing the Sha'ath profiles to weight bass octaves (where key lives) more heavily than treble (cymbal hash). The Direct Spectral Kernel is also more robust to tuning drift than MIDI rounding.

---

## 6. Actionable Recommendations

### Immediate (Post-test)
1. **Ensemble timing precision:** Change `timings.ensemble` from `as_millis()` to `as_micros()` to capture sub-millisecond ensemble times accurately.
2. **Add a "dominant ambiguity" flag:** When the runner-up is the dominant (V) of the winner, surface this in the UI as "strong dominant presence — key may feel less stable."

### Short-term
3. **Tonic clustering detection:** When multiple candidates share the same tonic but differ in mode (e.g., G# minor + G# major), flag "ambiguous mode — clear tonic." This is common in blues/rock tracks.
4. **Key stability index:** Compute the variance of chroma vectors across segments. High variance + low agreement = likely modulation. Surface as "key changes throughout track."

### Medium-term
5. **CNN augmentation:** The classical profiles are doing well on stable tracks but struggle with dominant-ambiguous tracks. A small CNN trained on CQT/Mel/HPCP features could break ties in these ambiguous cases.
6. **Profile weight tuning:** The default weights (`krumhansl=0.4, temperley=0.5, shaath=0.5`) were guessed. On the ambiguous tracks, Krumhansl may be over-weighted for electronic/hip-hop material (it was derived from Western classical listening studies). Consider genre-specific weight presets.

---

## 7. Raw Data

```
Track 1: E minor (9A)  conf=0.904  agree=87.5%  878ms
Track 2: A minor (8A)  conf=0.682  agree=50.0%  639ms
Track 3: A minor (8A)  conf=0.684  agree=50.0%  1354ms
Track 4: D# minor (2A) conf=0.686  agree=50.0%  1420ms
Track 5: E minor (9A)  conf=0.911  agree=87.5%  864ms
```

---

*Report generated by TuneLock CLI test harness (`tunelock-bench`).*
