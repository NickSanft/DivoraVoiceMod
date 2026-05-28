# Divora — Implementation Plan

A free, open-source, Windows-only real-time voice modulation app. Voicemod alternative.

---

## Vision

A voice modulator that respects users. Local-first, no account, no telemetry, no upsells. Modular DSP effect chains for live mic processing, a folder-based soundboard, push-to-modulate hotkey, and (eventually) on-device AI voice conversion. Free, MIT-licensed, public repo.

## Goals (v1.0)

1. Real-time mic effects (pitch, formant, reverb, EQ, compressor, robot, distortion, delay, noise gate) with sub-30 ms end-to-end latency on consumer hardware.
2. 15+ bundled "persona" presets (Deep Narrator, Helium, Robot, Phone Call, Stadium PA, Underwater, Demon, Chipmunk, etc.) with user-editable JSON.
3. Folder-backed soundboard: pick a folder, get clickable tiles, optional hotkeys per tile, mixed into output.
4. Self-monitoring (sidetone) so the user can hear themselves while tuning effects.
5. Output to a virtual audio device (VB-Cable) so other apps see the modulated voice as their microphone.
6. Push-to-modulate global hotkey: hold to apply chain, release to bypass.
7. Robust automated test suite + GitHub Actions for build / release.
8. MSI/NSIS installers on every tagged release.

## Non-goals (explicit)

- Cross-platform support in v1 (Windows only).
- Named celebrity voice clones (legal risk; Morgan Freeman has publicly objected to AI clones). Persona archetypes only.
- Cloud features, sign-in, telemetry.
- Custom virtual audio driver (defer indefinitely; using VB-Cable instead).
- VST plugin export. The app is a host, not a plugin. (Future: VST3 *host* support for advanced users.)

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
   │  └─ Optional AI conversion   │  (Phase 8+)
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
| Shell | Tauri 2.x | Small binaries (~10 MB), webview UI without Electron's footprint, mature Rust IPC, MSI/NSIS bundlers built in |
| Backend | Rust (stable) | No GC pauses on audio path, strong type system, good crate ecosystem for DSP |
| Audio I/O | `cpal` | Cross-platform-ready WASAPI backend, simple input/output stream API |
| DSP primitives | `fundsp`, `biquad`, `rubato`, `realfft` | Composable filter graph + battle-tested individual blocks |
| Audio file I/O | `hound`, `symphonia` | WAV write/read (tests), MP3/OGG/FLAC decode (soundboard) |
| Persistence | `serde` + JSON | Human-readable preset files |
| Hotkeys | `tauri-plugin-global-shortcut` | Cross-app Windows hotkeys via official plugin |
| Logging | `tracing` + `tracing-subscriber` | Structured logs to local file only |
| Frontend | SolidJS + TypeScript + Tailwind | Fine-grained reactivity (good fit for live meters), tiny runtime |
| Frontend tests | Vitest | Standard for Vite-based frontends |
| E2E tests | Playwright | Industry standard; works with Tauri via webdriver |
| Package manager | pnpm | Already installed; lockfile-friendly |

## SDLC Workflow

This project uses the per-phase ship workflow established on prior projects. Every phase follows the exact same loop:

