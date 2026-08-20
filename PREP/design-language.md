# TuneLock Design Language — Walnut Console

**Status:** Approved direction  
**Date:** 2026-08-20  
**Owner:** TuneLock  
**Related:** `PREP/proposal.md`, `PREP/transition-workbench-feature-spec.md`, plan file Part 7

## 1. Core principle

> **Character in the frame; precision in the display.**

TuneLock is a modern musical instrument housed inside a "Walnut Console." The controls are tactile and characterful — walnut, bronze, screws, bezels, engraved labels. The data plane — waveforms, tables, timing, compatibility, risk — is charcoal, crisp, and largely texture-free.

This is not a full vintage reskin. It is a hybrid: vintage instrument shell framing modern information density. The personality lives in the controls; the clarity lives in the readouts.

## 2. The three-level workspace

Mix Canvas is one continuous workspace with three levels of magnification. These are not disconnected modes — they are three views of the same saved mix state.

### 2.1 Set Map

The strategic view: "Where is this mix going?"

Shows only information that helps answer that question:
- Energy trajectory
- Key journey
- Tempo changes
- Active-layer density
- Vocal-presence regions
- Bass ownership
- Saved scenes and transitions
- Warnings (overcrowding, insufficient headroom)

This is TuneLock's clearest distinction from live DJ software.

### 2.2 Layer Lab

The exploratory view: "What sounds good together?"

Eight available source slots. Two to four typically audible. Each compact slot shows:
- Track, sample, loop, or stem-group name
- Musical role: foundation, drums, bass, vocal, harmony, texture, or FX
- Miniature waveform
- Playback position and beat phase
- Camelot key and source BPM
- Gain meter
- Mute, solo, loop, and launch state
- A/B/Master bus assignment
- Prepared-stems indicator
- Ready, queued, playing, or stopped state

Only the selected slot expands to show EQ, cue regions, detailed waveform, loop length, stems, and routing. One or two slots may be pinned open for comparison.

Stems nest inside their source slot — they do not consume four Layer Lab slots. A "4 stems" control expands vocals, drums, bass, and other beneath the selected source.

### 2.3 Transition Workbench

The precision view: "Exactly how should this transition work?"

This is the existing detailed workbench surface:
- Aligned waveforms
- Beat, downbeat, and phrase grids
- Loop and cue editing
- Shared transport
- Pitch-preserving synchronization
- A/B bus controls and crossfader
- EQ kills and meters
- Expandable stem lanes
- Transition automation
- Local harmonic, vocal, bass, and density analysis

The approachable two-source workflow remains intact. Advanced transitions can include a third or fourth source.

### 2.4 Navigation between levels

```text
┌──────────────────────────────────────────────────────────────────────┐
│ MASTER BRIDGE  124.0 BPM · 1 Bar Quantize · 4 Active · -6 dB Headroom│
├──────────────────────────────────────────────────────┬───────────────┤
│ SET MAP                                              │ CONTEXT       │
│ Intro ─ Build ─ Peak ─ Reset ─ Peak ─ Outro          │ INSPECTOR     │
│ energy, key, density and scene trajectory            │ compatibility │
├──────────────────────────────────────────────────────┤ risks          │
│ LAYER LAB — eight available source slots             │ suggested     │
│ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────┐         │ actions       │
│ │1 Drums  │ │2 Track  │ │3 Vocal  │ │4 FX │         │               │
│ │waveform │ │waveform │ │waveform │ │loop │         │               │
│ └─────────┘ └─────────┘ └─────────┘ └─────┘         │               │
│ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────┐         │               │
│ │5 Ready  │ │6 Empty  │ │7 Empty  │ │8 Empty│        │               │
│ └─────────┘ └─────────┘ └─────────┘ └─────┘         │               │
├──────────────────────────────────────────────────────┴───────────────┤
│ TRANSPORT · CUE · LOOP · CAPTURE SCENE · A/B BUSES · CROSSFADER     │
└──────────────────────────────────────────────────────────────────────┘
```

Selecting a transition expands the Transition Workbench in place. Selecting a scene reveals its layers. Zooming back out shows those same decisions as part of the set's larger story.

## 3. Semantic color system

The plan previously said Camelot colors should be the only saturated colors, but also called for Traktor-style multicolor waveforms. Those ideas conflict without an explicit semantic hierarchy. This system resolves it.

