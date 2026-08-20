# TuneLock Transition Workbench — Product Feature Specification

**Status:** Product direction approved; implementation not started  
**Version:** 0.1  
**Date:** 2026-08-20  
**Owner:** TuneLock  
**Primary surface:** Mix Canvas  
**Related roadmap:** Phase 7, Waveforms and the mix-planning workbench

## 1. Executive decision

TuneLock should make a synchronized, waveform-led **Transition Workbench** the standard detailed view for every transition in Mix Canvas.

The workbench combines:

- two aligned tracks on one musical timeline;
- beat and phrase grids;
- quantized loops and cue points;
- synchronized transport and pitch-preserving tempo matching;
- deck and master meters;
- crossfader and three-band EQ kills;
- full-mix and, when prepared, four-stem waveforms;
- per-stem gain, mute, and solo;
- a saved, non-destructive transition plan.

This is a planning and audition environment, not a live-performance deck and not a general-purpose DAW. It should help a DJ answer: **where should these tracks overlap, which musical elements should be present, and will the transition remain danceable?**

Stem separation is optional enrichment. The standard workbench must remain useful immediately with the original files and TuneLock's cached three-band waveforms. No model load or separation job may delay normal key, BPM, energy, library, or Mix Canvas results.

## 2. Product rationale

TuneLock already recommends and scores track relationships across an entire collection. Its current Mix Canvas can order tracks, persist a mix, and explain harmonic/BPM relationships, but its audition surface is two independent audio elements with no shared clock, waveform, beat alignment, loop, crossfader, EQ, or meters.

The Transition Workbench closes the gap between **a recommendation** and **a decision the DJ can hear**. Stem-aware planning adds particular value in four common situations:

1. **Vocal collision:** see and hear whether two lead vocals overlap.
2. **Bass handoff:** choose the bar where the outgoing bass yields to the incoming bass.
3. **Rhythmic continuity:** keep drums running through a breakdown or use a drum-only intro/outro.
4. **Phrase construction:** loop a clean 8- or 16-bar region and test the transition at phrase boundaries.

The differentiator is not stem separation by itself. It is the combination of stem-level evidence with TuneLock's harmonic, tempo, energy, local-key, and set-level reasoning.

## 3. Goals and non-goals

### Goals

- Make the selected transition the primary unit of detailed planning.
- Let a DJ visually align, loop, and audition two tracks on one musical clock.
- Make vocals, bass, drums, and other musical material independently visible and audible when stems exist.
- Preserve a useful full-mix workflow when stems do not exist.
- Save the DJ's choices as a versioned transition plan inside the persisted mix.
- Keep all processing local and originals untouched.
- Degrade clearly and safely when beat confidence, stems, hardware acceleration, or optional tools are unavailable.

### Non-goals for the first release

- Performing a live set for an audience.
- Replacing Traktor, Rekordbox, Serato, Ableton Live, or a DAW.
- Recording an entire continuous DJ mix in real time.
- Freehand waveform or destructive source editing.
- Arbitrary plug-ins, sends, effects racks, or MIDI mapping.
- Automatic public sharing, cloud processing, or uploading music.
- Bundling a downloader, GPL/AGPL component, or model without verified redistribution rights.
- Six-stem separation. The initial semantic vocabulary is vocals, drums, bass, and other.

## 4. Product principles

1. **The local result renders first.** Full-mix planning opens without waiting for stem preparation, a model, a network call, or an LLM.
2. **One standard workflow.** Stems enrich the Transition Workbench; they do not create a disconnected application mode.
3. **The original is always available.** A DJ can A/B the separated render against the untouched source at any time.
4. **Non-destructive by construction.** TuneLock writes plans, cached analysis, and generated derivatives only. It never changes, moves, or deletes the original.
5. **Manual correction is first-class.** An estimated beat grid is a starting point. The DJ can set the first beat, move the grid, mark a downbeat, and change BPM.
6. **Advice, not authority.** Warnings such as vocal collision or bass overlap may be ignored deliberately.
7. **Planning fidelity over performance breadth.** Tight sync, trustworthy meters, and saved decisions matter more than effects or deck emulation.
8. **Four semantic stems, one vocabulary.** `vocals`, `drums`, `bass`, and `other` are the product vocabulary across Rust, TypeScript, cache manifests, and UI.