1. **Implement** — narrow scope, one commit-cycle's worth.
2. **Tests** — unit (DSP blocks, pure helpers — exhaustively) + e2e Playwright (smoke for UI flows). Skip e2e for things adequately covered by unit tests; document the skip in the CHANGELOG.
3. **Pre-push checklist** — ALL must be green before push:
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --workspace`
   - `pnpm typecheck`
   - `pnpm test` (Vitest)
   - `pnpm test:e2e` (Playwright)
   - `cargo tauri build --debug` (confirms bundler still works)
4. **Commit** — detailed multi-line message via HEREDOC, ending with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
5. **Push to main**.
6. **Watch CI** in the background via `gh run watch <id> --exit-status`.
7. **Tag annotated `vX.Y.Z`** — ONLY after CI is green. Never before.
8. **Roll into next phase** without prompting.

Auxiliary rules:

- **CHANGELOG entry per phase** with: Added, Why it matters, Architecture, UX details, Tests, Bundle size, Pre-push checklist results.
- **Document partial work explicitly** — when shipping cosmetic or first-pass implementations, list remaining limitations.
- **Plan before implementing for multi-feature asks** — return a phase breakdown FIRST, wait for approval.
- **Pre-1.0 versioning: minor = phase number, patch = bugfixes within phase.** Phase 1 → v0.1.x, Phase 2 → v0.2.x. After Phase 1.0 cut, full semver applies.

## Phase Breakdown

| Phase | Version | Outcome | Done-when |
|---|---|---|---|
| 0 | v0.0.0 | Scaffold: repo, CI, blank Tauri shell, docs, license | `cargo tauri dev` opens an empty Divora window; CI passes; v0.0.0 tag annotated |
| 1 | v0.1.0 | Audio passthrough: mic → DSP graph (empty) → output, monitor toggle, device pickers | Hear yourself with sub-30 ms latency in headphones |
| 2 | v0.2.0 | First effects: pitch + EQ + reverb + compressor with live params | Sliders adjust live without glitches; golden-WAV tests pass |
| 3 | v0.3.0 | Preset system: JSON load/save, 5 bundled presets, switcher UI | Switching presets glitch-free; user can save custom |
| 4 | v0.4.0 | Soundboard: folder picker, tile grid, click-to-play, mixed into output | Sound from folder plays while modulated mic active |
| 5 | v0.5.0 | Virtual mic routing: VB-Cable detection, first-run wizard, output device picker | Discord call receives modulated voice |
| 6 | v0.6.0 | Push-to-modulate: global hotkey, configurable key, bypass logic | Holding Right Alt applies effects, releasing bypasses |
| 7 | v0.7.0 | Polish: 15 presets, level meters, settings persistence, signed installer, A/B compare | Public v0.7 release advertised as "feature complete v1" |
| 7.x | v0.7.x | Bug fixes from initial user feedback | All critical issues closed |
| 8 | v0.8.0 | RNNoise noise suppression | Cheap mics sound dramatically cleaner |
| 9 | v0.9.0 | AI voice conversion via LLVC | "Deep Narrator AI" works real-time on CPU |
| 1.0 | v1.0.0 | Cut after at least one quiet 0.7.x patch cycle. Stamps the stable surface | API/preset format stable for back-compat going forward |

After v1.0, additional features (VST3 host, Stream Deck integration, OBS WebSocket, etc.) follow standard semver.

## DSP Effect Set (Phase 2 → Phase 7)

| Effect | Algorithm | Crate / notes |
|---|---|---|
| Pitch shift | Phase vocoder or PSOLA | `rubato` for resampling, custom phase vocoder |
| Formant shift | LPC envelope warping | Custom; sits before pitch shift to keep voice natural |
| EQ | 5-band parametric (biquad peak/low-shelf/high-shelf) | `biquad` crate |
| Reverb | Freeverb (Schroeder + comb/allpass) | Custom; ~80 lines of Rust |
| Compressor | Standard feed-forward, soft knee | Custom |
| Distortion | tanh / soft-clip + bit-crush | Custom |
| Delay / echo | Circular buffer + feedback | Custom |
| Robot vocoder | Ring modulator + bandpass bank | Custom |
| Noise gate | Hysteresis-gated linear gain | Custom |
| Noise suppression | RNNoise FFI | Phase 8; `rnnoise-c` bindings |
| Voice conversion | LLVC ONNX | Phase 9; `ort` (ONNX Runtime Rust bindings) |

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
  "id": "deep-narrator",
  "name": "Deep Narrator",
  "description": "Movie trailer guy.",
  "icon": "microphone-deep",
  "version": 1,
  "chain": [
    { "type": "pitch_shift", "semitones": -4.0 },
    { "type": "formant_shift", "factor": 0.85 },
    { "type": "eq", "bands": [
      { "freq": 80,   "gain": 3.0, "q": 0.7 },
      { "freq": 2500, "gain": 2.0, "q": 0.8 }
    ]},
    { "type": "compressor", "threshold_db": -18, "ratio": 4, "attack_ms": 5, "release_ms": 100 },
    { "type": "reverb", "room_size": 0.4, "damping": 0.6, "wet": 0.15 }
  ]
}
```

Bundled defaults live in `presets/` at the repo root and are embedded into the binary via `include_str!`. On first run they're written to `%APPDATA%\Divora\presets\` so users can edit or duplicate them. User-created presets get a `"user": true` field so app updates can refresh bundled defaults without overwriting user changes.

## Soundboard

- Tauri command opens the folder picker.
- Background indexer (`walkdir` + `symphonia` metadata) builds a tile list, cached to `%APPDATA%\Divora\soundboard-cache.json`.
- Each tile = filename (label override available), color, icon, optional global hotkey.
- Click or hotkey → decode → push samples into the DSP mixer → mixed with modulated mic in output.
- Polyphony cap: 16 simultaneous clips.
- `Esc` = panic button (stops all clips).
- Soundboard audio routed to the same output as the modulated mic (so others hear it) and optionally to the monitor.

## Push-to-modulate

- `tauri-plugin-global-shortcut` registers a Windows-level hotkey. Default: `Right Alt`.
- DSP worker checks an `AtomicBool` at the top of each buffer; when false, the effect chain is bypassed (raw mic passes through).
- Inverted mode ("hold to bypass") configurable in settings — for users who want effects always-on except when they hold the key.
- Hotkey customization via a "press a key" capture UI.

