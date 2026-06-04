# DivoraVoice — Implementation Plan

A free, open-source, Windows-only real-time voice modulation app. Voicemod alternative. Public repo at [NickSanft/DivoraVoiceMod](https://github.com/NickSanft/DivoraVoiceMod).

---

## Vision

A voice modulator that respects users. Local-first, no account, no telemetry, no upsells. Modular DSP effect chains for live mic processing, a folder-based soundboard, push-to-modulate hotkey, and (eventually) on-device AI voice conversion.

The visual identity is **"spellcraft for your voice"** — a calm utility tool (Linear/Raycast bones) wearing a dusk-lit arcane skin. The signature element is the Mixer's **spell circle**: the user's voice is a glowing core, and effects orbit it as sigil-nodes connected by threads of light.

## Goals (v1.0)

1. Real-time mic effects (pitch, formant, EQ, robot, distortion, echo, reverb, noise gate) with sub-30 ms end-to-end latency on consumer hardware.
2. 5+ bundled "persona" presets — **Hollow King, Static Wraith, Velvet Demon, Choir of Ash, Clean Passthrough** — with user-editable JSON. Bonus 2 user defaults: Deep Warden, Glass Oracle.
3. Folder-backed soundboard: pick a folder, get clickable tiles, optional hotkeys per tile, mixed into output.
4. Self-monitoring (sidetone) so the user can hear themselves while tuning effects.
5. Output to a virtual audio device (VB-Cable) so other apps see the modulated voice as their microphone.
6. Push-to-modulate global hotkey: hold to apply chain, release to bypass (or inverted "hold to bypass" mode).
7. **A/B preset compare** on the Mixer header.
8. **Glyph casting easter egg** — drag closed shapes on empty space to cast bound presets.
9. **Tweaks** user-facing in Settings → Appearance (Mystical level / Motion / Color mood / Accent / Texture).
10. Robust automated test suite + GitHub Actions for build / release.
11. MSI/NSIS installers on every tagged release.

## Non-goals (explicit)

- Cross-platform support in v1 (Windows only).
- Named celebrity voice clones (legal risk; Morgan Freeman has publicly objected to AI clones). Archetype personas only.
- Cloud features, sign-in, telemetry.
- Custom virtual audio driver (defer indefinitely; using VB-Cable instead).
- VST plugin export. (Future: VST3 *host* support for advanced users.)
- Third-party icon libraries — all icons are the custom Sigil set.
- Inter / Roboto / Arial fonts — explicitly forbidden by design.

## Architecture

```
   Physical Mic
        │
        ▼
   ┌──────────────┐
   │  Capture     │  WASAPI (shared first, exclusive opt-in for latency)
   └──────┬───────┘
          ▼
   ┌──────────────────────────────┐
   │  DSP Pipeline                │
   │  ├─ Effect chain (presets)   │
   │  ├─ Side-chain: soundboard   │
   │  └─ Optional AI conversion   │  (Phase 9)
   └──────┬────────────┬──────────┘
          │            │
          ▼            ▼
   ┌────────────┐ ┌──────────────────┐
   │ Monitor    │ │ Virtual Mic Out  │  → other apps see this as your mic
   │ (headphone)│ │ (VB-Cable Input) │
   └────────────┘ └──────────────────┘
```

### Audio threading model

- **Audio callback thread** (real-time, cpal-driven): zero allocations, no locks, no syscalls. Pulls from input device, pushes raw samples into an SPSC ring buffer, pulls processed samples from another ring buffer, writes to output device(s).
- **DSP worker thread** (high priority, not real-time): pulls from the input ring, runs the effect chain + soundboard mixer, pushes to the output ring. Owns mutable DSP state.
- **UI / IPC thread** (Tauri main): owns the in-memory preset model, mixes preset updates into atomically-swappable chain pointers. Never touches the audio thread directly.

State sharing between threads uses `triple_buffer` or atomic pointer swaps for whole-chain changes, and `AtomicF32`/`AtomicU32` for individual parameter sweeps.

### Internal audio format

- 48 kHz, mono, f32 samples internally. Resample at device boundaries with `rubato`.
- Buffer size: 256 samples internal (= ~5.3 ms). Device callback can be larger; ring buffer absorbs the difference.

## Stack

| Layer | Choice | Rationale |
|---|---|---|
| Shell | Tauri 2.x (frameless window, OS drag region on titlebar) | Small binaries (~10 MB), webview UI without Electron's footprint, MSI/NSIS bundlers built in |
| Backend | Rust (stable) | No GC pauses on audio path |
| Audio I/O | `cpal` | Cross-platform WASAPI backend |
| DSP primitives | `fundsp`, `biquad`, `rubato`, `realfft` | Composable filter graph + battle-tested individual blocks |
| Audio file I/O | `hound`, `symphonia` | WAV write/read (tests), MP3/OGG/FLAC decode (soundboard) |
| Persistence | `serde` + JSON | Human-readable preset files |
| Hotkeys | `tauri-plugin-global-shortcut` | Cross-app Windows hotkeys |
| Logging | `tracing` + `tracing-subscriber` | Structured logs to local file only |
| Frontend | SolidJS + TypeScript + Tailwind | Fine-grained reactivity (good fit for live meters), tiny runtime |
| Fonts | Bricolage Grotesque / Space Grotesk / Space Mono | Self-hosted in production |
| Frontend tests | Vitest | Standard for Vite-based frontends |
| E2E tests | Playwright (via Tauri webdriver) | Industry standard |

## Design System

Locked direction. Full spec in [`docs/mockups/README.md`](mockups/README.md); interactive reference in [`docs/mockups/prototype/Divora.html`](mockups/prototype/Divora.html); high-res renders in [`docs/mockups/screenshots/`](mockups/screenshots/).

### Color tokens (default "Dusk Violet" mood)

```
--bg          #07060d        behind window / stage
--surface-0   #0d0a16        app canvas
--surface-1   #14101f        panels, sidebar, titlebar
--surface-2   #1a1528        cards
--surface-3   #221b34        raised / hover
--surface-4   #2c2442        active / pressed, toggle track

--text-hi     #ECE7F8
--text-mid    #A99FC4
--text-lo     #6E6590
--text-dim    #4b4368

--indigo      #6D5BF0        brightened brand indigo for dark UI
--indigo-deep #4F46E5        true brand indigo (app icon)
--pink        #EC4899
--pink-deep   #DB2777        true brand pink (app icon)
--grad        linear-gradient(120deg, #4F46E5 0%, #7C5CF6 42%, #DB2777 100%)

--success     #34D9A0        emerald — armed / ok / no-telemetry affirmations
--warning     #E9B14C        candlelit gold — VB-Cable missing, caution
--danger      #F2567A        crimson-rose — muted, delete, panic
--info        #58C6F2        cyan — hints
```

Three additional moods (Ink+Candle, Midnight) swap via `data-mood` attribute on the root. Two additional accents (Abyssal, Ember) swap `--grad`.

### Type scale

| Role | Family | Sizes |
|---|---|---|
| Display | Bricolage Grotesque (400–800) | 34 / 24 / 19 / 16 px |
| UI body | Space Grotesk (400–700, tabular-nums on meters) | 14 / 13 / 12 px |
| Mono / eyebrows / hotkeys | Space Mono (400/700) | 12.5 / 11 / 9.5 px |

### Iconography

- Custom **Sigil** SVG component (`<Sigil name="…" size={n} />`), all stroked with `currentColor` on a 24×24 viewBox.
- ~40 named icons covering effects, nav, status, brand/identity, and utility (full list in mockups README).
- Special **DMark** component: the white "D" on the gradient rounded square; used in titlebar, sidebar, wizard, About.

### Components (the design system)

Port from `docs/mockups/prototype/divora/components.jsx`:

- **Button** — variants: primary (gradient + glow), secondary, ghost, danger (+ `.solid`); sizes sm / default / lg
- **IconButton** — 34px square; `active` = accent
- **Toggle** — 40×23 switch with gradient when on
- **Slider** — 4px track, gradient fill, 14px white thumb with halo; `bipolar` variant fills from center
- **Badge** — mono uppercase pill (default/accent/success/warning/danger/info)
- **Kbd** — keycap chip
- **Segmented** — small tab group; selected = surface-4 (or gradient with `accent`)
- **Vertical/Horizontal level meters** — emerald→gold→crimson fill with white peak-hold cap
- **Select / Device picker** — 42px field opening a styled dropdown
- **HotkeyCapture** — dashed field; click → "capturing" → captures keychord into Kbd chips
- **EmptyState** — dashed-ring glyph + display title + helper copy + optional action
- **Tooltip**, **Card**, **Panel** surfaces

### Window chrome

- Custom frameless desktop window, 1100×720 default, 900×600 minimum.
- **Titlebar** (40px, `--surface-1`, hairline bottom): DMark + "DivoraVoice" wordmark + live status pill (Clean / Modulated / Muted) + persistent "🔒 LOCAL · NO ACCOUNT" affirmation + Windows min/max/close. OS drag region.
- **Sidebar** (80px, icon rail): 4 nav items (Mixer / Board / Presets / Settings); active item gets a 3px gradient bar + glow. Footer: replay-first-run bolt button + green "Local-first · no telemetry" shield badge.

### Motion

Voice core breathes and glows when modulated; threads animate dash-flow; the orbit ring + outer ring + an orbiting spark slowly rotate; pulse rings emanate from the core; particles drift up when modulated. All durations scale by a `motion` factor (functional=0 / ambient=0.6 / rich=1). Honor `prefers-reduced-motion` by mapping it to "functional".

## SDLC Workflow

Every phase follows the **same per-phase ship loop**:

1. **Implement** — narrow scope, one commit-cycle's worth.
2. **Tests** — unit (DSP blocks, pure helpers, components — exhaustively) + e2e Playwright (smoke for UI flows).
3. **Pre-push checklist** — ALL must be green before push:
   - `cargo fmt --check`
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `cargo test --workspace --all-features`
   - `pnpm typecheck`
   - `pnpm test` (Vitest)
   - `pnpm test:e2e` (Playwright, once wired)
   - `pnpm tauri build --debug --no-bundle`
4. **Commit** — detailed multi-line message via HEREDOC, ending with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
5. **Push to main**.
6. **Watch CI** via `gh run watch <id>` and **explicitly verify** with `gh run view <id> --json conclusion`.
7. **Tag annotated `vX.Y.Z`** — ONLY after conclusion is literally "success". Never before.
8. **Roll into next phase** without prompting.

Auxiliary rules:

- **CHANGELOG entry per phase** with: Added, Why it matters, Architecture, UX details, Tests, Bundle size, Pre-push checklist results.
- **Document partial work explicitly** — when shipping cosmetic or first-pass implementations, list remaining limitations.
- **Pre-1.0 versioning: minor = phase number, patch = bug fixes within phase.** Phase 1 → v0.1.x.

## Phase Breakdown

| Phase | Version | Outcome | Done-when |
|---|---|---|---|
| **0** ✓ | v0.0.0 | Scaffold: repo, CI, blank Tauri shell, docs, license | Tagged 2026-05-28; CI green |
| **1** | v0.1.0 | **Design system + app shell**: tokens, fonts, Sigils, DMark, custom titlebar with status pill, 80px sidebar nav, four empty screens, base components, Tweaks foundation (data-mood / data-motion attrs), reduced-motion handling, state skeleton, mock data layer | Shell looks like the screenshots with empty content; switching nav works; Mystical/Motion/Mood tweaks change visuals; no audio yet |
| **2** | v0.2.0 | **Audio passthrough + real meters**: cpal capture/output, device enumeration, device pickers wired, monitor toggle, live IN/OUT meters, voice status pill driven by real audio | Hear yourself in headphones with sub-30 ms latency; meters move with input level |
| **3** | v0.3.0 | **Effect chain + spell circle**: all 8 effects (pitch, formant, EQ, robot, distortion, echo, reverb, noise gate), spell circle visualization, breathing core, selected-rune inspector, push-to-modulate card + global hotkey | Slider in inspector changes effect live without glitches; core breathes when modulated; threads light up for enabled effects |
| **4** | v0.4.0 | **Presets**: 5 bundled + 2 user defaults, browser/editor with drag-reorder chain cards, Export JSON modal, save/duplicate/delete, **A/B compare snapshots** | Switching presets is glitch-free; A/B toggle swaps between two stored states; user-saved presets survive app updates |
| **5** | v0.5.0 | **Soundboard**: folder picker, tile grid with search, per-tile hotkeys, progress rings on playing clips, panic "stop all" | Sound from folder plays while modulated mic is active; 8 simultaneous clips play without dropout |
| **6** | v0.6.0 | **Settings + virtual mic**: full Settings screen, Audio devices section, Virtual microphone with VB-Cable detection card, Discord/Zoom/OBS screenshot cards, HotkeyCapture rows, Glyph casting binding UI, Appearance with full Tweaks (Mystical / Motion / Mood / Accent / Texture), About | Discord call receives modulated voice; user can rebind any hotkey; Tweaks knobs change visuals live |
| **7** | v0.7.0 | **First-run wizard + glyph casting**: 4-step wizard with ceremonial panel, spark canvas + RDP shape recognizer + on-canvas reveal + bound preset switch | Fresh install shows wizard; drawing ▲ ▽ ▢ ◯ casts bound presets with sparks + "SPELL CAST" reveal |
| **7.x** ✓ | v0.7.1 | Bug fixes from initial user feedback (PTM Space steal, Sigil DOM sharing, #root height anchor) | All critical issues closed |
| **8** | v0.8.0 | **Soundboard polish + cast alignment**: glyph-cast trace alignment, tile drag-reorder + persistence, per-tile colors, recent-folders history, global soundboard hotkeys (per-tile, system-wide via existing tauri-plugin-global-shortcut) | Drag tiles to reorder; right-click a tile to pick its color; recent folders dropdown switches between soundboards; hotkey-bound clips fire even while DivoraVoice is unfocused |
| **9** ✓ | v0.9.0 | **DSP quality**: real pitch-preserving shifter (phase vocoder or PSOLA), formant warping via LPC envelope, rubato resampling at device boundaries so mismatched sample rates no longer hard-fail | Pitch slider preserves formants + tempo; formant slider doesn't pitch-shift; mic + output at different sample rates Just Works |
| **10** ✓ | v0.10.0 | **Polish + signed installer**: manual test pass, RNNoise noise suppression, code-signing cert (if budget), sand off rough edges | Public v0.10 release advertised as "feature complete v1" |
| **11** ✓ | v0.11.0–.5 | **Live device switching + cast polish + soundboard verification**: changing the input/output device while the engine runs should restart it cleanly; the glyph-cast trail gets sparks + a "SPELL CAST" preset-name reveal in the bound preset's colour; verify (and document) that soundboard clips reach the modulated output | Picking a new mic in Settings restarts the engine to that mic; drawing a triangle shows sparks + "◆ SPELL CAST ◆ Velvet Demon" before the preset takes effect; soundboard tile played mid-call is audible to call participants |
| **12** ✓ | v0.12.0–.4 | **AI voice conversion**: ONNX Runtime (`ort`) framework + `VoiceConvert` effect (v0.12.0); Voice library UI + background model loading (v0.12.1); DSP **Deep Narrator** that's audibly deep with no model (v0.12.2); **LLVC** (KoeAI, MIT) exported to ONNX + wired end-to-end (v0.12.3); installer bundles `onnxruntime.dll` + the LLVC narrator so AI works out of the box (v0.12.4) | Deep Narrator preset audibly transforms the voice via DSP today; with the bundled LLVC model the mic is converted to the narrator voice; effect degrades to passthrough (never hangs) when no model/runtime |
| **13** ✓ | v0.13.0 | **Monitor output routing** (v1.0 blocker): a second, independent output stream to a user-chosen **monitor device**, so the main output can route to VB-Cable (→ Discord/games) while the user still hears themselves on headphones. Adds a monitor-output device picker in Settings; the DSP runs once and fans out to both the main + monitor outputs; the soundboard mix is pre-fanout so clips are audible on the monitor too. The existing "monitor" toggle becomes "hear myself on the monitor device". | With output = VB-Cable + monitor = headphones, the user hears their modulated voice + soundboard clips on headphones while Discord receives them; toggling monitor mutes only the headphones, not the cable |
| **14** ✓ | v0.14.0 | **Latency transparency**: a live "added latency" readout summing the active DSP chain's fixed delays (Voice Convert ≈ 256 ms, denoiser 10 ms, pitch/formant STFT window 1024). `AudioEffect::latency_samples` + `EffectChain::latency_samples`, surfaced via the level event + shown in the Mixer header. (Buffer-size selector deferred — WASAPI shared-mode makes it unreliable.) | A live "+N ms latency" readout that moves the moment Voice Convert / denoiser / pitch toggle |
| **15** ✓ | v0.15.0 | **Soundboard + tray polish**: (1) per-tile + master soundboard volume; (2) persist the chosen soundboard folder across restarts (bug fix); (3) minimize to the Windows **system tray** so the app runs in the background while using Discord/games | Each clip + the board have a volume control that persists; reopening the app restores the last soundboard folder; closing/minimizing hides to a tray icon with restore + quit |
| **16** ✓ | v0.16.0 | **Record modulated output**: one-click capture of the post-chain output to WAV, with an indicator + a recordings folder. Reuses the existing output tap | Record → speak → stop yields a playable `.wav` of the modulated voice |
| **1.0** | v1.0.0 | **Stable surface cut** after a quiet 0.16.x cycle: no new features — freeze the preset JSON + Tauri command surface for back-compat, manual test pass, ship. 8 effects + soundboard + presets + cast + AI voice + monitor routing + latency + tray + recording | API/preset format stable for back-compat; public "v1" release |

After v1.0, features follow standard semver:

| Rel. | Version | Theme | Success criterion |
|---|---|---|---|
| post-1.0 ✓ | v1.1.0 | **The Coven (voice cast)** — a browsable cast of character voices (Velvet Demon, Hollow King, Choir of Ash, Static Wraith, The Oracle), each a tuned **DSP voice identity**, plus the LLVC narrator, unified under one "Coven" browser. *Hybrid step 1.* Curation lives in the frontend (`coven.ts`) over the bundled presets; "Summon" reuses `set_effect_chain` / `set_voice_model`. The `kind`/`modelId` seam covers model-backed voices for v1.4. | Open the Coven browser, click a character → your voice takes on that character live |
| post-1.0 ✓ | v1.1.1 | **Coven tuning + device persistence** (from feedback): more reverb on Velvet Demon, de-pitched Static Wraith, properly-deep Deep Narrator; input/output device selection now persists across restarts. | Devices survive a restart; the tuned voices match their names |
| post-1.0 ✓ | v1.2.0 | **Coven ensemble — chorus effect**: a new `chorus`/doubler DSP effect (modulated multi-tap delay summed with dry) so **Choir of Ash** reads as many voices, not one. Additive `EffectKind::Chorus`. | Choir of Ash sounds like an ensemble; chorus is selectable in the chain editor |
| post-1.0 ✓ | v1.3.0 | **Streaming LLVC (low-latency AI)**: re-export LLVC in streaming mode (thread the encoder/decoder/convnet cache tensors between chunks), carry per-instance state through `run_inference`, shrink the chunk — so the AI voices are usable in live conversation, not just recordings | Voice Convert latency drops from ~256 ms toward <60 ms with no audible seams (verified via the Phase 14 readout) |
| post-1.0 ✓ | v1.4.0 | **Coven expansion — the choir family**: four DSP voices built on the adjustable Harmonizer — **Seraph** (major), **Dirge** (minor + low toll), **The Swarm** (dissonant cluster), **The Possessed** (octave-down doubling). No new effects/model; bundled presets + cast entries. | The Coven browser shows 10 voices; each choir-family member is an audibly distinct chord |
| post-1.0 ✓ | v1.5.0 | **Voice pack — range + utility**: five more DSP voices over the existing effects — **Leviathan** (deepest), **The Imp** (comedic high), **Dispatch** (clean bandlimited comms), **Corrupted** (bit-crush + ring-mod glitch), **Whisper Wraith** (airy/intimate). Bundled presets + cast entries; no new effects/model. | The Coven browser shows 15 voices, each a distinct character |
| post-1.0 ✓ | v1.6.0 | **Presets preview-then-Use + monitor volume** (from feedback): selecting a preset only previews/edits it — your live voice changes only on **Use** (the viewed/active split); and the Mixer gains a **monitor volume** slider (0–200 %) to set how loud you hear yourself, persisted + applied to the monitor stream. | Browsing presets doesn't change your live voice until Use; the monitor slider boosts/cuts sidetone |
| post-1.0 ✓ | v1.7.0 | **Loudness normalization / auto-gain**: an optional RMS-target auto-gain stage + brick-wall limiter, applied as a **global post-chain output stage** (not a per-preset effect, so it stays level *across* presets), with a live makeup-gain readout. Zero added latency. | Switching presets or toggling Voice Convert keeps output loudness roughly constant without clipping |

**Deferred — zero-shot conversion + personal voice** (*hybrid step 2*): an any-to-reference VC model (convert toward a reference clip, no per-voice training) would unlock realistic Coven members from bundled public-domain / synthetic clips **and** a consented "your voice" target recorded in-app, plus the AI-onboarding/voice-catalog idea. **Blocked on the model landscape:** as of 2026-06 there is no real-time, on-CPU, *permissively-licensed* zero-shot VC model — the fast CPU option (RT-VC, ~61 ms) ships no license, and the permissive ones (kNN-VC, FreeVC; both MIT) depend on WavLM-Large (~1.3 GB, GPU-oriented), which is the real-time killer on CPU. OpenVoice v2 (MIT) is viable only as offline *record-and-convert*, which cuts against the live-changer identity. Revisit when a real-time, ONNX-friendly, permissively-licensed option matures (RT-VC is the one to watch).

Further out (standard semver): VST3 host, Stream Deck integration, OBS WebSocket, etc.

## DSP Effect Set (Phase 3)

| Effect | Algorithm | Parameters |
|---|---|---|
| Noise Gate | Hysteresis-gated linear gain | threshold −80…−20 dB (−52) |
| Pitch | Phase vocoder or PSOLA | shift ±12 st (0, bipolar) |
| Formant | LPC envelope warping | shift ±10 (0, bipolar) |
| EQ | 3-band biquad parametric | low/mid/high ±12 dB each (0, bipolar) |
| Robot | Ring modulator + bandpass bank | carrier 40–400 Hz (120); mix 0–100% (70) |
| Distortion | tanh soft-clip + bit-crush | drive 0–100% (35) |
| Echo / delay | Circular buffer + feedback | time 40–800 ms (240); feedback 0–90% (35) |
| Reverb | Freeverb (Schroeder + comb/allpass) | size 0–100% (40); mix 0–100% (25) |

Each effect implements the `AudioEffect` trait:

```rust
pub trait AudioEffect: Send {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32);
    fn set_param(&mut self, id: ParamId, value: f32);
    fn reset(&mut self);
}
```

## Preset Format

```json
{
  "id": "hollow-king",
  "name": "Hollow King",
  "description": "Resonant gravitas. Deep, slow, ceremonial.",
  "color": "#7C5CF6",
  "glyph": "crown",
  "version": 1,
  "chain": [
    { "type": "noise_gate", "enabled": true, "threshold_db": -48 },
    { "type": "pitch", "enabled": true, "semitones": -5 },
    { "type": "formant", "enabled": true, "shift": -3 },
    { "type": "eq", "enabled": true, "low_db": 3, "mid_db": -2, "high_db": 1 },
    { "type": "reverb", "enabled": true, "size": 0.78, "mix": 0.42 }
  ]
}
```

Bundled defaults live in `presets/` at the repo root and are embedded into the binary via `include_str!`. On first run they're written to `%APPDATA%\DivoraVoice\presets\` so users can edit or duplicate them. User-created presets get a `"user": true` field so app updates can refresh bundled defaults without overwriting user changes.

## Soundboard

- Tauri command opens the folder picker.
- Background indexer (`walkdir` + `symphonia` metadata) builds a tile list, cached to `%APPDATA%\DivoraVoice\soundboard-cache.json`.
- Each tile = emoji + filename (label override available), color, optional global hotkey, optional duration display.
- Click or hotkey → decode → push samples into the DSP mixer → mixed with modulated mic in output.
- Polyphony cap: 16 simultaneous clips.
- `Esc` = panic button (stops all clips).
- Playing tile: colored border + glow, emoji scales up, SVG progress ring draws from `stroke-dashoffset`, "PLAYING" mono badge + countdown.

## Push-to-modulate

- `tauri-plugin-global-shortcut` registers a Windows-level hotkey. Default: `Space`.
- Logic: `effectiveModulated = (ptmMode === "apply") ? pressed : !pressed`. Then `status = muted ? "muted" : (anyEffectEnabled && effectiveModulated) ? "modulated" : "clean"`.
- Settings expose both the bound key and the mode (apply / bypass).
- Right-rail card shows the bound key chip, mode segmented toggle, and a large "Hold to test" button (visual feedback as APPLYING / BYPASSED while held).

## Virtual Mic Strategy

- First-run wizard step 2:
  1. Enumerate audio devices.
  2. Look for `CABLE Input (VB-Audio Virtual Cable)`.
  3. Missing → warning card with one-paragraph explanation + primary "Download" button linking to https://vb-audio.com/Cable/. **Do not redistribute** the installer.
  4. Present → emerald check + "Re-scan" button.
- After wizard, the user is instructed to set their mic to **CABLE Output (VB-Audio Virtual Cable)** in Discord/Zoom/OBS. Settings section provides actual app screenshots.

## Glyph Casting (Phase 7)

A signature easter egg. Spec from `docs/mockups/README.md`:

- A pointer-events-disabled `<canvas>` overlay (z-index 58) covers the app.
- Left-click drag on **empty space** (not on UI controls) starts capturing pointer trail; each move emits a fading spark particle (canvas particle system with requestAnimationFrame).
- On pointer-up, the captured path runs through:
  1. **Ramer-Douglas-Peucker** simplification → polyline corner count
  2. **Radial uniformity** check for circles (distance variance from centroid)
- Recognized closed shapes:
  - ▲ Triangle (3 corners)
  - ▽ Inverted triangle (3 corners, peak down)
  - ▢ Square (4 corners)
  - ◯ Circle (low radial variance, no clear corners)
- On match → a **burst of sparks traces the shape**, the bound preset's name blooms in its color ("◆ SPELL CAST ◆"), and the app switches to that preset on the Mixer.
- Settings → Glyph casting lets the user re-bind each shape to any preset. Defaults: ▲→Velvet Demon, ▽→Glass Oracle, ▢→Hollow King, ◯→Clean Passthrough.

## Tweaks (Phase 6, exposed in Settings → Appearance)

User-facing visual variations, persisted to settings:

- **Mystical level** (subtle / balanced / rich) — controls spell-circle decoration density (outer ring, tick marks, constellation dots)
- **Motion** (functional / ambient / rich) — animation intensity (mapped from `prefers-reduced-motion` by default)
- **Color mood** (Dusk Violet / Ink+Candle / Midnight) — surface/line palette swap via `data-mood` root attribute
- **Accent** (Brand / Abyssal / Ember) — swaps `--grad`
- **Texture** (Parchment grain on/off, Vignette on/off) — overlay layers

## Testing Strategy

### Unit tests (Rust)

- Every DSP block has a golden-WAV test: load input WAV, run effect, compare output to expected WAV within an f32 epsilon. Regenerate goldens with `--bless`.
- Property tests via `proptest` for buffer-handling code (varying sizes, edge cases).
- Preset (de)serialization round-trip tests.
- Glyph recognizer tests (well-formed shapes, edge cases, noise).
- Soundboard indexer tests on a fixture directory.

### Latency benchmark

- `tests/integration/latency.rs`: null-device engine, sine input, measure samples-to-first-output. CI fails if > 60 ms.

### Frontend tests (Vitest)

- Reactive stores: preset switching state, hotkey capture state, device list, Tweaks state.
- Component tests for each design-system component.
- Spell-circle math: positioning of effect nodes around the orbit.

### E2E tests (Playwright via Tauri webdriver)

- Smoke: launch app, switch nav, change preset, click soundboard tile (mocked audio engine), draw a square and see the SPELL CAST reveal.

### Manual checklist (`docs/MANUAL_TESTS.md`)

- Pre-release: Discord call, Zoom, OBS feed, soundboard during call, monitor toggle, push-to-modulate during all of the above, glyph casting during a call (doesn't interfere with audio).

## CI/CD

### `.github/workflows/ci.yml`

Triggers: PRs and pushes to `main`.

Jobs:
- `frontend`: pnpm install, typecheck, Vitest
- `rust`: pnpm install + build (dist/ for Tauri macros), `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`
- `build`: `pnpm tauri build --debug --no-bundle` — runs after `frontend` and `rust` pass

### `.github/workflows/release.yml`

Triggers: pushes of tags matching `v*.*.*`.

Jobs:
- `release`: `pnpm tauri build` (release MSI + NSIS), `softprops/action-gh-release@v2` to create GitHub Release with auto-generated notes + signed installers attached.

## Code Signing

- **Phase 0–9:** unsigned. README explains SmartScreen warning + "More info → Run anyway" workaround.
- **Phase 10+:** consider purchasing an EV cert (~$200/year via Sectigo or DigiCert) to bypass SmartScreen entirely.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| WASAPI latency varies across hardware | Build latency benchmark in Phase 2, test on 3+ machines |
| VB-Cable license disallows redistribution | Detect-and-prompt (not bundle) — chose this explicitly |
| SmartScreen scares users away | Document workaround; add signing in Phase 10 |
| LLVC quality lower than commercial RVC | Frame as "free local AI"; advanced users can drop in their own ONNX |
| Sample-rate mismatches between devices | Force internal 48 kHz, resample at boundaries with `rubato` |
| Spell-circle animation performance | Use CSS transforms + opacity (GPU-composited), throttle particle count, expose Motion=functional fallback |
| Glyph recognizer false positives during real use | Require drag-from-empty-space; ignore drags inside UI bounds; require minimum path length |
| Custom Sigil set is ~40 icons to author | Port wholesale from `docs/mockups/prototype/divora/sigils.jsx` (no re-design) |
| Self-hosting Bricolage/Space Grotesk/Space Mono | Use `@fontsource` packages or commit WOFF2 directly; never fall back to Inter |

## Standout Features (Locked for v1)

- **Local-first / no account / no telemetry** — persistent UI affirmation in the titlebar.
- **Push-to-modulate** with configurable apply/bypass mode.
- **A/B preset compare** on the Mixer.
- **Glyph casting** — drag closed shapes to cast bound presets.
- **Tweaks** in Settings → Appearance (Mystical / Motion / Mood / Accent / Texture).
- **Spell circle** — distinctive Mixer visualization.

## Standout Features (Deferred to post-v1)

- VST3 plugin host inside the effect chain
- Stream Deck integration
- OBS WebSocket integration
- MIDI controller bindings for soundboard
- Community preset registry (GitHub-hosted JSON)
- 30-second rolling clip recorder
- Light theme ("soon" in design)

## Conventions

- **License:** MIT.
- **Branch model:** trunk-based on `main`. Short-lived feature branches only when a PR is warranted (rare for solo work).
- **Commit style:** subject line under 70 chars, blank line, paragraph explaining the why, bullet list for notable items, sign-off with Co-Authored-By.
- **Tag style:** annotated tags with release notes lifted from CHANGELOG.
- **Code style:** rustfmt defaults; clippy strict (`-D warnings`); TypeScript strict mode; tabular-nums on all numeric meters/readouts.

## Reference

- Full design spec: [`docs/mockups/README.md`](mockups/README.md)
- Interactive prototype: [`docs/mockups/prototype/Divora.html`](mockups/prototype/Divora.html)
- High-res screenshots: [`docs/mockups/screenshots/`](mockups/screenshots/)
- Architecture (as-built, grows per phase): [`docs/ARCHITECTURE.md`](ARCHITECTURE.md)
- Contributing guide: [`docs/CONTRIBUTING.md`](CONTRIBUTING.md)
- Pre-release manual test checklist: [`docs/MANUAL_TESTS.md`](MANUAL_TESTS.md)