## 5. Primary user flow

1. The DJ adds and orders tracks in Mix Canvas.
2. The DJ selects the transition between Track A and Track B.
3. The Transition Workbench opens with both full-mix waveforms on one zoomable timeline.
4. TuneLock chooses an initial tempo master and aligns the estimated beat grids near a suggested phrase boundary.
5. The DJ corrects either grid if necessary, sets an overlap region, and chooses a quantized loop if useful.
6. The DJ starts synchronized playback, moves the crossfader, uses EQ kills, and watches deck/master meters.
7. If stems already exist, the four lanes are available immediately. If not, the DJ may choose **Prepare stems** and continue planning while the background job runs.
8. With stems available, the DJ mutes, solos, or adjusts vocals, drums, bass, and other for either track.
9. TuneLock visualizes relevant risks and opportunities, such as simultaneous lead-vocal activity, overlapping bass energy, a breakdown, or a local-key change.
10. The DJ saves the overlap, loop, cue, grid correction, gains, and automation as the transition plan.
11. On reopening the mix, the workbench restores the same audible result.

## 6. Information architecture

The Transition Workbench replaces the current fixed-height dual-audition footer when a transition is selected. It may be collapsed to recover canvas space, but it is the standard expanded transition view.

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ A: title / key / BPM / energy   Relationship   B: title / key / BPM / energy│
│ Master BPM  Sync  Quantize  Loop  Metronome  Original↔Stems  Save           │
├───────────┬───────────────────────────────────────────────────────┬──────────┤
│ TRACK A   │ bars / beats / phrase markers                         │ deck VU  │
│ Original  │ full-mix three-band waveform                          │ gain/EQ  │
│ Vocals    │ semantic stem waveform              M  S  gain        │          │
│ Drums     │ semantic stem waveform              M  S  gain        │          │
│ Bass      │ semantic stem waveform              M  S  gain        │          │
│ Other     │ semantic stem waveform              M  S  gain        │          │
├───────────┼════════ selected overlap / automation ════════════════┼──────────┤
│ TRACK B   │ bars / beats / phrase markers                         │ deck VU  │
│ Original  │ full-mix three-band waveform                          │ gain/EQ  │
│ Vocals    │ semantic stem waveform              M  S  gain        │          │
│ Drums     │ semantic stem waveform              M  S  gain        │          │
│ Bass      │ semantic stem waveform              M  S  gain        │          │
│ Other     │ semantic stem waveform              M  S  gain        │          │
├───────────┴───────────────────────────────────────────────────────┴──────────┤
│ ◀ cue  ▶ play/pause  ▶ next cue   time/bar   crossfader   master VU/clip    │
└────────────────────────────────────────────────────────────────────────────┘
```

### Visual rules

- Both tracks share horizontal scale, zoom, playhead, and beat/phrase grid.
- Beat lines are subtle; downbeats and phrase boundaries have increasing visual weight.
- The selected overlap is visible across both tracks.
- The original lane remains visible as a reference but is mutually exclusive with summed-stem playback for that deck, preventing double playback.
- Stem identity relies first on fixed lane order, labels, and icons. Stem tints remain restrained so Camelot remains the dominant saturated identity color.
- Muted lanes visibly recede. Soloed lanes remain prominent and all non-soloed lanes recede.
- Meter clipping may use a warning color; meters must not compete with key colors when idle.

## 7. Functional requirements

### 7.1 Selection and workspace

| ID | Requirement |
|---|---|
| TW-001 | Selecting a transition in Mix Canvas opens or updates the workbench with the correct adjacent tracks. |
| TW-002 | The workbench can be expanded, collapsed, and vertically resized. Its state persists per user, not per mix. |
| TW-003 | Switching transitions stops the old transport safely before changing sources. |
| TW-004 | The header shows each track's title, artist, Camelot key, BPM, energy, and relevant confidence/consensus state. |
| TW-005 | The relationship explanation remains accessible without covering waveform controls. |

### 7.2 Timeline and waveform

| ID | Requirement |
|---|---|
| TW-010 | Track A and Track B render on a shared, zoomable, horizontally scrollable musical timeline. |
| TW-011 | The default full-mix lane uses TuneLock's cached three-band waveform and is available without stems. |
| TW-012 | When prepared, each deck exposes aligned waveform lanes for vocals, drums, bass, and other. |
| TW-013 | All lanes for one track share duration, time origin, normalization policy, and zoom level. |
| TW-014 | Zooming or scrolling one deck updates both decks. |
| TW-015 | Clicking the ruler seeks the shared transport; dragging selects or adjusts the overlap region. |
| TW-016 | Waveform rendering remains smooth at 60 FPS during scroll, zoom, and playhead movement on reference hardware. |
| TW-017 | The UI distinguishes missing stems, queued work, active processing, failed work, stale cache, and ready stems. |

### 7.3 Beat and phrase grid

| ID | Requirement |
|---|---|
| TW-020 | Each track has a versioned beat grid containing BPM, first-beat time, meter, downbeat offset, and confidence. |
| TW-021 | The grid renders beats, bars, and configurable phrase markers, initially every 8/16/32 bars. |
| TW-022 | The DJ can set the current position as beat one, move the grid earlier/later, halve/double BPM, edit BPM, and mark a downbeat. |
| TW-023 | Manual corrections are stored separately from engine estimates and always win until explicitly reset. |
| TW-024 | Low-confidence grids are labeled plainly and never presented as certain. |
| TW-025 | Grid editing works without stems and does not trigger key/BPM re-analysis. |

### 7.4 Transport, sync, cues, and loops

| ID | Requirement |
|---|---|
| TW-030 | One master transport starts, pauses, seeks, and loops both decks from a shared clock. |
| TW-031 | The DJ can choose Track A, Track B, or a custom BPM as tempo master. |
| TW-032 | Tempo matching preserves musical pitch within an agreed perceptual tolerance. |
| TW-033 | Quantize choices include off, beat, 1 bar, 2 bars, 4 bars, 8 bars, 16 bars, and 32 bars. |
| TW-034 | Loop boundaries snap to the selected quantization and may be resized or moved. |
| TW-035 | The DJ can set, name, move, and delete transition-local cue points. Source-file cue metadata is not modified. |
| TW-036 | Optional metronome/count-in follows the master grid and is excluded from preview export. |
| TW-037 | A visible sync warning appears when measured playback drift exceeds tolerance. |

### 7.5 Mixing and meters

| ID | Requirement |
|---|---|
| TW-040 | Each deck has gain, three-band EQ, low/mid/high kills, and a stereo level meter. |
| TW-041 | An equal-power crossfader controls deck A and deck B summed output. |
| TW-042 | The master output has a stereo meter, peak/clip hold, and a clear-reset action. |
| TW-043 | With stems available, each stem has gain, mute, and solo controls. |
| TW-044 | Solo is exclusive within a deck by default; a modifier or explicit multi-solo action permits multiple soloed stems. |
| TW-045 | Mute/solo changes are click-free and apply without restarting transport. |
| TW-046 | A deck can switch between untouched Original playback and summed Stems playback for separation-quality A/B comparison. |
| TW-047 | The UI prevents Original and summed Stems from accidentally playing together on the same deck. |

### 7.6 Transition plan and automation

| ID | Requirement |
|---|---|
| TW-050 | Each transition persists grid overrides, anchors, overlap, cues, loop, tempo master, deck gains/EQ, stem states, crossfader state, and schema version. |
| TW-051 | The first release supports a small number of editable automation points for crossfader and per-stem gain, with linear or equal-power interpolation. |
| TW-052 | Presets include at least: Clean Blend, Bass Swap, Vocal Swap, Drums Under, and Hard Cut. Presets are starting points, never destructive replacements. |
| TW-053 | Saving is explicit and also participates in the existing dirty-state/autosave behavior once that behavior exists. |
| TW-054 | Loading an older plan performs a versioned migration or falls back safely without losing the mix. |
| TW-055 | Reset restores a neutral transition plan without deleting cached stems or track-level beat-grid corrections. |

### 7.7 Optional stem preparation

| ID | Requirement |
|---|---|
| TW-060 | **Prepare stems** is user-initiated and never part of the normal analysis critical path. |
| TW-061 | The initial separation contract returns exactly vocals, drums, bass, and other. |
| TW-062 | Progress, queue position, elapsed time, processing device, cancellation, and actionable errors are visible. |
| TW-063 | A cancelled or failed job removes incomplete outputs and preserves the original. |
| TW-064 | Outputs are cached by source fingerprint, provider, model, model version, and settings. |
| TW-065 | A configurable storage quota, per-project pinning, and **Remove generated stems** action prevent unbounded cache growth. |
| TW-066 | If no approved provider is available, the full-mix workbench remains functional and explains how stem support can be enabled. |
| TW-067 | TuneLock does not bundle or download a provider, model, or model weights until code and weight redistribution rights have been separately verified. |
| TW-068 | Separation may use CPU or an available accelerator, but GPU failure must either retry safely on CPU with notice or fail with a useful explanation. |

### 7.8 Planning intelligence

These signals arrive after the mechanical workbench is trustworthy.

| ID | Requirement |
|---|---|
| TW-070 | Vocal activity overlays identify likely lead-vocal overlap without claiming perfect vocal detection. |
| TW-071 | Bass-energy overlays highlight simultaneous bass passages and candidate handoff points. |
| TW-072 | Drum activity and onset density identify clean rhythmic entry/exit regions. |
| TW-073 | Energy and local-key timelines align with the same ruler and playhead. |
| TW-074 | TuneLock suggests phrase-aligned overlap regions and explains the evidence in plain English. |
| TW-075 | Every warning and suggestion can be dismissed or ignored without penalty. |

## 8. Audio behavior and quality targets

The implementation must prove the audio engine before expanding the UI.

- One `AudioContext` or equivalent master clock owns the transport.
- All lanes are sample-time aligned to their source and compensate for decoder/provider delay recorded in the stem manifest.
- Starts and seeks should be perceptually simultaneous; the target is no more than 20 ms deck-to-deck error.
- Sustained playback drift should remain below 20 ms over two minutes, or the engine should correct it without an audible jump.
- Gain, EQ, mute, solo, and crossfader changes use short ramps to prevent clicks.
- The output graph must include headroom before the master. Clipping is reported, not silently hidden with arbitrary normalization.
- A selected loop must remain musically aligned over repeated passes.
- Pitch-preserving tempo matching should keep pitch within 10 cents across the supported planning range, initially ±8%. Wider ranges may be enabled after measurement.
- A synthetic click-track fixture and known-tone fixture are required for sync, loop, gain, meter, and pitch tests.

The playback prototype must explicitly choose between streaming media elements, buffered sources, an AudioWorklet, or a native audio engine. Multiple independent `<audio>` elements are not an acceptable synchronization architecture merely because they appear to start together.

## 9. Data contracts

The precise Rust and TypeScript types must be mirrored through serialized test fixtures. A conceptual minimum follows.

```ts
type StemKind = 'vocals' | 'drums' | 'bass' | 'other';