## Virtual Mic Strategy

- First-run wizard:
  1. Enumerate audio devices.
  2. Look for `CABLE Input (VB-Audio Virtual Cable)`.
  3. Missing → show a card with one-paragraph explanation + button "Open VB-Cable download page" linking to https://vb-audio.com/Cable/. **Do not redistribute** the installer (VB-Audio's license is donationware with redistribution caveats).
  4. Present → auto-suggest CABLE Input as the modulated output target.
- Users instructed: "In Discord/Zoom/OBS/etc., set your mic to **CABLE Output (VB-Audio Virtual Cable)**."
- App provides a "Test in Discord" hint card with screenshots of common-app mic settings.

## Testing Strategy

### Unit tests (Rust)

- Every DSP block has a golden-WAV test: load input WAV, run effect, compare output to expected WAV within an f32 epsilon. Regenerate goldens with `--bless`.
- Property tests via `proptest` for buffer-handling code (varying sizes, edge cases).
- Preset (de)serialization round-trip tests.
- Soundboard indexer tests on a fixture directory.

### Latency benchmark

- `tests/integration/latency.rs`: null-device engine, sine input, measure samples-to-first-output. CI fails if > 60 ms.

### Frontend tests (Vitest)

- Reactive stores: preset switching state, hotkey capture state, device list.
- Component snapshot tests for `EffectCard`, `LevelMeter`, `DevicePicker`.

### E2E tests (Playwright via Tauri webdriver)

- Smoke: launch app, change preset, click soundboard tile (with mocked audio engine).
- Skipped for purely DSP logic that has unit coverage.

### Manual checklist (`docs/MANUAL_TESTS.md`)

- Pre-release: Discord call test, Zoom test, OBS feed test, soundboard during call, monitor toggle, push-to-modulate during all of the above.

## CI/CD

### `.github/workflows/ci.yml`

Triggers: PRs and pushes to `main`.

Jobs:
- `lint`: `cargo fmt --check`, `cargo clippy -- -D warnings`, `pnpm lint`
- `test-rust`: `cargo test --workspace` (Windows runner)
- `test-frontend`: `pnpm test`, `pnpm test:e2e`
- `build`: `cargo tauri build --debug`, uploads installer artifact

### `.github/workflows/release.yml`

Triggers: pushes of tags matching `v*.*.*`.

Jobs:
- `build`: `cargo tauri build` (release) on `windows-2022`
- `sign`: if `CODE_SIGNING_CERT` secret present, sign MSI/NSIS via `signtool`
- `release`: create GitHub Release, attach signed installers, update `latest.json` for `tauri-updater`

## Code Signing

- **Phase 0–7:** unsigned. README explains SmartScreen warning + "More info → Run anyway" workaround.
- **Phase 1.0+:** consider purchasing an EV cert (~$200/year via Sectigo or DigiCert) to bypass SmartScreen entirely.
- Until then, all installers carry SHA256 checksums on the Release page.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| WASAPI latency varies across hardware | Build latency benchmark in Phase 1, test on 3+ machines, document realistic numbers |
| VB-Cable license disallows redistribution | Detect-and-prompt (not bundle) — chose this path explicitly |
| SmartScreen scares users away | Document workaround in README; add signing in Phase 1.0 |
| LLVC quality lower than commercial RVC | Frame as "free local AI"; advanced users can drop in their own ONNX |
| Sample-rate mismatches between devices | Force internal 48 kHz, resample at boundaries with `rubato` |
| GC-style hitches in audio path | Rust eliminates the language-level case; lint against allocation in audio thread (custom clippy lint or manual review) |

## Standout Features (Selected for v1)

- **Local-first / no account / no telemetry.** Featured prominently in README and UI.
- **Push-to-modulate hotkey.** Hold to be modulated, release to be yourself.

## Standout Features (Deferred to post-v1)

- VST3 plugin host inside the effect chain
- Stream Deck integration
- OBS WebSocket integration (scene triggers on effects)
- MIDI controller bindings for soundboard
- Community preset registry (GitHub-hosted JSON)
- A/B preset comparison (consider promoting to v0.7)
- 30-second rolling clip recorder

## Conventions

- **License:** MIT.
- **Branch model:** trunk-based on `main`. Short-lived feature branches only when a PR is warranted (rare for solo work; the per-phase ship workflow pushes directly).
- **Commit style:** subject line under 70 chars, blank line, paragraph(s) explaining the why, optional bullet list of notable items, sign-off with Co-Authored-By.
- **Tag style:** annotated tags with release notes lifted from CHANGELOG.
- **Code style:** rustfmt defaults; clippy strict (`-D warnings`); TypeScript strict mode.