| Color role | Meaning | Saturation | Usage |
|---|---|---|---|
| **Camelot hue** | Harmonic identity | Highest | Key badges, wheel wedges, track key labels |
| **Waveform RGB** | Frequency content | Dark, less saturated | Waveform rendering only — bass/mid/high mapping |
| **Amber** | Queued, waiting, provisional | Medium | Launch-queued state, pending analysis |
| **Green** | Synchronized, prepared, safely active | Medium | Ready state, confirmed sync, successful preparation |
| **Red** | Clipping, failure, urgent risk | Medium-high | Clip indicators, errors, warnings that need action |
| **Cream/brass** | Selection, focus, labels, hardware trim | Low-medium | Selected slot frame, focus ring, engraved labels |
| **Bus A/B** | Distinguish by shape, position, lettering | — | Not another pair of loud colors |

### Rules

1. **Color is never the only indicator.** Labels, icons, patterns, and position reinforce every state.
2. **Camelot is the loudest saturated identity.** Key is always the strongest visual signal on screen.
3. **Waveform colors are darker and less saturated** than Camelot colors, so frequency information doesn't compete with key identity.
4. **Red is reserved for failure and risk.** It is not a decorative accent.
5. **Bus A/B distinction is structural**, not chromatic — position, lettering, and shape carry the distinction.

## 4. Walnut Console material language

The vintage language is applied according to the behavior of the control, not uniformly across every pixel.

| Control type | Visual language |
|---|---|
| **Launch pads** | Illuminated square hardware buttons |
| **Gain and EQ** | Knobs or short channel-strip faders |
| **Crossfade and automation** | Physical faders |
| **Mute, solo, bus routing** | Positive-state lamp buttons |
| **Confidence and headroom** | Analogue-style meters (needle or bar) |
| **Beat phase and fast peak levels** | Modern LED/bar meters (more readable than needles) |
| **Camelot wheel** | Brass-rimmed selector under glass — the signature object |
| **Arrangement** | Subtle tape-path and splice-marker language |
| **Scenes** | Cassette-memory or console-snapshot metaphor |

### Where walnut, bronze, and texture belong

- Framing modules and panels
- Control bezels and escutcheons
- Engraved labels and screws
- The Camelot wheel housing

### Where texture does NOT belong

- Behind waveforms
- Behind tables and dense data
- Behind small text
- In the data plane generally

The data plane stays charcoal, crisp, and largely texture-free. The frame has character; the display has precision.

## 5. Progressive disclosure

The mistake would be displaying eight complete decks, each with four stem lanes, EQ, transport, meters, and metadata — 32 visible audio lanes before any planning information appears.

Instead:

- **Set Map** shows trajectory, not individual waveforms.
- **Layer Lab** shows compact slots. Only the selected slot expands.
- **Transition Workbench** shows the full precision surface for the selected transition.
- **Stems nest inside source slots**, not as top-level lanes.

One or two slots may be pinned open for comparison. The rest stay compact until selected.

## 6. Interaction design

The fun comes from hearing the consequences of direct manipulation:

- Drag a track, cue region, sample, or stem into a slot
- Tap a pad to queue it at the next beat, bar, or phrase
- Drag a waveform to change the entry point
- Drag loop edges and feel them snap to the grid
- Route multiple layers to Bus A or B
- Move a crossfader and see spectral/headroom consequences update
- Mute bass in one source and watch "bass ownership" transfer to another
- Capture a successful combination as a scene
- Morph or audition between scenes
- Promote a discovery into the Set Map as a planned segment
- Undo every experiment safely

### Animation rules

- Subtle lamp fades, button depression, dial detents, and meter damping add delight
- Transport, launch, mute, and loop actions remain immediate
- **Decorative animation must never delay sound**

## 7. Musical intelligence display

Rather than one opaque compatibility score, the Context Inspector visualizes separate musical dimensions:

| Dimension | What it shows |
|---|---|
| Harmonic relationship | Compatible, mild tension, or conflicting |
| Beat and phrase alignment | How well layers align rhythmically |
| Bass conflict | How many active bass sources |
| Lead-vocal overlap | Duration and timing of simultaneous vocals |
| Transient density | Competing kicks, snares, hats |
| Spectral crowding | Low/mid/high occupancy |
| Available headroom | Predicted peak and short-term loudness |
| Energy contribution | Whether layers build, maintain, or release |
| Local-key changes | Key at selected regions, not just global |
| Beat-grid confidence | How trustworthy the grid is |