interface BeatGrid {
  schemaVersion: 1;
  source: 'engine' | 'manual' | 'imported';
  bpm: number;
  firstBeatMs: number;
  meterNumerator: number;
  downbeatOffsetBeats: number;
  confidence: number | null;
}

interface StemManifest {
  schemaVersion: 1;
  trackId: number;
  sourceFingerprint: string;
  provider: string;
  model: string;
  modelVersion: string;
  createdAt: string;
  durationMs: number;
  alignmentOffsetMs: number;
  files: Record<StemKind, string>;
}

interface TransitionPlan {
  schemaVersion: 1;
  transitionId: string;
  masterBpm: number;
  tempoMaster: 'a' | 'b' | 'custom';
  overlap: { startBeat: number; lengthBeats: number };
  deckA: DeckPlan;
  deckB: DeckPlan;
  crossfader: AutomationPoint[];
  notes?: string;
}
```

Track-level beat-grid corrections and stem manifests belong to the catalog/cache. Transition-specific anchors, loops, mixer state, and automation belong to the persisted mix. The existing playlist `rules` JSON may hold a versioned transition plan for the first vertical slice; dense automation should move to normalized tables rather than turn one JSON field into an unbounded event store.

## 10. Technical boundaries

### Workbench frontend

Suggested components:

- `TransitionWorkbench`
- `TransitionHeader`
- `MusicalTimeline`
- `DeckLanes`
- `StemLane`
- `BeatGridEditor`
- `LoopRegion`
- `TransitionMixer`
- `LevelMeter`
- `StemPreparationStatus`

React components own presentation and interaction state. A dedicated playback controller owns the audio graph, scheduling, drift measurement, and teardown. High-frequency playhead/meter updates must not cause the entire React tree to rerender.

### Rust services

- `media/` owns source decoding, optional external tool/provider detection, and safe derivative paths.
- `engine/` owns beat/downbeat/phrase features and waveform data, but not UI or SQL.
- `catalog/` owns versioned beat-grid corrections, stem manifests, jobs, and cache bookkeeping.
- `commands/` remains a thin registered adapter.
- `harmony/` remains the only relationship vocabulary; this feature must not introduce another key/relationship implementation.

### Separation provider boundary

Use a provider interface rather than coupling TuneLock directly to Demucs or StemDeck:

```rust
trait StemProvider {
    fn identity(&self) -> ProviderIdentity;
    fn availability(&self) -> ProviderAvailability;
    fn separate(&self, request: SeparationRequest, events: JobEvents)
        -> Result<StemManifest>;
    fn cancel(&self, job_id: &str) -> Result<()>;
}
```

The first experimental provider should detect a user-installed local tool or separately running local service. It must use a persistent worker if model startup materially affects repeated jobs, serialize GPU-heavy work by default, support cancellation, and write into a temporary directory before atomically publishing a complete manifest.

### Command contract

Likely commands include:

- `get_beat_grid`
- `save_beat_grid_override`
- `reset_beat_grid_override`
- `get_transition_plan`
- `save_transition_plan`
- `get_stem_status`
- `start_stem_separation`
- `cancel_stem_separation`
- `delete_generated_stems`
- `get_stem_waveforms`

Names are provisional until the Rust types are designed. No TypeScript `invoke(...)` wrapper may land before the corresponding Rust command is implemented and registered in the same change.

## 11. Storage and lifecycle

- Originals are never modified, moved, renamed, or deleted.
- Generated stems live under TuneLock-managed application data or a user-selected cache directory, never beside originals by default.
- Use lossless compressed storage when the provider and playback path support it. Four uncompressed stems from a typical song can consume hundreds of megabytes.
- Default generation is on demand for tracks used in a selected transition. There is no automatic whole-library separation.
- Cache policy supports a user-defined quota, least-recently-used cleanup, project pins, per-track deletion, and a visible storage summary.
- Cache cleanup is recoverable by regeneration and must never remove a source file.
- Source fingerprint changes mark a manifest stale rather than silently associating old stems with new audio.

## 12. Failure and degraded states

| Condition | Required behavior |
|---|---|
| No stems/provider | Show full-mix lanes and all standard transport, grid, loop, EQ, crossfader, and meter functions. |
| Separation queued/running | Continue full-mix planning; show compact progress and allow cancel. |
| Separation failure | Preserve full-mix plan, remove partial files, show cause and retry path. |
| GPU failure | Notify the user and retry CPU only when safe and configured. |
| Low-confidence beat grid | Label it and expose manual correction prominently. |
| Missing source file | Disable playback without discarding the saved transition plan. |
| Stale stem cache | Use Original playback; offer explicit regeneration. |
| One corrupt stem | Treat the stem set as incomplete; do not silently sum three of four files. |
| Excessive playback drift | Warn, stop cleanly if necessary, and never save a misleading preview. |
| Unsupported model/license | Keep provider unavailable; do not download around the restriction. |

## 13. Accessibility and keyboard workflow

- Every transport, grid, loop, mute, solo, gain, EQ, and crossfader action is keyboard reachable.
- Space toggles transport only while the workbench has focus and no text field is active.
- Suggested shortcuts: `M` mute focused lane, `S` solo, `L` loop, number keys for loop length, arrow keys to nudge the beat grid with modifiers for coarse/fine movement.
- All controls have visible focus and accessible names; waveform meaning is also available through textual track/section information.
- Controls do not rely on color alone for state.

## 14. Acceptance criteria for the first releasable vertical slice

The first release is acceptable only when all of the following are demonstrated:

1. A selected Mix Canvas transition opens two correct tracks in the expanded workbench.
2. Both cached full-mix waveforms render aligned to a shared ruler and remain interactive at 60 FPS.
3. The DJ can correct and persist both beat grids, set a phrase-aligned overlap, and create a quantized loop.
4. Shared play/pause, seek, loop, tempo match, crossfader, deck EQ kills, and deck/master meters work without stems.
5. Measured start error and two-minute drift meet the targets in section 8 on reference hardware.
6. The complete transition state survives application restart and mix reload.
7. When a provider is available, the DJ can prepare four stems in the background, cancel safely, and see ready lanes without reopening the mix.
8. Stem gain, mute, solo, and Original/Stems A/B work during synchronized playback without clicks or accidental double playback.
9. Provider absence or failure never blocks full-mix planning and never damages the original.
10. Stem cache contents can be inspected and removed from TuneLock without touching source media.
11. Key/BPM readout latency is unchanged because separation and stem loading are off the critical path.
12. Every new frontend command maps to a registered Rust command, with a contract test preventing phantom wrappers.
13. Windows CPU operation is supported for the standard workbench; accelerator-specific stem support degrades explicitly.
14. Installer contents contain no unapproved downloader, GPL/AGPL code, or model weights with unclear redistribution rights.

## 15. Validation plan

### Product validation

Test with 5–10 working DJs using representative transitions from house/techno, hip-hop, pop/open-format, drum and bass, and tracks with live or drifting timing.

Each DJ should complete these tasks without coaching:

- identify a vocal collision;
- construct a bass handoff;
- loop a clean phrase;
- correct a deliberately offset beat grid;
- compare Original with Stems;
- save, close, reopen, and reproduce the transition.

Success target: at least 80% task completion, with most participants reporting that the view changes or increases confidence in at least one transition decision. Record qualitative artifact tolerance; separation benchmark scores alone do not predict DJ usefulness.

### Engineering validation

- Synthetic click tracks at several BPMs for grid and drift tests.
- Known sine tones for pitch-preserving stretch verification.
- Silence, clipping, corrupt-file, missing-file, and variable-duration fixtures.
- Short, long, VBR, video-demuxed, and non-4/4 material.
- CPU-only Windows plus available NVIDIA hardware; macOS/MPS when hardware is available.
- Repeated prepare/cancel/retry, process crash, application restart, and cache-quota tests.
- A 20-track listening corpus with leakage/artifact notes for each stem and provider/model version.

## 16. Delivery sequence

### Slice A — Playback and beat-grid proof

- Build a non-production two-track audio prototype with one master clock.
- Prove seek, loop, pitch-preserving tempo change, meters, start accuracy, and sustained drift using fixtures.
- Decide the playback architecture before building the final component hierarchy.

**Gate:** sync and loop targets are measured, not inferred from visual playback.

### Slice B — Standard full-mix Transition Workbench

- Replace the dual-audition footer with the expanded/resizable workbench.
- Add aligned three-band waveforms, grid editing, cues, loops, transport, crossfader, EQ kills, meters, and persistence.
- Keep the original three-band waveform path and current transition reasoning.

**Gate:** useful and releasable without any stem provider.

### Slice C — Optional four-stem provider and cache

- Implement provider discovery, job queue, progress, cancel, failure cleanup, manifest, cache, quota, and deletion.
- Begin with a user-installed provider; do not bundle model assets.
- Benchmark representative tracks and hardware.

**Gate:** licensing/provenance recorded and no effect on first-result latency.

### Slice D — Stem lanes and transition recipes

- Add four aligned lanes per deck, per-stem meters/gain/mute/solo, Original/Stems comparison, and saved recipes.
- Add limited crossfader/stem gain automation and preview rendering only after interactive playback is stable.

**Gate:** reopening a saved mix reproduces the planned transition.

### Slice E — Intelligent overlays

- Add vocal, bass, drums, energy, phrase, and local-key evidence.
- Generate suggestions from deterministic signals first; LLM language remains optional and off the audio path.

**Gate:** DJs find the overlays actionable and false warnings can be ignored easily.

## 17. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Audio streams drift | Prove a master-clock architecture first; measure with click fixtures and stop on intolerable drift. |
| Beat grid is mistaken for BPM | Store phase/downbeat separately, show confidence, and make manual correction fast. |
| Stem artifacts mislead the DJ | Keep Original A/B, record provider/model provenance, and avoid certainty language. |
| Runtime and installer become too large | Start with an external provider and on-demand model use. |
| Model/code/weight licenses differ | Maintain a dependency and model bill of materials; require explicit commercial redistribution approval for each asset. |
| Cache consumes terabytes | Four stems, on-demand jobs, lossless compression, quota, LRU cleanup, and project pinning. |
| Scope expands into a DAW | Hold non-goals; every addition must improve transition planning directly. |
| UI becomes visually noisy | Shared timeline, fixed lane order, restrained stem tints, progressive disclosure, Camelot priority. |
| New analysis slows normal results | Run beat refinement and separation in background; preserve local-result-first performance tests. |
| Provider crashes or GPU runs out of memory | Isolated worker, one heavy job at a time, watchdog, cancellation, partial cleanup, explicit CPU fallback. |

## 18. Handoff decisions

These decisions are binding for the first implementation proposal:

- The feature lives inside Mix Canvas and is the standard selected-transition view.
- Full-mix operation ships before stem separation and remains permanently supported.
- The stem vocabulary is four sources: vocals, drums, bass, and other.
- Separation is explicit, background, cached, local, and non-destructive.
- The first provider is external/user-installed unless redistribution rights are proven.
- Beat-grid manual correction is required, not a later polish item.
- One master transport replaces independent deck playback.
- Transition settings persist with the mix and are versioned.
- The product remains a mix planner, not a live DJ deck or DAW.

## 19. Open product choices with recommended defaults

| Choice | Recommended default |
|---|---|
| User-facing name | **Transition Workbench** |
| Entry point | Select a transition; workbench expands automatically |
| Initial stems | Vocals, drums, bass, other |
| Default phrase length | 16 bars, with 8/32-bar choices |
| Default sync range | ±8% with pitch preservation |
| Default crossfader curve | Equal power |
| Default stem policy | On demand for selected tracks only |
| Default storage policy | User-set quota with LRU cleanup; current-project stems pinned |
| First provider | User-installed local provider behind `StemProvider` |
| Preview export | Defer until interactive reproduction and licensing behavior are proven |