### Language examples

The program should say things like:
- "Two lead vocals overlap for 12 seconds."
- "Player 3 can own the bass if Player 1 low EQ is removed."
- "The percussion loop reinforces the offbeat."
- "This source enters two beats before the phrase boundary."
- "Four layers leave approximately 3 dB of headroom."

This leaves the creative decision with the DJ.

### Source roles

Each active source can be assigned a role:
- Foundation
- Drums
- Bass
- Lead vocal
- Harmony
- Texture
- Transition effect

Role-aware guidance is more meaningful than treating four full songs as equivalent blocks.

## 8. Scenes and arrangements

A **scene** captures a reproducible musical moment:

```text
master BPM
launch quantization
active players
source regions
loops
beat offsets
stem masks
gains and pan
bus routing
EQ state
automation
```

Scenes can be placed in a lightweight arrangement:

```text
Scene 1: Track A + percussion loop
Scene 2: Track A vocal + Track B drums
Scene 3: Track B + Track C texture
```

This bridges exploratory grid work and a planned set without becoming Ableton Live.

## 9. Design tokens

The theme is implemented as a CSS-token layer over the existing charcoal base. The charcoal data plane tokens remain unchanged; the Walnut Console tokens frame and surround them.

### Data plane tokens (charcoal, unchanged)

| Token | Value | Purpose |
|---|---|---|
| `--data-bg` | `#1a1a1e` | Waveform background, table background |
| `--data-surface` | `#222226` | Panels containing data |
| `--data-text` | `#e8e8ec` | Primary text on data surfaces |
| `--data-text-dim` | `#888892` | Secondary text |
| `--data-border` | `#333338` | Borders on data surfaces |
| `--data-grid` | `#2a2a2e` | Beat grid lines |

### Walnut Console frame tokens

| Token | Value | Purpose |
|---|---|---|
| `--walnut-base` | `#3d2b1f` | Primary wood tone |
| `--walnut-dark` | `#2a1e15` | Recessed wood, shadow areas |
| `--walnut-light` | `#4a3829` | Highlighted wood grain |
| `--bronze-face` | `#8b7355` | Brushed bronze faceplate |
| `--bronze-dark` | `#5c4a35` | Recessed bronze |
| `--brass-accent` | `#c9a96e` | Brass trim, Camelot wheel rim |
| `--brass-bright` | `#e8c87a` | Illuminated brass, selection |
| `--cream-label` | `#f0e6d2` | Engraved label text |
| `--lamp-amber` | `#d4a04c` | Queued/waiting lamp |
| `--lamp-green` | `#5c9c5c` | Active/ready lamp |
| `--lamp-red` | `#c45c5c` | Clip/error lamp |
| `--screw` | `#6b5b4a` | Screw heads, fasteners |

### Camelot color tokens (unchanged, highest saturation)

The 12 Camelot colors remain the most saturated elements in the interface. They are defined in `src/lib/harmony.ts` and mirrored in Rust. No design token overrides them.

## 10. Implementation approach

1. Define semantic design tokens (data plane + walnut frame)
2. Add Walnut Console instrument shell using those tokens
3. Replace wrapping Mix Canvas cards with Set Map
4. Add eight-slot Layer Lab, initially with two active players
5. Make Transition Workbench the expandable precision view
6. Add scene capture and multi-layer persistence
7. Add expandable stems inside source slots
8. Add multi-layer musical diagnostics and scene morphing

### Guardrails

- Theme adds zero latency to the key/BPM readout path
- Data text stays crisp — no texture behind dense tables
- Camelot colors remain the only saturated elements
- Color is never the only indicator
- Decorative animation never delays sound
- The charcoal base remains functional without the walnut frame

## 11. Identity statement

> **TuneLock is the planning desk where a DJ can see an entire set, experiment like a sampler performer, and inspect a transition like an audio engineer.**

The walnut-console language makes that environment inviting and memorable. The three-level workspace keeps it usable. The semantic color system keeps key identity dominant. The progressive disclosure keeps density manageable.
