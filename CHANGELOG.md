# Changelog

All notable changes to Divora are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning is **phase = minor** until v1.0 (see [docs/PLAN.md](docs/PLAN.md)).

## [Unreleased]

## [1.3.0] — 2026-06-01 — Streaming AI voice conversion (low latency)

The AI voices are now usable in live conversation.

### Added

- **Streaming LLVC.** Re-exported LLVC in its streaming mode — the cache-tensor forward `(audio, enc_buf, dec_buf, out_buf, convnet_pre_ctx) → (output, …caches)` — and rewrote the Voice Convert effect to thread the four cache tensors + a 2·L front-context between tiny **208-sample (~13 ms)** chunks. Voice Convert latency drops from **~256 ms to ~13 ms**; the Mixer's "+N ms" readout reflects it.
- The effect **auto-detects the model contract**: a model exposing the cache inputs (`enc_buf` …) uses the low-latency streaming path; an older single-`audio`→`output` model falls back to the 256 ms stateless path. Both still degrade to passthrough when the ONNX runtime or model is absent — and the caches are zero-initialized (verified equivalent to the model's `None`-init).
- The bundled narrator is now the streaming export, hosted on the new **`voice-assets-v2`** release (v1 kept the non-streaming model so pre-1.3 tags still build). The reproducible exporter is committed at [`docs/voice-models/export_streaming_onnx.py`](docs/voice-models/export_streaming_onnx.py) — it validated the streaming ONNX against PyTorch to **max|diff| ≈ 2e-7** over an 8-chunk run.

### Tests

- **Rust**: the ignored LLVC integration test now threads the four caches through 8 streaming chunks against the real model (run locally with the runtime DLL — verified `mean|delta| ≈ 0.19`, full-length finite chunks, streaming detected). Unit + bypass tests unchanged; total unchanged.
- **Frontend**: unchanged (254) — no UI/IPC change.

### Notes

- No frozen-surface change: Voice Convert's UI (Mix) + the Deep Narrator preset/cast are identical — it's the same effect, just fast enough for live calls. Final latency + audio sign-off is a desktop check (see `docs/MANUAL_TESTS.md`).

### Pre-push checklist (local, 2026-06-01)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (+ streaming integration test verified locally with the DLL)
- `pnpm typecheck` — pass
- `pnpm test` — pass (254)
- `pnpm tauri build --debug --no-bundle` — pass

## [1.2.1] — 2026-06-01 — Choir as a chord + "In use" fix

Two fixes from feedback.

### Fixed

- **Choir of Ash now sings a chord.** v1.2.0's chorus still read as a single (high) voice — a chorus only detunes, it can't make harmony. Replaced it with a new **Harmonizer** that stacks pitched copies into an actual **diminished chord** (the dry root plus +3 / +6 / +9 semitones).
- **Presets "In use" badge no longer shows on every preset.** It compared the active preset to itself (always true), so every voice in the editor claimed "In use." It now appears only when the engine is actually running the active preset (nothing shows it while stopped).

### Added

- **Harmonizer effect** — sums the dry "root" with up to three independent phase-vocoder pitch voices. Defaults to a diminished stack (3 / 6 / 9 st), but each interval is adjustable (**Voice 1 / 2 / 3**), so it doubles as a general 3-voice harmonizer (e.g. 4 / 7 for a major triad). **Mix** sets the chord level; the dry path adds no latency. Additive `EffectKind::Harmonizer`.

### Tests

- **Rust**: 137 → 141 (+4 harmonizer: passthrough at mix 0, adds chord energy once warm, DC stays finite, zero added latency; the Choir-of-Ash lock now asserts the harmonizer).
- **Frontend**: 254 (catalog + Choir tests updated chorus → harmonizer).

### Pre-push checklist (local, 2026-06-01)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (148 across crates, +1 ignored LLVC)
- `pnpm typecheck` — pass
- `pnpm test` — pass (254)
- `pnpm tauri build --debug --no-bundle` — pass

## [1.2.0] — 2026-06-01 — Coven ensemble: the chorus effect

Choir of Ash finally sounds like a choir.

### Added

- **Chorus effect** — a new DSP effect: several short, LFO-modulated delay taps summed with the dry signal. The continuously-varying delays detune each tap slightly, so one voice reads as an ensemble ("many voices from one"). Selectable in any chain with **Mix** + **Depth** params; it's a wet tail on the dry path, so it adds no latency. (`EffectKind::Chorus` — an additive addition to the frozen effect set.)
- **Choir of Ash rebuilt** around it: the pitch is eased (+5 → +2 st, less chipmunk) and a chorus is layered in, so it now reads as a true ensemble instead of a single high voice — directly addressing the "only sounds like one voice" feedback.

### Tests

- **Rust**: 133 → 137 (+4: chorus is a passthrough at mix 0, produces delayed ensemble copies, adds zero latency; Choir of Ash layers an enabled chorus).
- **Frontend**: 252 → 254 (+2: chorus is in the effect catalog; Choir of Ash uses it).

### Notes

- Roadmap shifts out by one: **streaming LLVC → v1.3.0**, zero-shot + personal voice → v1.4.0, loudness → v1.5.0.

### Pre-push checklist (local, 2026-06-01)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (144 across crates, +1 ignored LLVC)
- `pnpm typecheck` — pass
- `pnpm test` — pass (254)
- `pnpm tauri build --debug --no-bundle` — pass

## [1.1.1] — 2026-05-31 — device persistence + Coven tuning

Bug fix + first pass of Coven voice tuning from real-world feedback.

### Fixed

- **Input & output device selections now persist across restarts.** They lived in non-persisted signals (only the monitor device was saved), so every launch reset to the host defaults. Both are now stored in `localStorage` and restored on startup; if a saved device is gone (unplugged since last run), it falls back to the host default instead of failing to start.

### Changed — voice tuning

- **Velvet Demon** — more reverb (size 30 → 48, mix 18 → 34) for a roomier, wetter menace.
- **Static Wraith** — dropped the upward pitch (+2 → −2) so it reads as an eerie specter instead of "just a higher-pitched voice."
- **Deep Narrator** — actually deep now: stronger pitch (−4 → −7) and formant (−3 → −5) shifts, so it lands low even when the AI conversion target sits higher than your voice.

### Tests

- **Frontend**: 249 → 252 (+3: device selections persist, restore on a fresh store, and fall back when a saved device is gone).

### Notes

- **Choir of Ash** still sounds like a single (high) voice — a real choir needs an ensemble/doubler, so the new **chorus effect + Choir rebuild lands in v1.2.0**. Hollow King and The Oracle were reported good and are unchanged.

### Pre-push checklist (local, 2026-05-31)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (140 across crates, +1 ignored LLVC)
- `pnpm typecheck` — pass
- `pnpm test` — pass (252)
- `pnpm tauri build --debug --no-bundle` — pass

## [1.1.0] — 2026-05-31 — The Coven (voice cast)

A curated cast of character voices, browsable in one place and summoned with a click.

### Added

- **The Coven** — a new **Coven** screen (sidebar, second slot) presenting Divora's character voices as a gallery. Each card shows the character's sigil (in its color), name, a **DSP** / **AI Voice** badge, and a lore blurb; **Summon** applies that voice live and highlights the active member. For the AI narrator, Summon also loads its conversion model — and when the model isn't installed, it falls back to the deep DSP voice with a clear hint (never hangs).
- **The Oracle** — a new fifth DSP character: calm, resonant, natural-pitch. The cast is now Velvet Demon · Hollow King · Choir of Ash · Static Wraith · The Oracle, plus the LLVC **Deep Narrator** (AI).
- A `summon(presetId, modelId?)` store action that applies a cast member's chain and sets (or clears) its conversion model in one step.

### Notes

- The cast curation lives in the frontend (`src/data/coven.ts`) as a thin layer over the existing bundled presets — **no new backend command and no preset-schema change**, so it stays entirely within the v1.0 frozen surface. The `kind: "dsp" | "model"` + `modelId` seam is in place so realistic, model-backed voices (the v1.3 zero-shot work) drop in as new entries.
- Per-character ear-tuning and a `chorus`/doubler effect for a lusher Choir of Ash are deferred to a v1.1.x refinement pass (the recipes already work; tuning is best done by ear).
- Roadmap reordered: **v1.1 Coven → v1.2 streaming LLVC → v1.3 zero-shot + personal voice**.

### Tests

- **Rust**: divora-core 132 → 133 (+1: locks The Oracle's calm/resonant character).
- **Frontend**: 242 → 249 (+7: cast-data integrity ×5, `summon` behavior ×2).

### Pre-push checklist (local, 2026-05-31)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (140 across crates, +1 ignored LLVC)
- `pnpm typecheck` — pass
- `pnpm test` — pass (249)
- `pnpm tauri build --debug --no-bundle` — pass

## [1.0.0] — 2026-05-31 — Stable release

First stable release. No new features over 0.16.1 — this is the **surface cut**: the Tauri command + preset-JSON contract documented in [docs/STABLE-SURFACE.md](docs/STABLE-SURFACE.md) is now frozen and **additive-only** across the 1.x line (guarded by serialization tests in CI), and the manual test pass ([docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md)) is complete.

### What's in 1.0

- **Real-time voice modulation** — eight DSP effects (gate, denoiser, pitch, formant, EQ, robot, distortion, echo, reverb) in a live, reorderable chain with per-effect parameters, plus a live "added latency" readout.
- **AI voice conversion** — an ONNX-Runtime `VoiceConvert` effect with a bundled LLVC narrator voice and bring-your-own `.onnx` model support; degrades to passthrough (never hangs) when no model or runtime is present.
- **Monitor output routing** — an independent second output so the main send can go to VB-Cable (→ Discord / games) while you still hear yourself on headphones.
- **Soundboard** — folder-based clips with per-tile + master volume, drag-reorder, per-tile colors, recent folders, and system-wide hotkeys that fire even while unfocused.
- **Recording** — one-click capture of the modulated output to a timestamped WAV.
- **Presets** — five bundled + unlimited user presets, A/B compare, JSON export/import, and glyph-cast switching.
- **System tray** — minimize to tray so audio keeps running in the background during calls/games.
- **Local-first** — no telemetry, no account, no cloud. Windows + VB-Cable.

### Stability guarantee

From 1.0 on, the contracts in [docs/STABLE-SURFACE.md](docs/STABLE-SURFACE.md) — commands, events, wire-type shapes, the preset JSON schema, `localStorage` keys, and the on-disk layout — change in an additive-only way. Presets saved or exported under 1.0 keep loading across the 1.x line.

## [0.16.1] — 2026-05-31 — v1.0 prep: surface freeze + audit

Groundwork for the v1.0 stable cut — no behavior changes for end users beyond the About version fix. Audited the full IPC + preset surface (it was in good shape), then locked it down.

### Changed

- **About shows the real build version.** Replaced the long-stale hardcoded `v0.6.0` in Settings → About with the version stamped into the build via Tauri `getVersion()`. Release builds now show the actual release (e.g. `v1.0.0`); a `tauri dev` build reads `v0.0.0`.

### Removed

- Dropped two vestigial Phase-0 IPC commands — `ping` and `project_name` — that the frontend never called, trimming dead surface before the freeze. (The `divora_core::project_name()` library fn stays.)

### Added

- **[docs/STABLE-SURFACE.md](docs/STABLE-SURFACE.md)** — the v1.0 back-compat contract: every Tauri command (args/returns), the two events, all wire-type JSON shapes, the preset JSON schema + forward-compat rules, the `localStorage` keys, and the on-disk layout — plus the additive-only rule for the 1.x line.
- **Freeze-guard serialization tests** so an accidental break fails CI: the preset JSON schema + `Bundled`/`User` tag casing + legacy-file load (missing `version`, unknown fields), and the camelCase keys of `StreamInfo` / `EngineStatus` / `LevelUpdate` / `VoiceInfo` / `OnnxRuntimeStatus`.
- Extended **[docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md)** with Phase 12–16 sections (AI voice conversion, monitor routing, latency readout, soundboard volume + tray, recording) and a v1.0 release-gate note.

### Tests

- **Rust**: divora-core 129 → 132 (+3 freeze-guard); src-tauri 3 → 7 (+4 IPC-shape lock).
- **Frontend**: 242 (removed the dead `ping` mock case; no count change).

### Pre-push checklist (local, 2026-05-31)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (139 across crates, +1 ignored LLVC)
- `pnpm typecheck` — pass
- `pnpm test` — pass (242)
- `pnpm tauri build --debug --no-bundle` — pass

## [0.16.0] — 2026-05-31 — Phase 16: record the modulated output

One-click capture of exactly what your listeners hear — voice effects, soundboard, and all — to a timestamped WAV file.

### Added

- **Record button** on the Mixer (right rail). While the engine is running, hit **Record** to start capturing the post-chain output; the dot pulses red and the button turns into **Stop**. Files are written as 16-bit PCM mono WAV at the engine's sample rate.
- **Recordings folder** — files land in `%APPDATA%/DivoraVoice/recordings/`, named `divora-<date>_<time>.wav`. A new **Recordings** section in Settings shows the folder path, an **Open folder** button, and the name of your most recent take.
- **Engine plumbing** — recording reuses the existing processed-mono tap (the same signal sent to the main output / VB-Cable), so a saved take matches the call audio sample-for-sample. A dedicated **writer thread** drains a lock-free ring to disk, so no file I/O ever runs on the real-time audio callback. Hot samples clamp to the i16 range instead of wrapping.
- New backend commands `start_recording` / `stop_recording` / `recordings_dir`, a `recording` flag on the `audio-levels` event + status snapshot, and `AudioEngine::{start_recording, stop_recording, is_recording}`.

### Tests

- **Rust**: 128 → 129 (+1): the recording drain converts f32 → 16-bit PCM, clamps over-unity samples to ±full-scale (no wrap), and writes a valid mono WAV that hound reads back.
- **Frontend**: 237 → 242 (+5): `recordings_dir` / `start_recording` (forwards the filename, returns the path) / `stop_recording` command wrappers; the store's `toggleRecording` refuses to start while stopped, and starts → stops with a timestamped filename while running.

### Note

The live capture writes through a real device callback, so the end-to-end recording is best confirmed on the desktop; the drain/format correctness and the command wiring are covered by the unit tests above.

### Pre-push checklist (local, 2026-05-31)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (129, +1 ignored LLVC test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (242)
- `pnpm tauri build --debug --no-bundle` — pass

## [0.15.0] — 2026-05-31 — Phase 15: soundboard volume, folder persistence, system tray

Three requested quality-of-life items.

### Added

- **Soundboard volume** — per-tile and master gain. The mixer's `SoundboardCommand::Play` now carries a per-voice `gain`, plus a new `SetMasterGain` command; `mix_into` multiplies each voice by `voice_gain × master_gain` (both clamped 0–4). UI: a **master volume slider** in the soundboard toolbar and a **per-tile volume** slider in each tile's right-click menu. Both persist to `localStorage` (`divora.tileGains`, `divora.soundboardMasterGain`); the master gain is re-applied after an engine restart (a fresh session resets the mixer to unity).
- **Minimize to system tray** — closing/minimizing the window now **hides to a tray icon** instead of quitting, so audio (and the Discord/VB-Cable route) keeps running in the background — same as Discord itself. Left-click the tray icon (or "Show DivoraVoice") to restore; **"Quit"** exits for real. (`tray-icon` Tauri feature + a `WindowEvent::CloseRequested` → `prevent_close` + `hide`.)

### Fixed

- **Soundboard folder now persists across restarts.** It was held in a non-persisted signal, so every launch started with no folder. It's now saved to `localStorage` (`divora.soundboardFolder`) and re-scanned at startup — your tiles + per-tile hotkeys come back automatically.

### Tests

- **Rust**: 125 → 128 (+3 mixer): per-voice gain scales output; master gain scales every voice; gains clamp to a safe range (no runaway levels).
- **Frontend**: 233 → 237 (+4 store): soundboard folder persists; `tileGain` defaults to 1.0 + `setTileGain` persists; `setSoundboardMasterGain` persists + sends the backend command; `playClip` passes the tile's gain. (Existing play assertions updated for the new `gain` arg.)

### Note

The tray + close-to-tray behavior can't be exercised headless, so it's verified by compile + build; the live tray interaction is best confirmed on the desktop.

### Pre-push checklist (local, 2026-05-31)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (128, +1 ignored LLVC test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (237)
- `pnpm tauri build --debug --no-bundle` — pass

## [0.14.0] — 2026-05-30 — Phase 14: live latency readout

Voice Convert adds ~256 ms; the denoiser 10 ms; pitch/formant ~21 ms each. Now you can see it: the Mixer shows how much latency the active effects are adding, updating the instant you toggle one.

### Added

- **`AudioEffect::latency_samples(sample_rate)`** (default 0) + **`EffectChain::latency_samples`** which sums the *enabled* effects' fixed delays. Implemented for the effects that actually buffer:
  - **Voice Convert**: the 16 kHz inference chunk (≈ 256 ms), and only when a model is loaded (passthrough adds nothing).
  - **Denoiser**: one 480-sample frame (10 ms), and only at 48 kHz (off-rate it bypasses).
  - **Pitch** / **Formant**: the phase-vocoder STFT window (1024 samples ≈ 21 ms each).
  - Gate / EQ / robot / distortion add 0 (sample-by-sample); echo / reverb add 0 to the dry-path latency (they're wet tails).
- The engine publishes the chain's added latency (ms) via the `EngineState` + the ~30 Hz `audio-levels` event, and the **Mixer header shows "· +N ms latency"** while running (with a tooltip explaining the contributors). It moves live as effects toggle.

### Tests

- **Rust**: 120 → 125 (+5): empty chain = 0; sums enabled effects (denoiser 480 + pitch 1024); disabled effects add 0; denoiser latency only at 48 kHz; Voice Convert adds 0 without a model.

### Deferred

- The **buffer-size selector** from the original Phase 14 sketch is deferred — WASAPI shared mode largely ignores requested buffer sizes, so it can't reliably "change the latency." The readout (the genuinely useful part) ships now.

### Pre-push checklist (local, 2026-05-30)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (125, +1 ignored LLVC test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (233)
- `pnpm tauri build --debug --no-bundle` — pass

### Roadmap

Per request, the next phase (**15**) bundles soundboard + tray polish — per-tile/master soundboard volume, persisting the soundboard folder across restarts, and minimize-to-system-tray. Recording moves to Phase 16; v1.0 after.

## [0.13.0] — 2026-05-30 — Phase 13: monitor output routing (hear yourself + route to VB-Cable)

Field report:

> "When I change the current output to VB Cable I cannot hear it anymore. Can you add an option to choose a monitor output, and hear the soundboard on it too?"

Exactly right: with one output stream, routing to VB-Cable sent the voice to Discord but left nothing playing to your ears. v0.13.0 adds a **second, independent output** so you can route the main output to VB-Cable *and* hear yourself (plus soundboard clips) on headphones.

### Added

- **Monitor output device** — Settings → Audio devices gains a "Monitor output (hear yourself)" picker (with a "None — use main output" default). Pick your headphones there, set the main Output to CABLE Input, and you hear your modulated voice while Discord/games receive it.
- A **second cpal output stream** to the monitor device. The main output callback runs the DSP + soundboard mix once and **taps the processed audio into a monitor ring**; the monitor stream resamples that to its own device rate and plays it. So the soundboard is audible on the monitor for free (it's mixed in before the tap).

### Changed

- **Gating semantics** (`main_output_plays`): with a separate monitor device active, the **main output always plays** (it's the send to VB-Cable — Discord must keep hearing you), and the **Monitor toggle mutes only the headphone stream**. With no separate monitor device, the toggle gates the main output exactly as before — fully backward compatible.
- The monitor device persists to `localStorage` (`divora.monitorDevice`) and is wired into the live device-switch effect, so changing it restarts the engine cleanly (like input/output).
- `StreamInfo` gains `monitorName`; `start_audio_engine` + `AudioEngine::start` take a monitor device argument.

### Tests

- **Rust**: 119 → 120 (+1): `main_output_gating_truth_table` locks the four gating cases (no-monitor → toggle gates output; monitor → output always sends).
- **Frontend**: 230 → 233 (+3): `startEngine` passes the monitor device; changing the monitor while running restarts the engine; `setSelectedMonitor` persists to `localStorage`. (Two `api.test` assertions updated for the new `monitorName` arg.)

### Pre-push checklist (local, 2026-05-30)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (120, +1 ignored LLVC test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (233)
- `pnpm tauri build --debug --no-bundle` — pass

### How to use it (for Discord/games)

Settings → Audio devices: **Output** = `CABLE Input (VB-Audio Virtual Cable)`, **Monitor output** = your headphones. In Discord, set the mic to `CABLE Output`. You'll hear your modulated voice + soundboard on headphones; Discord hears it through the cable. The Monitor toggle mutes only your headphones.

## [0.12.5] — 2026-05-30 — Installers carry the real version number

### Fixed

- **Release installers are now named `DivoraVoice_<version>_…`** (e.g. `DivoraVoice_0.12.5_x64_en-US.msi`) instead of `0.0.0`, and the installed exe reports the real version. `release.yml` gains a "Stamp version from tag" step that writes the tag's version (stripped of the leading `v`) into `tauri.conf.json` + `Cargo.toml` on the CI runner **before** building. The committed files stay at `0.0.0` — git tags remain the single version source of truth; the stamp only edits the ephemeral CI checkout.

### Notes

- CI-only change: no app code touched. The stamp regex was dry-run locally against both files (replaces exactly one `0.0.0` in each). Validation that the published installer carries the version is inherent in this release — the v0.12.5 installers will be named `0.12.5`.

## [0.12.4] — 2026-05-30 — Installer bundles voice conversion (out-of-the-box AI)

The release installer now ships `onnxruntime.dll` + the LLVC narrator model, so a fresh install does AI voice conversion with **zero manual setup** — no Python, no copying DLLs.

### Added

- **Bundled voice assets in the MSI/NSIS installer.** A config overlay (`src-tauri/tauri.bundle.conf.json`) adds `bundle.resources` for the runtime DLL + model; the release workflow fetches them and builds with `--config` that overlay. Verified locally: both installers build with the assets staged to `<resource_dir>/onnxruntime.dll` + `<resource_dir>/voices/llvc-narrator.onnx`.
- **Runtime bundled-asset discovery** (`src-tauri/src/lib.rs` setup): on startup the app points `ORT_DYLIB_PATH` at the bundled `onnxruntime.dll` (unless already set / a dev DLL sits next to the exe), and exposes the bundled `voices/` dir.
- **`list_voices` now merges user + bundled voices** via a new `scan_voice_dir` helper. A user-installed `<id>.onnx` shadows a bundled voice of the same id, so users can override shipped voices.
- **`scripts/fetch-voice-assets.ps1`** — downloads the binaries from the `voice-assets-v1` GitHub release into `src-tauri/resources/` (gitignored). Run before a full local `pnpm tauri build`.
- **`voice-assets-v1` GitHub release** hosting `onnxruntime.dll` (ONNX Runtime 1.26, MIT) + `llvc-narrator.onnx` (LLVC, MIT; LibriSpeech narrator target). Marked prerelease so it stays out of the "Latest" slot.

### Why a separate overlay config

Tauri validates `bundle.resources` paths during the **build script**, so any reference in the base `tauri.conf.json` would break every `cargo build` / `cargo clippy` / `cargo test` / `--no-bundle` CI run that doesn't have the ~31 MB of binaries. Keeping the resources in an overlay that only the release build applies means the entire normal CI matrix stays green **without fetching anything** — only `release.yml` (which runs on `v*.*.*` tags) fetches + bundles.

### Tests

- **Rust**: 119 → 122 (+3 in a new `lib.rs` test module): `scan_voice_dir` lists `.onnx` only; user voices shadow bundled ones by id (and the retained path is the user copy); a missing dir is a no-op.

### Verified locally (2026-05-30)

- Base config: `cargo check`/`--no-bundle` build **pass with `src-tauri/resources/` absent** (CI scenario) — confirms normal CI won't need the assets.
- Overlay config + assets present: `pnpm tauri build --config src-tauri/tauri.bundle.conf.json` produced `DivoraVoice_0.0.0_x64_en-US.msi` (24.9 MB) + `DivoraVoice_0.0.0_x64-setup.exe` (20.6 MB) with resources staged to the expected layout.
- The fetch script pulls both assets from the release into `src-tauri/resources/`.

### Pre-push checklist (local, 2026-05-30)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (122, +1 ignored LLVC integration test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (230)
- `pnpm tauri build --debug --no-bundle` — pass (base config, CI command)

### Not verified in-session

Installing the produced MSI/NSIS and launching the *installed* app — that needs a system install. The bundling config, staged layout, runtime wiring, and the end-to-end conversion path (v0.12.3) are all verified; only the final "double-click the installer" step is manual.

## [0.12.3] — 2026-05-30 — LLVC wired up: real AI voice conversion runs end-to-end

The ONNX path is no longer hypothetical. LLVC (KoeAI's MIT-licensed Low-latency Low-resource Voice Conversion) is exported to ONNX, validated, and confirmed converting audio through the actual `VoiceConverter` → `ort` → ONNX Runtime path on a dev machine.

### Changed

- **`run_inference` now speaks LLVC's real tensor contract**: input `audio` `[1, 1, T]` f32 → first output `[1, 1, T]` f32 (was a placeholder `[1, N]`). `T` is dynamic; the engine feeds 4096-sample 16 kHz chunks (a clean multiple of LLVC's internal stride L=16, so no boundary padding). Mismatched models still degrade to passthrough rather than crashing.

### Added

- **`docs/voice-models/`** — a reproducible recipe + the export/validation scripts:
  - `export_onnx.py` traces LLVC's non-streaming `Net.forward` (`dynamo=False` for predictable graph io) and checks ORT-vs-PyTorch parity.
  - `validate_chunked.py` runs a real speech clip through the ONNX in 4096-sample chunks (mirroring the engine) and gates on finite, audible, non-clipping output.
  - `README.md` documents the model I/O contract, the minimal dep set (**no fairseq** — only torch/torchaudio/speechbrain), the speechbrain `PositionalEncoding` vendoring needed for export, and Windows install (model → voices dir, `onnxruntime.dll` → next to the exe).
- **Gated end-to-end test** `llvc_model_converts_a_chunk` (`#[ignore]`d so CI + plain `cargo test` skip it — they have no model/DLL). On a dev box with the model installed it points `ORT_DYLIB_PATH` at the runtime, loads the model, and asserts a chunk comes back finite, full-length, and changed.

### Verified on a dev machine (not in CI — needs the binary assets)

- Export: ONNX vs PyTorch `max|diff| = 6.2e-06`.
- Chunked real-speech conversion: 15×4096 chunks, rms 0.0197 → 0.0210, no clipping.
- Rust end-to-end: model loads via `ort` 2.0-rc.12 against the **onnxruntime 1.26** DLL (api-24 forward-compat confirmed), `run_inference` returns a transformed chunk (`mean|delta| = 0.19`).

### Not in this release (deliberately)

- The **model file (~14 MB ONNX) and `onnxruntime.dll` (~17 MB) are not committed** — they're reproducible via `docs/voice-models/` and are installed locally on the dev machine. Bundling them into the installer (so end users get voice conversion out-of-the-box) remains future packaging work.

### Pre-push checklist (local, 2026-05-30)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (119, +1 ignored LLVC integration test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (230)
- `pnpm tauri build --debug --no-bundle` — pass

### How to hear it (on the dev machine)

Settings → Voice library now shows "ONNX Runtime detected" and lists `llvc-narrator`. Select it, then pick the **Deep Narrator** preset (or enable **Voice Convert** in any chain): the mic is converted to the LLVC target voice, layered under the DSP shaping.

## [0.12.2] — 2026-05-30 — Deep Narrator that actually sounds deep (DSP), AI as bring-your-own

Field report:

> "I see the new Voice Convert option, but I am not hearing that it actually sounds different than my normal voice."

Right — and it won't until a real ONNX model + the runtime are installed (v0.12.0/v0.12.1 shipped the framework, not a model). Rather than block the audible payoff on a model I can't source/verify here, this release makes **"Deep Narrator" sound dramatically deep right now using DSP**, and keeps the ONNX `VoiceConvert` effect as an optional bring-your-own-model layer on top. (User picked this path.)

### Changed

- **"Deep Narrator AI" → "Deep Narrator", rebuilt as a real DSP deep-voice.** The v0.12.0 version leaned entirely on the passthrough `VoiceConvert` for its character, so it was nearly inaudible. The new chain does the work with proven DSP:
  - `gate` (−50 dB) + `denoiser` (60%) → clean studio input
  - `voice_convert` (enabled, 90% mix) → the bring-your-own-AI slot; a true no-op (zero added latency) until a model is selected in Settings → Voice library
  - `pitch` −4 st + `formant` −3 → drops the voice into the chest, "bigger throat" resonance
  - `eq` low +5 / mid +1 / high −2 → body + presence, tamed sibilance
  - `reverb` size 38 / mix 14 → intimate room gravitas (deliberately *not* cavernous like "Hollow King")

  Preset id (`deep-narrator-ai`) is unchanged, so A/B snapshots and any references keep working; only the display name + chain changed.

### How the two voice paths relate now

- **Deep Narrator (DSP)** — works on every machine today, no model or runtime needed. This is the audible "deep voice."
- **Voice Convert (ONNX)** — still the real-AI path; passthrough until you install `onnxruntime.dll` + drop an `.onnx` model into the voices folder and select it. When you do, it layers *on top* of the DSP chain (it runs before pitch/formant in the chain).

### Tests

- **Rust**: 118 → 119 (+1): `deep_narrator_lowers_the_voice_via_dsp` locks the preset's audible character — asserts pitch + formant are enabled and shift *down*, and that the enabled `voice_convert` BYO slot is retained. Guards against a future edit silently flattening it back to a clean voice.

### Pre-push checklist (local, 2026-05-30)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (119)
- `pnpm typecheck` — pass
- `pnpm test` — pass (230)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The whole point of Phase 12 is "make my voice sound different." The AI route is real but gated on assets that need a deliberate sourcing decision; meanwhile DivoraVoice already has a tested phase-vocoder pitch shifter, LPC formant warp, parametric EQ, and reverb. Composing them into a genuinely deep, warm narrator delivers the payoff today — and the ONNX slot is right there in the same preset for when a model arrives.

## [0.12.1] — 2026-05-29 — Voice library: select voices, background loading, runtime status

Follow-up to v0.12.0. The framework shipped, but the `VoiceConvert` effect had no way to be pointed at a model and no UI — so it was always a silent passthrough. v0.12.1 wires the **selection path end-to-end**: a Settings → Voice library panel, off-audio-thread model loading, and runtime-presence reporting. (The bundled `onnxruntime.dll` + a shipped model are still v0.12.2 — until then the panel honestly reports "Runtime not installed.")

### Added

- **Settings → Voice library** section: a live ONNX-runtime status indicator (green "detected" / amber "not installed" with install guidance), a selectable voice list (radio-style rows, "None — pass my voice through" first), the voices folder path with an **Open folder** button, and a **Refresh** button. Empty + missing-runtime states are spelled out rather than left blank.
- **Tauri commands**: `voices_dir`, `list_voices` (scans `%APPDATA%/DivoraVoice/voices/*.onnx` → id/name/path/size), `onnx_runtime_status` (runtime locatable? + voices dir), `set_voice_model` (routes a model path to the `VoiceConvert` effect by chain index). The voices directory is created at startup alongside the preset store.
- **`DspCommand::SetResource { index, key, value }`** + a default-no-op `AudioEffect::set_resource` trait method. `VoiceConvert` overrides it: `key == "model"` loads the given path (voice name derived from the file stem), `None`/empty clears to passthrough.
- **Store**: `voices()`, `onnxStatus()`, `activeVoiceId()`, `setActiveVoice()`, `refreshVoiceLibrary()`. The active voice id persists to `localStorage` and is re-applied whenever the chain is (re)sent — so a `SetChain` (which rebuilds the `VoiceConvert` fresh) doesn't silently drop the selection. A persisted voice that's no longer on disk is cleared on refresh.

### Changed

- **Model loading moved off the audio thread.** `VoiceConverter::set_model_path` now spawns a background loader thread and hands the `Session` back through an `mpsc` channel; `process` picks it up with a non-blocking `try_recv`. The audio callback never blocks on `Session::builder` (which can be slow, and — without the runtime — could otherwise stall). Switching voices mid-stream just swaps sessions on the next callback.

### Tests

- **Rust**: 113 → 118 (+5 in `dsp::voice_convert`): `set_resource` derives the voice name from the file stem; `None`/empty clears it; an unknown key is a no-op; a missing-file `set_resource` stays in passthrough across 50 callbacks without hanging (exercises the background-loader poll path).
- **Frontend**: 226 → 230 (+4 in a new `SettingsScreen — Voice library` block): queries voices + runtime status on mount; renders the section + not-installed guidance; offers the "None" passthrough option; shows the empty hint with no models. The two v0.11.4 Audio-devices Refresh tests were retargeted by button title (there are now two Refresh buttons).

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (118, no hang)
- `pnpm typecheck` — pass
- `pnpm test` — pass (230)
- `pnpm tauri build --debug --no-bundle` — pass

### Still to come in v0.12.x

- Bundle `onnxruntime.dll` with the installer (Tauri resources/externalBin) so voice conversion works out of the box.
- Source + ship a compatible LLVC ONNX model and a "Deep Narrator" voice → first audible conversion.
- Firm up the model I/O tensor contract once a real model is in hand.

## [0.12.0] — 2026-05-29 — Phase 12: AI voice conversion framework (ONNX Runtime + `VoiceConvert` effect)

The plan's headline Phase 12 goal is on-device AI voice conversion ("Deep Narrator AI works real-time on CPU"). This is the first ship of that work: the **ONNX Runtime integration, the streaming `VoiceConvert` effect, and the chain/preset/UI wiring**. The model file + bundled `onnxruntime.dll` follow in v0.12.x patches (the user pre-accepted that Phase 12 "may take multiple commits to land green").

### Added

- **`ort` (ONNX Runtime 1.24 wrapper) + `ndarray`** in `divora-core`. Pinned to `=2.0.0-rc.12` with `default-features = false` + `["load-dynamic", "ndarray", "std", "api-24"]`. `load-dynamic` means the runtime DLL is resolved lazily at first use rather than linked at build time — so CI and machines without the runtime still compile and run. (`api-24` is required: without it `ort`'s vitis EP module fails to compile against the `load-dynamic` `OrtApi` struct.)
- **`VoiceConverter` streaming effect** (`divora-core/src/dsp/voice_convert.rs`). Full real-time pipeline:
  - Buffers native-rate (48 kHz) input → resamples to **16 kHz** (the rate published voice-conversion models assume) via `rubato::SincFixedOut`.
  - Accumulates a **4096-sample (≈256 ms) chunk**, runs it through the loaded ONNX session, and resamples the result back to 48 kHz.
  - Delay-matched **wet/dry mix** (`mix` param, 0–100 %) so users can blend converted + original voice without phase artifacts.
- **`EffectKind::VoiceConvert`** wired into `EffectChain::build_effect`. The `EffectKind` serde rename changed from `lowercase` → `snake_case` so the new multi-word variant round-trips as `voice_convert` (all existing single-word variants are unchanged).
- **Frontend**: `voice_convert` added to `EffectId`, `EffectKindWire`, and the `EFFECTS` catalog (mix param, `wave` sigil), positioned after the cleanup effects (gate → denoiser → **voice_convert** → pitch → …) so it converts a clean, gated, denoised signal.
- **"Deep Narrator AI" bundled preset** (`deep-narrator-ai.json`): gate → denoiser → voice_convert → EQ low-shelf → ceremonial reverb. Works as a polished clean voice today; becomes true AI conversion once a model + runtime are installed.

### Safety: graceful degradation, never a hang

The effect is built to run on **every** machine, model or no model:

- **No `onnxruntime.dll`** (every CI runner, every fresh install) → `load_session` returns `None` *before touching `ort`* → effect is a clean passthrough.
- **No `.onnx` model file** → same; no session, passthrough.
- **Off-rate input (≠ 48 kHz)** → bypass.
- **Inference error mid-stream** → that chunk echoes dry; the session stays loaded for the next chunk.

Critically, `load_session` checks that **both the model file and the runtime dylib physically exist** (`ORT_DYLIB_PATH` or a platform-default filename next to the exe, cached via `OnceLock`) before calling `Session::builder()`. This is load-bearing: with `load-dynamic`, the first `ort` call *blocks* rather than errors when the DLL is missing — so calling it unconditionally hung `cargo test` (and would hang CI for hours, since there's no per-test timeout). Gating on physical presence means `ort` is only ever invoked when the runtime is genuinely installed.

### Tests

- **Rust**: 104 → 113 (+9 in `dsp::voice_convert`): kind id; default-disabled; passthrough when disabled / when enabled-but-no-model / when model path cleared; missing-file does-not-panic-or-hang; off-rate bypass; mix clamp; enable/disable pipeline clear. All complete in <1 ms (the missing-file test previously hung 60 s+ before the presence-gate fix).
- **Frontend**: 224 → 226 (+2): `voice_convert` exposes a 0–100 % mix param; its chain position sits after `denoiser` and before `pitch`.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (113, no hang)
- `pnpm typecheck` — pass
- `pnpm test` — pass (226)
- `pnpm tauri build --debug --no-bundle` — pass (ort links cleanly)

### Still to come in v0.12.x

- Bundle `onnxruntime.dll` with the installer (Tauri `externalBin` / resources).
- Source + ship a compatible LLVC ONNX model and a "Deep Narrator" speaker.
- Settings → **Voice library** UI: install/select voices, model-status indicator, background (off-audio-thread) model loading.
- Firm up the model I/O tensor contract once a real model is in hand.

## [0.11.5] — 2026-05-29 — Restore titlebar drag (regression from v0.11.3)

Field report:

> "Can you also allow the window to be dragged? It looks like with the 'cast anywhere' changes that stopped."

v0.11.3's `SparkLayer` listens at the window level on `pointerdown` with `capture: true` so a drag started anywhere on the Mixer (except UI controls) becomes a cast. The capture phase fires before any element-level listener — including Tauri's native handler for `data-tauri-drag-region`, which is what makes the custom titlebar a draggable window-chrome surface in our frameless build. `isCastBlocker` only knew about controls and cards, so it let the titlebar pointerdown through, started a cast, and silently swallowed the would-be drag.

### Fixed

- **`isCastBlocker` now also short-circuits on `[data-tauri-drag-region]`.** Adding this attribute selector to the CSS list makes the predicate report any titlebar surface (or anything else marked as OS drag chrome) as a cast blocker. The SparkLayer pointerdown handler bails without `preventDefault`-ing, the event continues to Tauri's drag-region listener, and the OS-level `startDragging` fires as it did before v0.11.3.

### Tests

- **Frontend**: 222 → 224 (+2). Two new cases in `SparkLayer.isCastBlocker` — a bare `[data-tauri-drag-region]` div is a blocker; a descendant span inside one is also a blocker.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass
- `pnpm typecheck` — pass
- `pnpm test` — pass (224)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The custom frameless titlebar is the only way to move the window — there's no OS-drawn one to fall back on. A regression here turned the app into a "stuck where I opened it" experience whenever the user was on the Mixer. Trivial one-line fix in the CSS selector; regression test ensures the case is now hard-coded into the predicate's expectations.

## [0.11.4] — 2026-05-29 — Live device enumeration: refresh on focus + Settings entry + manual button

Field report:

> "When adding a new input device (and potentially output device) after starting the app it does not update."

cpal's `Host::devices()` is a snapshot — it only reflects the system list at the moment of the call. There's no cross-platform "device arrived" notification we can subscribe to without dropping into Windows-specific COM (`IMMNotificationClient`). v0.11.0's live-switching effect handled the "user changed the SELECTION" case, but not the "user plugged in a NEW device" case — so v0.11.4 covers it with three converging refresh paths.

### Added

- **Auto-refresh on window focus.** `App` now subscribes to Tauri's `getCurrentWindow().onFocusChanged` (the source of truth on the OS-window level) and re-runs `refreshDevices()` every time `focused === true`. A browser-level `window.focus` listener runs in parallel as a belt-and-suspenders fallback for browser preview / older Tauri builds where the plugin import fails. The common workflow — plug device in → alt-tab back to DivoraVoice — now updates the device list transparently.
- **Auto-refresh on Settings entry.** `SettingsScreen`'s `onMount` calls `refreshDevices()` alongside the existing `refreshVirtualMicStatus()`. Catches the "user navigates straight to Settings after plugging in" path.
- **Manual `Refresh` button** at the top of the Audio devices section (`Sigil.refresh` + ghost button). Sets a transient `refreshing` signal so the button renders `Scanning…` and disables itself while in flight — guards against double-clicks.

### Tests

- **Frontend**: 215 → 222 (+7).
  - 3 in `App shell — focus device refresh (v0.11.4)`: re-enumerates devices when Tauri `onFocusChanged({ payload: true })` fires; ignores blur events (`payload: false`); falls back to `window.focus` when the Tauri path is unavailable.
  - 4 in a new `SettingsScreen — audio device refresh (v0.11.4)` test file: re-enumerates on mount; renders the `Refresh` button labelled `Refresh` when idle; clicking it triggers a fresh enumeration; the `Audio devices` section eyebrow renders alongside the control.
  - The `getCurrentWindow` mock in `App.test.tsx` now includes an `onFocusChanged` stub that registers callbacks in a module-level array, so tests can drive focus changes directly.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass
- `pnpm typecheck` — pass
- `pnpm test` — pass (222)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The user's workflow when a new mic or headset arrives is: plug it in, switch to DivoraVoice, open Settings, pick it. Until v0.11.4, that last step quietly didn't work — the device dropdown still showed whatever was around at app launch. Refreshing on every focus change makes the new device appear without any "re-scan" interaction, while the manual button covers the few edge cases where focus never changed.

## [0.11.3] — 2026-05-29 — Canvas-based cast: drag anywhere, mockup-faithful omen

Follow-up to v0.11.2. Three field reports:

> "Remove the cast button and allow drawing anywhere like the mockup. Change the drag animation to match the mockup. Have the same stylized glyph animation as the mockup."

v0.11.2 added drag-from-empty-space as a *third* invocation path beside the Cast button and `G` hotkey. The mockup design has *no* button at all — drag IS the cast — and the spark trail + post-cast "omen" animation are richer than our SVG fallbacks could ever be.

### Changed

- **Cast button removed from the Mixer header.** The right-side controls now only carry the `Compare A/B` segmented (matching the prototype's `screen_mixer.jsx`). The `G` hotkey is gone too — there's no overlay to open anymore.
- **`SparkLayer` (new, canvas-based) replaces `GlyphCastOverlay` + `SpellCastReveal`.** A single transparent `<canvas>` sits over the Mixer at all times with `pointer-events: none` and `z-index: 58`. It listens at the window level with capture-phase pointer events, draws sparks and the post-cast omen via `requestAnimationFrame`, and is sized DPR-aware for crisp strokes on hi-DPI displays.
  - **Sparks (drag animation)** — `parts.length` capped at 1600; gravity (`vy += 0.012`), friction (`vx *= 0.985`, `vy *= 0.985`), per-spark decay `0.012 + Math.random() * 0.02`, direction-following emission (`1 + min(6, d/6)` sparks per move, biased toward stroke direction), 4-color trail palette `["#7C5CF6", "#EC4899", "#9F7CFF", "#C9B8FF"]`, `shadowBlur=10` glow.
  - **Omen (post-cast glyph animation)** — when a shape matches a bound preset, the SparkLayer:
    1. Sprays sparks along the recognised shape outline (along the corners for polygons, along the radius for circles) in the preset's brand color.
    2. Bursts 110 omnidirectional sparks at the centroid.
    3. Draws a 2.5-second omen over three phases: pop-in (0-280 ms, alpha + 0.6→1.0 scale), hold (full alpha), fade (last 650 ms). Renders an expanding halo ring (1.12× → 1.67× radius), the canonical shape outline in the preset color with `shadowBlur=22` glow, the preset name in Bricolage Grotesque 19 px under it, and "◆ S P E L L C A S T ◆" in Space Mono 10 px below that.

### Added

- **`detectShape()` in `data/glyphs.ts`** — returns rich geometry `{ type, cx, cy, size, corners?, r? }` instead of just a `GlyphId`. Ported 1:1 from the prototype's `spark_canvas.jsx`: Ramer–Douglas–Peucker line simplification + radial uniformity (stdR/meanR) + corner-count voting + polygon-area threshold per shape. The existing `classifyGlyph` stays untouched (different algorithm, different tuning) since the canvas pipeline wants the geometry to draw the recognised outline.
- **`isCastBlocker(target)`** — predicate exported from `SparkLayer` that the capture-phase pointer handler uses to decide whether to ignore a pointerdown (control) or capture it (empty space). Uses `Element.closest()` with a single CSS selector: `button, a, input, select, textarea, [role="button"], [role="slider"], [role="switch"], [role="tab"], [role="radio"], [role="radiogroup"], .card, .seg, .kbd, [data-cast-block]`.

### Removed

- `src/components/GlyphCastOverlay.tsx` + test file — replaced by `SparkLayer`. The SVG-based overlay couldn't render the canvas-style spark cloud or the omen ring anyway.
- `src/components/SpellCastReveal.tsx` + test file — replaced by the canvas-based omen. The HTML reveal panel is gone.
- `src/screens/MixerScreen.test.tsx` (v0.11.2) — its `isInteractiveAncestor` predicate moved into `SparkLayer.isCastBlocker` with a tighter CSS-selector implementation; tests moved to `SparkLayer.test.tsx`.
- `@keyframes spell-cast-veil`, `spell-cast-reveal`, `spell-cast-breathe` in `styles.css` — orphaned by `SpellCastReveal` deletion.
- `G` hotkey for cast — no overlay, no open-cast verb, no need for a hotkey.

### Tests

- **Frontend**: 209 → 215 (+6 net; +24 new, −18 deleted).
  - 9 new in `glyph classifier > detectShape` — short paths rejected; tiny strokes rejected; open paths rejected; triangle / inv-triangle / square / circle all matched with correct corners / radius / centroid; arbitrary scribbles rejected.
  - 3 new in `glyph classifier > rdp` — line endpoints preserved; interior corner preserved when above epsilon; `< 3` points returned as-is.
  - 24 new in `SparkLayer.isCastBlocker` — element + 5 tags + 6 roles + 3 classes + `data-cast-block` + deep-nested targets + null defensiveness.
  - −18 deleted across `GlyphCastOverlay.test.tsx` (9), `SpellCastReveal.test.tsx` (4), `MixerScreen.test.tsx` (17 — superseded by `SparkLayer.test.tsx`).

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (104)
- `pnpm typecheck` — pass
- `pnpm test` — pass (215)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The cast is the most distinctive interaction in the app — and until v0.11.3 it had a button stapled to it, an SVG overlay that didn't quite feel like the mockup, and a reveal that lived in a different visual layer than the sparks that summoned it. Folding all three into one canvas matches the prototype byte-for-byte: drag anywhere on the Mixer, sparks chase the cursor with weight + glow + gravity, and on release the bound preset's glyph blooms in its brand color over the same canvas. No "open the cast overlay" step. No layer switch. Just drawing.

## [0.11.2] — 2026-05-29 — SpellCircle keyframes + drag-from-empty-space cast

Follow-up to v0.11.1. Two field reports:

> "The 'Motion' and 'Mystical' Appearance options are still not working as expected from the mockup. Additionally, the drawing effects are not the same and require pressing the 'cast' button still unlike the mockup."

v0.11.1 fixed the *values* and *persistence* of the Tweaks knobs, but on inspection the SpellCircle was driving six animation names — `breathe`, `spin-slow`, `spin-rev`, `pulse-ring`, `dash-flow`, `float-up` — that were referenced from the JS but **never defined as `@keyframes` in `styles.css`**. So the orbit didn't rotate, the pulse rings didn't pulse, the constellation didn't drift, and the particles didn't float. Motion appeared "broken" because there was literally nothing for the duration multiplier to scale. Mystical appeared "broken" because the visuals it gated (outer ring, ticks, constellation) were sitting in CSS that never animated even at `rich`.

Separately, PLAN.md's Phase 11 spec for the cast called for "left-click drag on empty space (not on UI controls) starts capturing pointer trail" — but the only invocation paths shipped were the explicit Cast button and the `G` hotkey. The mockup expects the drag itself to *be* the gesture.

### Fixed

- **Missing SpellCircle `@keyframes` ported 1:1 from the prototype.** Added `breathe`, `spin-slow`, `spin-rev`, `pulse-ring`, `dash-flow`, `float-up` (and `shimmer`, used in future polish) as top-level rules in `src/styles.css`. Placed outside any `@layer` so animation-name lookups are unambiguous regardless of cascade order. The `breathe` keyframe additionally consumes `var(--motion)` directly so a low motion setting visibly damps both the opacity floor and the breathe amplitude even when `animation-duration` isn't overridden.

### Added

- **Drag-from-empty-space cast invocation.** A `pointerdown` on the Mixer's outer container now opens the cast overlay and immediately seeds it with the originating pointer, so the user's existing drag continues without a second mouse press. The Cast button + `G` hotkey remain unchanged as alternatives — this is purely additive.
  - The MixerScreen exports `isInteractiveAncestor(target, root)` which walks up from the pointer target stopping at the cast root. Any `<button>`, `<input>`, `<select>`, `<textarea>`, `<a>`, `role="button|slider|switch"`, `contenteditable`, `.card`, or `data-cast-block` ancestor short-circuits the cast trigger so clicks on controls still go to the control. The walk halts at the root so chrome ancestors (titlebar, sidebar) don't false-positive.
  - `GlyphCastOverlay` accepts an optional `seedPointer: { pointerId, clientX, clientY }` prop. When supplied, on mount it calls `setPointerCapture(pointerId)` on its root, pre-fills `points[0]` with the local-coord-mapped seed, sets `drawing=true`, and starts the rAF spark loop. A `try { … } catch` around capture means a fast click+release (pointer already up) degrades to "overlay open, idle blurb visible" instead of throwing.

### Tests

- **Frontend**: 179 → 209 (+30).
  - 7 new tests in `styles.css > SpellCircle keyframes (v0.11.2)` — one per missing keyframe asserting `@keyframes <name>` is present, plus a regression test that `breathe` references `var(--motion)`.
  - 1 new test asserting the `:root[data-motion="functional"] *` global rule still pins animation-duration + transition-duration to 0.001ms (v0.11.1's coverage was incomplete).
  - 17 new tests in `MixerScreen > isInteractiveAncestor` — root-itself empty space; plain descendants; `<button>` direct + nested; `<input>` / `<select>` / `<textarea>` / `<a>`; `role="button|slider|switch"`; `.card`; `data-cast-block` opt-out; `contenteditable="true"` accepted + `"false"` ignored; walk terminates at the cast root; null start is defensive.
  - 5 new tests in `GlyphCastOverlay seedPointer (v0.11.2)` — captures the seed pointer id on mount; enters drawing mode immediately (hides the idle blurb); without a seed does NOT call setPointerCapture; without a seed still shows the idle blurb (existing behavior preserved); survives a setPointerCapture failure without throwing.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (104)
- `pnpm typecheck` — pass
- `pnpm test` — pass (209)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

v0.11.1 made the *knobs* correct; v0.11.2 makes the *visuals they control* exist. Without the six SpellCircle keyframes, the Mixer's signature element was a static SVG no matter where Motion / Mystical were set — a clear "did anything actually happen?" UX failure. And drag-from-empty-space restores the mockup's headline gesture: the Mixer *is* the casting surface, not a screen with a Cast button stapled to it.

## [0.11.1] — 2026-05-29 — Settings → Appearance: Mystical + Motion actually do something

Field report: "the Motion and Mystical Appearance options don't seem to be working as expected from the mockup."

### Fixed

- **`Mystical` levels mapped to the wrong numbers.** The prototype's `tweaks.jsx` maps `subtle / balanced / rich → 0.3 / 0.7 / 1.0`. Our code shipped `0 / 0.5 / 1.0`. With `balanced = 0.5`, the SpellCircle's `mystical >= 0.5` cut treated balanced as borderline-rich and `subtle = 0` looked harsh (every decoration off). Aligned to the prototype values via three exported constants (`MYSTICAL_SUBTLE`, `MYSTICAL_BALANCED`, `MYSTICAL_RICH`) used by both the store and the SettingsScreen segmented control. Snap thresholds in `mysticalSegment()` updated to `0.4 / 0.85` (midpoints of the new triplet).
- **Default `mystical` was `1.0` ("rich")** instead of the prototype's `0.7` ("balanced"). Defaulting at the top of the range meant moving the slider only ever produced *less*; coming back to the centre felt like "did nothing change?" Default now matches the prototype.
- **`Motion = functional` did nothing outside the Spell Circle.** The `--motion` CSS variable was defined under `:root[data-motion="…"]` but **no rule consumed it anywhere in `styles.css`**. So picking "Functional" disabled the Spell Circle's orbit / particles (gated in JS) but the hotkey-capture shimmer, the SPELL CAST reveal, and every component transition kept playing. Added a universal `:root[data-motion="functional"] *` rule that pins animation + transition durations to 0.001 ms. The spell-circle JS gating still skips work entirely (saving CPU), while the CSS rule covers the rest of the app.
- **`Tweaks` didn't persist across restarts.** Every reload reset Mystical / Motion / Mood / etc. to defaults, which made it feel like the controls weren't sticking. Tweaks are now serialised to `localStorage["divora.tweaks"]` on every `setTweaks` call, partial-merged onto defaults on init so adding a new tweak field in a future phase doesn't blow up old payloads.
- **No global `--mystical` CSS variable.** Mystical was a JS-only signal consumed exclusively by the SpellCircle. Added `--mystical: <0..1>` and a discrete `data-mystical="subtle|balanced|rich"` root attribute so future components can react without prop-drilling.

### Tests

- **Frontend**: 175 → 179 (+4).
  - 1 updated Phase 6 test (default mystical is 0.7, not 1).
  - 4 new tests in a `Phase 11.1 tweak persistence + mystical values` group: default mystical equals 0.7; `setTweaks` writes to `localStorage["divora.tweaks"]`; values persist across a store re-init; partial-merge tolerates old payloads missing newer fields.

### Why it matters

The Tweaks panel is the user's most visible knob for what the app *looks* like. When the knobs don't move anything outside one screen and reset every restart, the whole "personalise your spellcraft" pitch falls apart. v0.11.1 makes Mystical + Motion actually reach the rest of the app, persist your choices, and start from a centred default the way the prototype always intended.

## [0.11.0] — 2026-05-29 — Phase 11: live device switching + cast polish + soundboard verification

Three field reports rolled into one polish phase.

### Added

- **Live input + output device switching.** Picking a different device in Settings now restarts the engine onto the new device automatically. The store grows a `createEffect(on([selectedInput, selectedOutput], …))` with `defer: true` and an `engineRunning` guard — if the engine is running and either selection actually changed, it stops + starts within one effect tick. No-op when stopped, and re-selecting the same value is a no-op too.
- **Glyph-cast trail of sparks.** `GlyphCastOverlay` emits two short-lived sparks per pointer-move event during a cast. Each spark has random ±0.8 px/frame velocity, gentle damping, a hint of gravity, and fades over 700 ms. A capped pool (96 sparks max) plus a single rAF loop keep the effect cheap.
- **"◆ SPELL CAST ◆" ceremonial reveal** (`SpellCastReveal` component) — after a successful cast, the bound preset's glyph blooms in its brand colour with a breathing radial glow, the preset name displays in the same colour, the `Bundled / User` tag sits below in mono. Three CSS keyframes drive the choreography: `spell-cast-veil` (backdrop fade), `spell-cast-reveal` (panel pop-in + hold + fade), `spell-cast-breathe` (glyph glow pulse). Auto-dismisses after 1.4 s.
- **Soundboard-mic confirmation hint** on the Soundboard header (only shown once a folder is picked): a small `info`-toned chip reading *"Clips play through your selected output device — including your modulated mic, so Discord / Zoom / OBS callers hear them."*
- **Engine architecture clarification** — the `chain.process` + `soundboard.mix_into` pair was extracted into a free function `mix_voice_and_soundboard(mono, chain, soundboard, sr)`. The output callback now calls that helper, and a doc comment on the helper explains *why* the order matters (effects on the user's voice; clips on top, as-is; both into the same mono buffer the fan-out consumes). The new function is unit-testable in isolation.

### Tests

- **Rust**: 102 → 104 (+2).
  - `audio::engine::soundboard_clips_land_in_the_same_output_buffer_as_the_mic` — feeds a 4096-sample constant-0.25 clip + a 480-sample mic buffer of constant 0.10 into `mix_voice_and_soundboard`; expects every output sample to equal 0.35 (additive mix). Direct regression for "do soundboard clips reach the modulated output?"
  - `audio::engine::mic_only_passes_through_when_no_clip_is_playing` — same scaffolding without the clip; expects untouched 0.10 throughout.
- **Frontend**: 167 → 175 (+8).
  - 4 store tests (`app store — Phase 2`): changing `selectedInput` while running restarts the engine on the new device; same for `selectedOutput`; no-op when stopped; no-op when the value didn't actually change.
  - 4 `SpellCastReveal` component tests: renders preset name + tag + "SPELL CAST" eyebrow; applies the preset's brand colour to the name; fires `onDone` after `REVEAL_DURATION_MS`; is keyboard-accessible via `role="status"` + `aria-live="polite"` + an aria-label naming the preset.

### Architecture notes

- **Why a store-level effect for device switching, not a Settings-component effect**: the engine restart must happen whether the user changes devices via Settings, an automated test, or a future hotkey. Putting it in the store keeps the rule in one place.
- **Why sparks + reveal are two separate components** instead of one mega-overlay: `GlyphCastOverlay` is for input (drag + classify). `SpellCastReveal` is for output (animation + preset announcement). They never coexist (overlay closes before reveal starts), so coupling them would only complicate the lifecycle. Plus the reveal is now reusable — Phase 12 (AI voice conversion) might surface a similar reveal when a converted-voice preset matches.
- **Why an info chip about soundboard routing** rather than a tooltip: users were *uncertain* whether clips reached call participants. A tooltip on a tile wouldn't surface unless they hovered. The chip sits at the top of the screen the moment a folder is picked, where the question naturally arises.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (104)
- `pnpm typecheck` — pass
- `pnpm test` — pass (175)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The device-switching bug was the largest "does this app even work?" issue still present in v0.10. Now picking a new mic Just Works. The cast reveal is the missing celebratory beat that turns gesture recognition from "did it pick the right preset?" into "yes, *Hollow King*." And the soundboard chip closes a documentation gap that was causing real confusion in calls.

## [0.10.0] — 2026-05-29 — Phase 10: polish (RNNoise denoiser + README rewrite)

### Added

- **RNNoise-based denoiser effect** (`divora-core::dsp::denoiser`). Wraps the pure-Rust `nnnoiseless` port (BSD-3-Clause, derived from Xiph's RNNoise). Streaming pipeline:
  - Accumulate native-rate input into a `VecDeque`.
  - Every full 480-sample frame (= 10 ms at 48 kHz), hand to `DenoiseState::process_frame`. Push the denoised result onto an output queue + a delay-matched dry copy.
  - On each call, write the head of the output queue back into the buffer with the wet/dry mix; output silence during the sub-frame warm-up so the dry signal can't audibly repeat once the wet stream catches up.
  - Hard 48 kHz constraint (model assumes that rate); off-rate input bypasses with no allocation. Users with a 44.1 kHz mic should pick a 48 kHz device for now — the rubato resampler around the denoiser is a future polish job.
- **`Denoiser` effect kind** registered in `EffectKind` + `EffectChain::build_effect`. Distinct from `Gate` (the hysteresis-threshold effect) — both can stack.
- **Frontend EFFECTS catalog entry** for `denoiser` with the `shield` sigil, a single `mix` param (0–100 %, default 80 %), and a description that names the 48 kHz / 10 ms latency caveats. `EFFECT_ORDER` places denoiser between `gate` and `pitch` so the chain runs cleaning steps first.
- **Wire types**: `EffectKindWire` adds `"denoiser"` to mirror the backend serde rename.

### Changed

- `divora-core::dsp::mod` — `EffectKind` gains the `Denoiser` variant; `build_effect` constructs a `RnnDenoiser` for it.
- `divora-core/Cargo.toml` — `+nnnoiseless = "0.5"` with `default-features = false` (we don't need the CLI deps that the `bin` feature pulls in).
- **README rewrite**. The prior README still claimed `"Status: Phase 0 — scaffold. Not yet usable."` Restored truthfulness: shipped feature list, install steps from the Releases page, SmartScreen workaround, build prerequisites (now including cmake for libopus + libnnnoiseless), architecture pointer.

### Tests

- **Rust**: 95 → 102 (+7).
  - 7 `dsp::denoiser`: passthrough when disabled, off-rate bypass, sub-frame chunks silent until frame is full, post-warmup output is non-zero and finite, disable clears the pipeline so re-enable starts in warm-up, `mix` clamps to 0..1 + unknown-key ignore, `mix = 0` leaves the buffer untouched.
- **Frontend**: 165 → 167 (+2).
  - 2 `EFFECTS catalog` (denoiser has a `mix` param in 0..100 with `%` unit; denoiser sits between gate and pitch in `EFFECT_ORDER`).

### Architecture notes

- **Why a learned model on top of the noise gate**: the Phase 3 noise gate silences input below a hard threshold — surgical but binary. `RNNoise` works on the *spectrum* of speech, suppressing background components without crushing dynamics. Stacking gate (chops the truly-silent moments) then denoiser (cleans what's left) gives the cleanest output without either step doing too much on its own.
- **Why 48 kHz only for v1**: `nnnoiseless` is a port of an RNN trained on 48 kHz data with a fixed 480-sample frame and a hard-coded filterbank that assumes that rate. Running off-rate input through the model's filterbank produces unstable output. Our existing `MonoResampler` from Phase 9 could bridge the rate, but each "44.1 → 48 → process → 48 → 44.1" round adds latency + cost. For Phase 10 we keep it simple: bypass at non-48 kHz, document the constraint. Phase 11 polish can revisit.
- **Why 10 ms warm-up silence (instead of passthrough)**: the denoiser produces output for *frame N* once it has *frame N* of input. If we passthrough during accumulation, those input samples are emitted as dry audio — and then the denoised version of the same samples arrives one buffer later, audibly repeating the audio. Silence during warm-up is a one-time event when the engine starts (or when the user toggles denoiser on); the user hears ~10 ms of nothing, then crisp audio.
- **Why `mix = 0` shortcut**: the wet path costs CPU and queues memory. When the user wants the dry signal anyway, skip the wet pipeline but still queue inputs through `drain_frames` so `state` stays warm — toggling mix > 0 mid-session doesn't have to reseed the model.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (102)
- `pnpm typecheck` — pass
- `pnpm test` — pass (167)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

`RNNoise` makes the voice-modulation pipeline actually sound clean in calls. The previous gate-only path passed everything above the threshold straight through, so room noise / fan hum / mechanical-keyboard chatter rode along with the voice. Denoising at the head of the chain means every downstream effect (pitch, formant, reverb) operates on a clean signal — pitch shifts of noise no longer sound like wobbly noise; reverb on noise doesn't sound like a foggy room with a person in it.

The README rewrite is the second half of Phase 10's "polish" mandate: the project page now matches what's actually shipped. v1.0 is a quiet patch cycle away.

## [0.9.0] — 2026-05-29 — Phase 9: DSP quality (real pitch + formant + rubato resampling)

### Added

- **Real phase-vocoder pitch shifter** (`divora-core::dsp::pitch`) replaces the v0.3.1 passthrough stub. Runs a 1024-sample / 256-hop Hann-windowed STFT; for each frame:
  1. Compute the *true instantaneous frequency* per analysis bin from the actual phase advance vs. the expected hop advance.
  2. Build the synthesis spectrum by sampling magnitude at `k_out / ratio` (linear interpolation between adjacent bins).
  3. Evolve the per-bin synthesis phase by `(true_freq × ratio) × 2π × HOP / sr` so the output stays phase-coherent across hops.
  4. ISFFT and overlap-add.
  The "hearing yourself twice" varispeed bug from v0.3.0 can't recur — the vocoder never pulls from two unrelated points in time. Up-shift by 12 st measurably doubles the steady-state zero-crossing rate of a 440 Hz sine; down-shift by 12 st halves it. Zero-shift and disabled both still bypass to bit-identical passthrough.
- **Formant shifter via spectrum warping** (`divora-core::dsp::formant`) replaces the three-bandpass coloration. For each STFT frame:
  1. Estimate the spectral envelope by moving-averaging the magnitude spectrum on the frequency axis (33-bin Hann-style smoother).
  2. Compute the excitation as `magnitude / envelope` — the harmonic fine structure.
  3. Warp the envelope on the frequency axis by the formant ratio (linear interp).
  4. Re-impose the original excitation on the warped envelope.
  Result: vowel colour darkens / brightens *without* the fundamental moving. Pure sines pass straight through (no formants to warp); a formant-shifted 440 Hz tone retains a 440 Hz fundamental within ±25 % of zero-crossing rate.
- **Streaming STFT helper** (`divora-core::dsp::stft`) — shared by pitch and formant. Pre-allocated Hann window + realfft analysis / synthesis + overlap-add output ring. The user supplies a closure that mutates the `(magnitude, phase)` of each frame.
- **`MonoResampler`** (`divora-core::audio::resampler`) wraps `rubato::SincFixedOut` with a `push_input` / `process` API. 128-tap sinc, Blackman-Harris window, 0.95 cutoff, 128× oversampling — high-fidelity but realtime-friendly (allocates only at construction).
- **Sample-rate mismatch is no longer a hard error.** The engine constructs a `MonoResampler` whenever input and output device rates differ; DSP runs at the input rate and the resampler bridges to the output rate just before fan-out. The `SampleRateMismatch` error variant remains for backward compatibility but is no longer produced. A new `ResamplerBuild` error covers the (rare) case where rubato refuses a particular rate pair.

### Changed

- `divora-core::dsp::mod.rs` — registered the new `stft` submodule alongside the existing eight effects.
- `divora-core::audio::mod.rs` — registered `resampler` submodule + re-exported `MonoResampler`. New `ResamplerBuild { input, output, message }` error variant.
- `divora-core::audio::engine` — `build_output_stream` now takes both `input_rate` and `output_rate`; the output callback chooses between direct passthrough and `MonoResampler::process` based on whether the rates match. DSP runs at the input rate either way.
- `divora-core::Cargo.toml` — `+rubato = "0.16"`, `+realfft = "3"`.

### Tests

- **Rust**: 80 → 95 (+15).
  - 4 `audio::resampler`: identity 48k → 48k, 44.1k → 48k produces ~48k frames, 48k → 44.1k produces ~44.1k frames, `reset` clears pending without panicking.
  - 4 `dsp::stft`: identity modifier reconstructs input after warm-up, DC stays finite, zeroing the spectrum silences output, `reset` brings output back to zero.
  - 6 `dsp::pitch` (replacing 5 passthrough tests): zero-shift bit-identical bypass, disabled bypass, +12 st doubles the dominant frequency of a 440 Hz sine, −12 st halves the dominant frequency of an 880 Hz sine, clamp + unknown-key behavior, DC stability.
  - 7 `dsp::formant` (replacing 1 stability test): zero-shift bypass, disabled bypass, shifted output stays finite, DC under shift stays finite, formant shift does NOT move the fundamental frequency (sanity-check the whole point of formant shifting), `smooth_spectrum` preserves DC, clamp.
- **Frontend**: 165 (unchanged).

### Architecture notes

- **Why a phase vocoder instead of WSOLA / PSOLA**: WSOLA gives slightly better quality on voice at small shifts but needs a pitch detector to lock the OLA cut points, and the cut-point search is non-trivial to test. The phase vocoder is the textbook algorithm, well-covered by acceptance tests (frequency doubling / halving), and shares all of its STFT infrastructure with formant warping.
- **Why moving-average envelope instead of LPC**: LPC envelope warping is the "right" formant algorithm but adds Levinson-Durbin recursion + residual computation + re-synthesis — a lot more code with comparable end-result quality for ±12 st shifts on speech. We can swap in LPC later without breaking the public effect surface.
- **21 ms latency budget**: the STFT window is 1024 samples ≈ 21 ms at 48 kHz. Added on top of the ~5 ms cpal buffer this puts the engine at ~26 ms end-to-end — still under the 30 ms Phase 2 goal. The bypass-at-zero-shift policy means uneffected paths see zero added latency.
- **Resampler placement**: chose to run DSP at the input rate (not at a fixed internal 48 kHz) because input rates ≥ 44.1 kHz are universally common and the resampler's quality cost only applies to the output side. Resampling at the internal boundary would have meant resampling *every* signal regardless of device match.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (95)
- `pnpm typecheck` — pass
- `pnpm test` — pass (165)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

Phase 9 turns three "looks present but doesn't actually do anything" sliders into three working DSP effects. Pitch finally shifts pitch without doubling the voice; formant finally moves vowel colour without pitch; and "your mic and speakers are running different rates" stops being a wall the user hits — the engine just resamples. The phase vocoder + rubato infrastructure also unlocks future work (better quality formants via LPC, on-device AI voice conversion in Phase 11) without further architectural change.

## [0.8.2] — 2026-05-29 — Chain-card drag in Presets editor actually works

### Fixed

- **Effect-card drag-reorder in the Presets editor was non-functional.** The card was `<div draggable={true}>` but it contained interactive children (the effect Toggle button + parameter Sliders). HTML5 drag refuses to initiate when the pointer-down lands on any interactive descendant, so dragging anywhere except the small drag-handle sigil silently did nothing — and even *that* was unreliable in WebView2.

### Changed

- `PresetsScreen` `ChainCard` switched to the explicit drag-handle pattern:
  - The card itself drops `draggable={true}` and only acts as a **drop target** (`onDragEnter` / `onDragOver` / `onDragLeave` / `onDrop`).
  - The drag-handle `<span>` (the existing drag sigil) takes on `draggable={true}` + `onDragStart` + `onDragEnd`, plus `role="button" tabindex={0}` and an explicit `aria-label` ("Drag to reorder *<effect>*. Currently at position *N* of *M*."). The user must grab the visible drag handle to initiate — which matches the existing visual affordance.
- Same WebView2-friendly polish as v0.8.1: cursor switches `grab → grabbing` while dragging; 0.6 opacity on the dragging source; custom MIME `application/x-divora-chain-index` in addition to `text/plain`; `onDragLeave` ignores events that bubble up from descendants (no border flicker as the cursor crosses inner spans).

### Tests

- **Frontend**: 162 → 165 (+3).
  - `PresetsScreen.test.tsx` (new):
    - Card itself is NOT directly draggable (regression catch for the Phase 4 / v0.8.0 implementation).
    - The drag handle `<span>` carries `draggable="true"` + `role="button"` + an aria-label that mentions "drag".
    - Dispatching `dragstart` on the handle of card 0 + `drop` on card 2 actually reorders the chain through `app.reorderChainEntries` — the first entry slides to position 2, the others shift up.

### Architecture notes

- **Why drag handles instead of "draggable card + draggable={false} on every child"**: the second pattern works in theory but breaks down when the card grows new interactive elements (every future Toggle / Slider / Select inside a ChainCard would have to remember the opt-out). The drag-handle pattern moves the affordance to a single named place that's already visually distinguished. Net cost is the user *must* grab the handle — which is what the visual design wanted anyway.

### Why it matters

The chain editor is the whole point of the Presets screen — without working reorder, "Save with the runes in a different order" was a UI lie. The fix is small (~30 LoC) but unblocks the screen's promise.

## [0.8.1] — 2026-05-29 — Tile drag-and-drop reorder actually works in WebView2

### Fixed

- **Soundboard tile drag-reorder was non-functional in the v0.8.0 build.** Two issues stacked: (1) tile containers were `<button draggable={true}>`, and Chromium / WebView2 (Tauri's Windows webview) often refuses to initiate a drag from a `<button>` element. (2) Even when drag did fire elsewhere, the synthetic `click` event Chromium dispatches on the drop target after a successful drop ran the tile's `onClick` and immediately played the clip — masking whether reorder had happened.

### Changed

- Tile container converted from `<button type="button">` to `<div role="button" tabindex={0}>` + `onKeyDown` for Enter/Space, keeping the tile keyboard-operable. `cursor: grab` (and `grabbing` while dragging) makes draggability discoverable; a 0.6 opacity hint on the source tile during the drag mirrors what users expect from sortable lists.
- Drag payload now uses a custom MIME (`application/x-divora-tile-index`) in addition to `text/plain` — Chromium's `getData` is occasionally finicky about the plain-text path; the custom MIME survives every Tauri build we've tested.
- Click suppression window: a 300 ms post-drop timer + a post-`dragend` timer block the next `click` from firing `playClip`. `onDragLeave` now ignores events that bubble up from descendants (the old behavior caused flicker as the cursor crossed inner spans).
- Tile `aria-label` is explicit about all three affordances ("Play *bell*. Right-click for color, drag to reorder.").

### Tests

- **Frontend**: 158 → 162 (+4).
  - `SoundboardScreen.test.tsx` (new):
    - Tiles render as `div[role="button"][draggable="true"]` (NOT `button[draggable]`) — direct regression catch for the v0.8.0 root cause.
    - Aria-label mentions both play and drag.
    - Dispatching a synthetic `dragstart` on tile 0 + `drop` on tile 2 reorders `[a,b,c]` → `[b,c,a]` via `app.reorderTiles`. jsdom doesn't fire real HTML5 drag, but it accepts the explicit event sequence the browser would generate.
    - The synthetic post-drop `click` Chromium dispatches does NOT fire `play_soundboard_clip` (suppression window works).

### Why it matters

Without working tile reorder, the soundboard layer of Phase 8 — drag to organise — was effectively dead in the user-facing build despite the store logic being correct. The store tests passed because they call `reorderTiles` directly; the missing test coverage was the UI wiring on top. This patch adds that coverage.

## [0.8.0] — 2026-05-29 — Phase 8: cast alignment + soundboard polish

### Added

- **Glyph cast trace now lines up with the cursor.** `GlyphCastOverlay.tsx` translates every pointer event through a new `localPoint(svg, e)` helper that subtracts the SVG's `getBoundingClientRect()` top-left from the event's viewport coordinates. The SVG `viewBox` is set from a `size()` signal that tracks the SVG's actual rendered dimensions (re-measured on mount, after each pointerdown, and on window resize) instead of the previous `window.innerWidth × window.innerHeight` viewport hack — so 1 SVG unit = 1 CSS pixel and the trace stays under the cursor regardless of titlebar / sidebar offset.

- **Soundboard tile drag-reorder, persisted per folder.** Each `<Tile>` is HTML5-draggable; drop on another tile reorders the local sequence. Order is saved as a `tileOrder: Record<folderPath, clipId[]>` map in `localStorage["divora.tileOrder"]`. New tiles that appear in a later scan (files added since the saved order) fall to the end in scanner-default alphabetical order via the new `sortedTiles()` memo.

- **Per-tile color palette.** Right-click a tile to open a small popover with 8 brand-aware swatches (Indigo / Pink / Cyan / Emerald / Gold / Crimson / Lilac / Slate) plus a "Reset to default" button. Selection persists to `localStorage["divora.tileColors"]` and overrides the default per-id color hash on the active tile (border glow, status dot, progress ring, countdown).

- **Recent folders dropdown.** Header gains a "Recent" ghost button next to "Change folder" that opens a menu of up to 5 most-recently-used folders. Click an entry to switch folders instantly (no native picker round-trip). Per-entry × button removes one. Backed by `localStorage["divora.recentFolders"]`, MRU-ordered, capped at 5.

- **Global per-tile hotkeys via `tauri-plugin-global-shortcut`.** `bindTileHotkey(clipId, keys)` now registers each binding under the `sb:<clipId>` id namespace so soundboard clips fire even when DivoraVoice isn't the focused window. `clearTileHotkey` unregisters. `syncHotkeyBindings` (run on `App.tsx` mount) re-registers every persisted tile binding so they survive restarts. The Phase 5 in-app `keydown` listener was removed — the global path covers both focused and unfocused use and removes the double-fire risk.

### Changed

- Store: new exports `tileColors` / `setTileColor`, `tileOrder` / `reorderTiles`, `sortedTiles`, `recentFolders` / `pushRecentFolder` / `removeRecentFolder` / `useRecentFolder`, `playTileById`. `bindTileHotkey` + `clearTileHotkey` are now async (they await the global-shortcut register / unregister calls).
- Persistence helpers `loadJson`/`saveJson` + `STORAGE_KEYS` constant centralise localStorage I/O for Phase 8 metadata.
- `pickSoundboardFolder` now pushes the chosen folder to recents.
- `App.tsx` global-shortcut dispatcher routes any event whose `id` starts with `sb:` to `playTileById(id.slice(3))`.

### Tests

- **Frontend**: 136 → 158 (+22).
  - 4 new `GlyphCastOverlay.localPoint` tests (offset SVG, origin-anchored SVG, null-ref fallback, fractional offsets).
  - 18 new store tests (sortedTiles passthrough + reorder, new-tile append, persistence, no-op clamping; `setTileColor` set + clear + persistence; recent folders MRU/cap/dedup/remove; `useRecentFolder` scans; `pickSoundboardFolder` re-uses pushRecentFolder; `bindTileHotkey` sb:-prefixed registration + clear; `syncHotkeyBindings` re-registers persisted tile hotkeys; `playTileById` looks up by id and no-ops cleanly when missing).
- **Rust**: 80 (unchanged — Phase 8 is frontend / store only).

### Architecture notes

- **Why localStorage for tile metadata (not a Tauri app-config plugin)**: tile order + colors + recent folders are pure UI preferences. If they vanish, the next scan still works (orders restore to alphabetical, colors restore to the per-id default hash, recents restore to whatever the user picks next). The state never needs to be migrated, encrypted, synced, or backed up — file-based storage would just add a Tauri command surface and a new permission for no user-visible win.
- **Why `sb:<clipId>` instead of just the clipId**: the global-shortcut backend keeps a single id → Shortcut map shared between the PTM/panic/monitor namespace and every tile. Prefixing tile ids prevents an unlikely future collision and gives the dispatcher in `App.tsx` a free fast-path discriminator (`event.id.startsWith("sb:")`).
- **Why the in-app tile-hotkey listener was removed**: in Phase 7's model, the in-app `keydown` listener AND the global-shortcut handler would both fire whenever DivoraVoice was focused, leading to double `playClip` calls. Removing the in-app listener and trusting the global hotkey path keeps both modes (focused / unfocused) playing exactly one voice.
- **Cast alignment fix is purely a coordinate-frame change** — no classifier work needed because the classifier is translation-invariant (it operates on the trace's bounding box and turning angles, not absolute positions).

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (80)
- `pnpm typecheck` — pass
- `pnpm test` — pass (158)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The cast trace lining up makes the spellcraft gesture feel like it's actually responding to the user, not approximating. The soundboard set — drag to organise, color to organise visually, hop between sound folders without re-picking, hotkey-trigger clips while you're in a game/call — turns the screen from "browse and click" into a real performance surface. The hotkey work is the most consequential: it's the difference between "remember to alt-tab back" and "trigger the bell-sound mid-conversation."

## [0.7.1] — 2026-05-29 — Field bugs from v0.7.0: PTM steal, blank sigils, no scroll

Four reports from v0.7.0 that, taken together, made the app feel broken in normal use.

### Fixed

- **Push-to-modulate stole Space from every other app.** `DEFAULT_HOTKEY_BINDINGS.ptm` was `"Space"` and `App.tsx` called `syncHotkeyBindings()` on mount, which registered Space via `tauri-plugin-global-shortcut` — capturing the key system-wide so Discord, browsers, and games never saw it. Defaulted PTM to `""`. The in-app focused-window listener still handles Space because `ui.ptmKey` keeps its `"Space"` default, so PTM works while DivoraVoice is the active window without burning the key globally. Users who want a true OS-wide PTM can still bind one explicitly in Settings → Hotkeys.
- **Mixer icon in the sidebar was blank.** SolidJS gotcha — the `SIGILS` map in `src/components/Sigil.tsx` stored *pre-evaluated* JSX elements (real DOM nodes built at module load). When two `<Sigil name="mixer">` components were mounted simultaneously (the sidebar nav + the wizard's "Real-time" pillar card), they shared a single DOM node; Solid moved it to the most recently mounted location and the earlier site went empty. Converted every entry in `SIGILS` to a factory (`() => JSX.Element`) and invoke at the use site (`SIGILS[props.name]()`), so each `<Sigil>` instance gets its own freshly built subtree.
- **Presets icon disappeared after clicking Settings.** Same shared-DOM root cause as the Mixer bug — Settings → Glyph Casting renders four `<Select icon="presets">` rows, each of which mounts a `<Sigil name="presets">`. Navigating to Settings instantiated them and yanked the shared DOM node away from the sidebar. Fixed by the same `SIGILS` factory conversion.
- **Scrolling didn't work in any tall screen** (Settings, Presets editor, etc.). `#root` had no CSS rules; the App's outer `<div>` set `height: 100%`, which then had no parent height to resolve against and collapsed the entire flex chain. Tall screens rendered at content-size, which exceeded the viewport, and `body { overflow: hidden }` clipped the excess silently. Added `#root { height: 100% }` to anchor the chain. (The v0.6.1 SoundboardScreen restructure fixed the *internal* layout there; this is the missing *outer* anchor that affected every other screen.)

### Tests

- **Frontend**: 129 → 136 (+7).
  - 3 new Sigil tests: two `<Sigil>` instances with the same name render independent SVG subtrees (the regression catch for bugs 2 + 3) for both `presets` and `mixer`, plus a mixed-glyph render test.
  - 2 new `styles.css` source-level tests: `#root { height: 100% }` is present (the regression catch for bug 4); `html, body { height: 100% }` + `body { overflow: hidden }` survive future edits.
  - 2 updated store tests: `hotkeyBindings.ptm` now defaults to `""` (regression catch for bug 1) and `syncHotkeyBindings` with an empty default produces zero `register_global_shortcut` calls. Added a positive test that `ui.ptmKey` still defaults to `"Space"` so the in-app fallback works.
- **Rust**: 80 (unchanged — no Rust changes in this patch).

### Build / tooling

- Added `@types/node` devDependency so test files can use `node:fs` / `node:path` / `node:url` for source-level assertions like the styles.css inspection.

### Architecture notes

- **Why `SIGILS` as a factory map and not a `<Switch>` of literals**: factories keep every glyph definition co-located with its name in a single object literal, which is what `SIGIL_NAMES` enumerates for tests and pickers. A `<Switch>` would scatter the same definitions and break that enumeration.
- **Why the in-app PTM fallback is enough as the default**: Discord / OBS / browsers are where users want PTM to feel responsive, and DivoraVoice has to be focused to do anything useful (you're picking a preset, adjusting effects, or watching the spell circle). If you Alt-Tab away DivoraVoice goes quiet, which is the expected behaviour. Users who genuinely want PTM-while-game-focused are a smaller cohort and they can bind a modifier-chord (Ctrl+Space, Right Alt) in Settings.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (80)
- `pnpm typecheck` — pass
- `pnpm test` — pass (136)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

v0.7.0 was technically feature-complete but felt broken: the wizard launched, the sidebar showed blank icons, you couldn't scroll Settings, and Space was hijacked from your other apps. Four small fixes restore the baseline experience. Phase 8 picks back up with new features (likely soundboard hotkeys / tile reordering / additional DSP) once we've confirmed v0.7.1 is solid.

## [0.7.0] — 2026-05-29 — Phase 7: first-run wizard + glyph casting

### Added

- **First-run wizard** (`src/components/Wizard.tsx`): a four-step ceremony shown automatically the first time DivoraVoice is launched.
  - **Welcome** — display headline, four pillar cards (Local-first / Private / Free / Real-time).
  - **Virtual cable** — auto-refreshes `virtualMicStatus` on entry; emerald banner when VB-Cable is detected, gold warning + Download button (opens `https://vb-audio.com/Cable/` via the Tauri shell plugin) when missing. Re-scan ghost button.
  - **Devices** — auto-refreshes inputs + outputs on entry; Microphone/Modulated-out selects backed by the same store as Settings; live HMeter under "Hearing you".
  - **Ready** — emerald checkmark, "You're ready" headline, Discord routing instructions card.
  - Ceremonial left rail with the DMark, a breathing radial-gradient sigil that swaps icon per step (`clean` → `output` → `mic` → `modulated`), and a step pill list with check-marks for completed steps.
  - Persistent first-run gating via `localStorage["divora.wizardSeen"]`. Finish or skip writes the flag; `Settings → About → Replay setup` clears it and re-opens.
- **Glyph casting on the Mixer** — pointer-trace gesture that switches the active preset based on which of four glyphs (▲ ▽ □ ○) the user draws.
  - New `src/data/glyphs.ts` classifier:
    - `dedupe` / `boundingBox` / `pathLength` / `resamplePath` (uniform arc-length resampling to 48 points) / `smooth` (3-tap moving average, off by default) / `turningAngles` (cyclic-aware) / `findCorners` (cyclic peak detection with minimum-separation dedup) / `endpointGap` (closed-path detector).
    - `classifyGlyph(points, config?)` → `"triangle" | "invtriangle" | "square" | "circle" | null`. Pipeline: dedupe → bbox check → resample → smooth → check closedness → cyclic turning-angle pass → cyclic corner search. Decision tree: 0 corners ⇒ circle; 4 corners ⇒ square; 3 corners ⇒ triangle vs invtriangle via median-Y apex test (the corner whose Y is most distant from the median Y is the apex; apex above the median ⇒ ▲, apex below ⇒ ▽). Open paths and corner counts outside {0, 3, 4} return null.
  - New `src/components/GlyphCastOverlay.tsx` — full-screen pointer-capture surface with a glowing dusk-violet stroke that traces the user's path. Floating instruction card while idle; Escape cancels.
  - `MixerScreen.tsx` — new "Cast" button in the preset header opens the overlay; pressing `G` (when no field is focused) also opens it. On classification, calls `app.usePreset(app.glyphs[shape])` and surfaces a 2.4 s pill-shaped flash toast (`{glyph} → {preset name}` on success, "Glyph not recognised — try again" on null, "No preset bound to {glyph}" when the glyph maps to a missing preset).

### Tests

- **Frontend**: 105 → 129 (+24).
  - 17 new classifier tests: `dedupe` / `boundingBox` / `pathLength` / `resamplePath` / `smooth` / `turningAngles` / `findCorners` / `endpointGap` + classify a clean circle, a noisy circle, a square, an upright triangle, an inverted triangle, plus null cases (tiny trace, too few points, single direction change) and custom-config overrides.
  - 7 new wizard tests: first-launch renders welcome step, seen-flag auto-closes on mount, `clearWizardSeenFlag` erases the flag, Continue advances through every step + finish writes seen flag, Back walks backward, Skip closes and marks seen, opening the wizard re-runs the device + virtual-mic refresh effects.

### Architecture notes

- **Why a custom classifier instead of a library**: the four target glyphs are simple closed convex shapes and the user is drawing them quickly with a mouse / finger. The full `$1 Unistroke Recognizer` template-matching pipeline (rotation invariance, golden-section search) would be overkill; corner-counting on a cyclic turning-angle series catches every target with one tunable threshold. The classifier ships as a pure-function module so we can extend it (e.g. add pentagon / spiral) without touching React/Solid code.
- **Why cyclic angles**: a glyph trace starts and ends at the same point; the corner at the seam would otherwise vanish because the endpoint-trim left both ends at 0 turning angle. Cyclic mode wraps neighbour lookups so the seam corner registers like any other, and the cyclic dedup in `findCorners` collapses the inevitable double-detection at the wrap.
- **localStorage rather than the Tauri app-config plugin**: the wizard seen-flag is a UI preference, not user data. It's fine if it disappears (re-running setup is harmless), and we avoid pulling in another Tauri plugin / capability just for one boolean.
- **`G` hotkey lives in `MixerScreen.tsx`, not the global shortcut store**: glyph casting only makes sense while the Mixer is visible. The local listener is cheap, scoped, and won't fight with system-level hotkeys the user may have bound to G.

### Pre-push checklist (local, 2026-05-29)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (80 in divora-core)
- `pnpm typecheck` — pass
- `pnpm test` — pass (129)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The wizard turns "I downloaded this app and don't know where to start" into a three-minute guided rite that ends with audio actually routing into Discord. Glyph casting turns the Mixer from a static preset picker into a tactile spellcraft surface — draw a triangle, the chain swaps. Both features were on the design's must-have list since Phase 1; v0.7.0 makes them real.

## [0.6.1] — 2026-05-28 — Soundboard scroll + OGG-Opus decode

### Fixed

- **Soundboard scrolling never engaged when more tiles existed than fit on screen.** The `flex:1 / overflow:auto` lived on `TileGrid`, nested under two `<Show>` blocks that broke its flex sizing. Lifted the scroll container above the Show chain so it's the same DOM element regardless of which branch (folder picked / loading / no tiles / tiles) renders inside.
- **OGG-Opus files (Discord voice clip exports) failed to play with `unsupported feature: core (codec):unsupported codec`.** symphonia 0.5 demuxed the OGG container fine but had no Opus decoder. Upgraded to symphonia 0.6, switched to its new `Probe`/`CodecRegistry`/`AudioDecoder` API, and registered `symphonia-adapter-libopus`'s `OpusDecoder` alongside every default codec. libopus is vendored via the adapter's `bundled` feature (`opusic-sys` builds it from source on the Windows MSVC runner; cmake is now an explicit step in CI/Release workflows).

### Changed

- `divora-core::soundboard::decoder` — full rewrite for symphonia 0.6:
  - Custom `Probe` (via `register_enabled_formats`) and `CodecRegistry` (via `register_enabled_codecs` + manual `OpusDecoder` registration), both behind `OnceLock`s.
  - `probe.probe()` (replaces `format()`), `format.default_track(TrackType::Audio)`, `format.next_packet()` now returns `Result<Option<Packet>>`, `track.codec_params.audio()`, `make_audio_decoder(audio_params, &opts)`.
  - Decoded buffers come back as `GenericAudioBufferRef` — we call `copy_to_vec_interleaved::<f32>` (symphonia handles every integer → float normalisation internally), then mix to mono.
- `.github/workflows/ci.yml` + `release.yml` — added `lukka/get-cmake@latest` before the rust toolchain step so libopus's cmake-driven build never breaks if the runner image's cmake moves.

### Tests

- **Rust**: 76 → 80. New decoder unit tests:
  - `codec_registry_includes_opus_decoder` — regression for the exact v0.6.0 bug; asserts the registry no longer returns "unsupported codec" for `CODEC_ID_OPUS`.
  - `codec_registry_keeps_all_default_audio_decoders` — sanity check that adding Opus didn't drop Vorbis / FLAC / MP3 / PCM.
  - `probe_registry_is_non_empty_after_seeding` — touches the probe `OnceLock` to confirm seeding doesn't panic.
  - `decode_clip_reports_a_friendly_error_for_missing_files` — verifies the "open" error path still surfaces a clean message.

### Why it matters

These two bugs were the lived experience of v0.6.0 for anyone trying to use the soundboard with a folder of more than ~6 clips OR with Discord recordings (which are the most common ".ogg" files in the wild). Both fixes are tiny in code but huge in usability — they're the difference between "neat demo" and "actually works."

## [0.6.0] — 2026-05-28 — Phase 6: virtual mic + global hotkeys + full Settings

### Added

- **`divora-core::audio::virtual_mic` module** — `detect_virtual_mic()` walks both device lists looking for VB-Cable's canonical names (`CABLE Input (VB-Audio Virtual Cable)` / `CABLE Output (...)`), or the broader `VB-Audio … Input/Output` variant for HiFi Cable / Voicemeeter VAIO. Returns a `VirtualMicStatus { detected, cableInputDevice, cableOutputDevice, downloadUrl }` struct (camelCase wire). The download URL is a hard-coded constant pointing at https://vb-audio.com/Cable/.
- **`tauri-plugin-global-shortcut`** wired in. New `AppState.shortcuts: Mutex<HashMap<String, Shortcut>>` keyed by stable id (`ptm` / `panic` / `monitor`). The plugin's handler emits a Tauri event `global-shortcut` with payload `{ id, accelerator, state: "pressed" | "released" }` on every transition.
- **Tauri commands**: `detect_virtual_mic`, `register_global_shortcut(id, accelerator)`, `unregister_global_shortcut(id)`, `unregister_all_global_shortcuts`. The capability extension permits `global-shortcut:default`, `…allow-register`, `…allow-unregister`, `…allow-unregister-all`, `…allow-is-registered`.
- **Frontend audio API extensions** (`src/audio/api.ts`): `VirtualMicStatus`, `GlobalShortcutEvent` types + `detectVirtualMic`, `registerGlobalShortcut`, `unregisterGlobalShortcut`, `unregisterAllGlobalShortcuts`, `subscribeGlobalShortcut` wrappers.
- **Store**: `virtualMicStatus()` signal + `refreshVirtualMicStatus()` action; `hotkeyBindings` store (`{ ptm: "Space", panic: "", monitor: "" }`) + `setHotkeyBinding(action, accelerator)` / `clearHotkeyBinding(action)` / `syncHotkeyBindings()`. PTM bindings also update `ui.ptmKey` so the in-app keyboard fallback (used while the window is focused) tracks the same key.
- **`SettingsScreen.tsx`** filled out: keeps the Phase 2 Audio devices block and adds
  - **Virtual microphone** — emerald check + routing hint when detected, gold warning + Download button when missing. Re-scan button calls `refreshVirtualMicStatus`. Three call-app instruction cards (Discord / Zoom / OBS) appear once VB-Cable is detected.
  - **Hotkeys** — three rows (push-to-modulate / panic / toggle monitor) each with a `HotkeyCapture` chip set + a "Clear" button. Captured key arrays are joined with `+` into Tauri accelerator strings.
  - **Glyph casting** — Triangle / Inverted triangle / Square / Circle rows, each with a preset Select. Already-existing `app.glyphs` store from Phase 1 is the source of truth.
  - **Appearance** — Phase 1's mood / accent / motion plus Mystical (subtle / balanced / rich), Parchment grain toggle, Vignette toggle (all already in `app.tweaks`).
  - **About** — DMark + version v0.6.0 + "MIT License · Tauri + SolidJS" + GitHub link + three pillar cards (No telemetry / No account / Free forever) + Replay setup button that flips `wizardOpen` for the Phase 7 wizard.
- **`App.tsx`** subscribes to `global-shortcut` events on mount. PTM events drive `ui.pressed`; `panic` triggers `panicSoundboard`; `monitor` triggers `toggleMonitor`. The in-app Space-key fallback was generalised to read the current `ui.ptmKey` so re-binding PTM works without a window restart. `syncHotkeyBindings()` is called once after subscribe so persisted bindings survive a restart.
- **`openExternal(url)`** helper inside SettingsScreen — lazy-loads `@tauri-apps/plugin-shell` so the screen still renders in browser preview that lacks the bridge.

### Tests

- **Rust**: 70 → 76. New 6 tests on `virtual_mic.rs` (canonical cable input recogniser, canonical cable output recogniser, case-insensitive matching, rejects unrelated devices, accepts VB-Audio HiFi / Voicemeeter variants, `detect_virtual_mic` runs without panicking on CI hosts with no audio hardware).
- **Frontend**: 86 → 105. New 6 API wrappers (`detectVirtualMic` two variants, `registerGlobalShortcut` happy + error, `unregisterGlobalShortcut`, `unregisterAllGlobalShortcuts`, `subscribeGlobalShortcut`) + 13 store helpers (refresh status, swallow detect failure, default hotkeys, set + register, PTM dual-write to ui.ptmKey, empty accelerator → unregister, clearHotkeyBinding, register failure stays consistent, syncHotkeyBindings skips empty, Phase 6 tweak defaults, setTweaks updates, glyph defaults + setGlyphs).

### Architecture notes

- **Why bridge VB-Cable instead of writing our own kernel driver**: writing a virtual audio device for Windows means a signed WDF driver, an EV cert, and a months-long Microsoft attestation pipeline. We are a free OSS tool; we don't have that runway. VB-Cable is the de facto standard, is free for personal use, and works identically on every supported Windows release. Detection-only is the right scope.
- **Global vs in-app hotkeys**: `tauri-plugin-global-shortcut` is the only way to get the key while the window is unfocused, but on Windows it does *not* swallow the press from the focused app (a Discord/OBS chat box still receives the Space). The in-app keyboard listener stays as a focus-time backup that suppresses the default behaviour. Both paths converge on the same store action (`setUi("pressed", …)` / `panicSoundboard` / `toggleMonitor`).
- **Hotkey accelerator format**: we store and emit the Tauri-native string (`"Space"`, `"Ctrl+Shift+P"`). The `HotkeyCapture` component natively works with `string[]` (one chip per key); the SettingsScreen joins/splits on `+` at the boundary.
- **Glyph casting** is stored entirely on the frontend (`app.glyphs`). The cast → preset mapping fires on the Mixer (Phase 7 wizard wires the picker UX too); the backend never sees it.

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (76 in divora-core)
- `pnpm typecheck` — pass
- `pnpm test` — pass (105)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

Phase 5 ended with both voice and clips landing in the same engine output. Phase 6 makes that output reachable from another app: VB-Cable detection turns the "did the user install the bridge?" question into a one-glance answer, and the global-shortcut layer means push-to-modulate / panic / monitor work even when Discord, Zoom, or OBS has focus. Plus the full Settings screen is now the home for everything users need to configure end-to-end: devices, cable, hotkeys, glyph casts, look, and the about block. Phase 7 builds the welcome wizard that uses these signals to walk first-run users through setup.

## [0.5.0] — 2026-05-28 — Phase 5: soundboard

### Added

- **`divora-core::soundboard` module** — `scanner.rs` (`scan_folder`), `decoder.rs` (`decode_clip` via symphonia), `mixer.rs` (`SoundboardMixer` + `SoundboardCommand` + `PlayingClipSnapshot`), `mod.rs` (`SoundboardError`).
- **`scanner::scan_folder(&Path)`** walks the chosen directory (one level deep), picks out `.wav` / `.mp3` / `.ogg` / `.oga` / `.flac` / `.opus`, returns `SoundboardTile { id, path, label, extension, sizeBytes, modifiedSecs }` sorted by label. IDs are stable hex hashes of the canonicalised path so they survive renames.
- **`decoder::decode_clip(&Path)`** decodes any supported format into a mono f32 buffer plus the native sample rate. Multi-channel audio is summed to mono; integer PCM is normalised to `[-1, 1]`.
- **`SoundboardMixer`** — fixed-size 16-voice pool. Commands flow through SPSC mpsc from the engine; `play` steals the oldest voice when full, `stop` / `stop_all` deactivate matching voices. `mix_into(&mut [f32], engine_rate)` adds every active voice (with linear-interpolation sample-rate matching), so 44.1 kHz clips on a 48 kHz engine play at the right pitch instead of chipmunked.
- **AudioEngine integration**: new `Command::Soundboard(SoundboardCommand)` variant routes UI → engine_thread → live output callback via a fresh sb channel per stream start. The output callback now runs the DSP chain on the mic mono buffer, then mixes soundboard voices on top before fanning to channels. Effects apply only to the user's voice; soundboard clips play as-is.
- **`AudioEngine::send_soundboard(cmd)`** — non-blocking handle for soundboard commands.
- **Tauri commands**: `scan_soundboard_folder`, `play_soundboard_clip`, `stop_soundboard_clip`, `stop_all_soundboard_clips`. `play_soundboard_clip` returns the clip's duration in seconds and caches the decoded buffer (`Mutex<HashMap<String, DecodedClip>>` on `AppState`) so hot tiles never decode twice.
- **`tauri-plugin-dialog`** wired in for the native folder picker; capability extended with `dialog:default` + `dialog:allow-open`.
- **Frontend audio API extensions** (`src/audio/api.ts`): `SoundboardTile` wire type + `pickSoundboardFolder` (lazy-loads `@tauri-apps/plugin-dialog`), `scanSoundboardFolder`, `playSoundboardClip`, `stopSoundboardClip`, `stopAllSoundboardClips`.
- **Store signals**: `soundboardFolder`, `soundboardTiles`, `soundboardLoading`, `soundboardError`, `playingClips` (store), `tileHotkeys` (store), `soundboardSearch`, `clockTick`.
- **Store actions**: `pickSoundboardFolder`, `scanCurrentSoundboardFolder`, `playClip`, `stopClip`, `panicSoundboard`, `bindTileHotkey`, `clearTileHotkey`, `markClipFinished`. A `createEffect` runs an rAF loop while any clip is playing — increments `clockTick` for live progress rings and auto-removes clips that have run past their duration.
- **`SoundboardScreen.tsx`** full implementation:
  - Header: eyebrow + folder path + Change folder ghost button on the left; Search input (260 px) + Stop all (danger, solid when active, dimmed when nothing plays) on the right.
  - Empty states for "no folder picked", "loading", "no tiles match", "no audio files in folder".
  - Grid of tiles (auto-fill 180 px+, 120 px tall, rounded, surface-2 background). Each tile: emoji top-left (derived from label initial), hotkey `Kbd` chip top-right when bound, label + colour dot + file size at the bottom. Playing tiles get a coloured 1.5 px border + glow, a larger emoji, an SVG progress ring (`stroke-dashoffset` animated via `clockTick`), and a `—Xs` countdown.
  - In-app keydown listener routes hotkeys to `playClip(tile)`. Global registration (works while window unfocused) is Phase 6.

### Changed

- `src/types.ts` removes the Phase 1 sample-data `SoundboardTile` interface and re-exports the canonical wire type from `src/audio/api.ts`. Adds a new `PlayingClip` interface (clip id + `startedAt` + duration).
- `src/data/soundboard.ts` (mock data) removed — the soundboard list now comes entirely from the backend.

### Tests

- **Rust**: 54 → 70. New: 6 scanner (missing folder errors, empty folder, picks supported extensions, sorts by label, id is stable across scans, label strips extension, ignores nested subdirs) + 10 mixer (empty mixer is a no-op, play adds into output, voice deactivates at buffer end, stop deactivates matching voice, stop_all clears voices, sample-rate mismatch interpolated, polyphony sums voices, max-voices steals oldest, snapshot reports duration + progress).
- **Frontend**: 73 → 86. New: 5 API wrappers (`scanSoundboardFolder`, `playSoundboardClip`, `stopSoundboardClip`, `stopAllSoundboardClips`, error-propagation) + 9 store actions (scan no-folder no-op, scan success, scan error, playClip records duration, playClip records error, stopClip swallows backend error, panicSoundboard clears all, markClipFinished removes a single clip, bind / clear hotkey).

### Architecture notes

- The decoded-clip cache lives on the Tauri shell, not in `divora-core`. The engine only ever sees `Arc<Vec<f32>>` payloads — no path lookups happen in the audio callback.
- The voice pool's "newest play wins via oldest-steal" matches every commercial soundboard's behaviour and avoids the alternative (drop-on-full) that would silently miss clicks during a busy moment.
- Progress tracking is purely UI-side. The audio callback emits no per-voice events; the frontend's `clockTick` rAF loop derives positions from `(performance.now() - startedAt) / durationSecs`. This keeps the audio thread completely allocation-free.
- The current implementation only enumerates the chosen folder's direct children (no recursion). Recursive scan is an easy follow-up if users want subfolders to count.

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (70 in divora-core)
- `pnpm typecheck` — pass
- `pnpm test` — pass (86)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The mic + soundboard collide into the same output stream the virtual mic eventually drains. That means when Phase 6 wires VB-Cable routing, sound effects appear on the *other* end of Discord / Zoom / OBS calls without any extra plumbing. The audio architecture for streamer-style "play this clip into the call" is now done; what remains is the cable.

## [0.4.0] — 2026-05-28 — Phase 4: presets + A/B compare

### Added

- **`divora-core::presets` module** — `Preset` / `PresetChainEntry` / `PresetTag` types + `PresetStore` (file-backed user preset I/O) + `bundled_presets()` (compile-time-embedded defaults).
- **5 bundled preset JSON files** in `divora-core/src/presets/bundled/`: `hollow-king.json`, `static-wraith.json`, `velvet-demon.json`, `choir-of-ash.json`, `clean.json`. They mirror the previous frontend mock data exactly; editing a file and rebuilding ships new defaults.
- **User preset persistence**: `PresetStore` writes/reads JSON under `%APPDATA%\DivoraVoice\presets\` (or platform equivalent). Each user preset is `<id>.json`. The id is sanitised (`[a-z0-9_-]+`) so it can't escape the directory. Corrupt files are logged and skipped — a single bad preset can't take down the whole list.
- **Tauri commands**: `list_presets`, `save_user_preset`, `delete_user_preset`, `export_preset_json`, `preset_store_path`. Bundled presets are read-only (saving one returns `BundledIsReadOnly`).
- **Frontend audio API extensions** (`src/audio/api.ts`): `WirePreset` / `WireChainEntry` types + `listPresets`, `saveUserPreset`, `deleteUserPreset`, `exportPresetJson`, `presetStorePath` wrappers.
- **Reactive presets in the store**: `presets()` signal seeded with the frontend's `FALLBACK_PRESETS` and replaced after `refreshPresets()` pulls the live list. `presetsLoaded()` flips true on the first successful load.
- **A/B compare snapshots** (`abSlots: Record<string, { A, B }>`): per-preset two-slot store. `setAbSlot(slot)` snapshots the live chain into the current slot, restores the destination slot, syncs to the audio thread, and updates `ui.ab`. `resetAbSlots()` collapses both slots to the current chain.
- **Preset actions in the store**: `usePreset(id)` switches the active preset (and resets A/B); `savePreset(preset)`, `duplicatePreset(sourceId)`, `deletePreset(id)`, `exportPreset(preset)`. Bundled presets refuse `savePreset` / `deletePreset` and ask the user to duplicate instead.
- **Chain reorder**: `reorderChainEntries(from, to)` moves an effect within the active chain and re-syncs to the audio thread.
- **`PresetsScreen.tsx`** full implementation:
  - Left panel (248 px): bundled list + user list, each row clickable → `usePreset(id)`. Active row gets the accent-bg + indigo border + emerald "in use" dot.
  - Right editor: header (color glyph chip + display-font name + Bundled/User badge + "in use" badge + description + Use button), action row (Duplicate / Export JSON / Save / Delete), and a vertical list of `ChainCard`s.
  - Each `ChainCard` has a drag handle (HTML5 drag/drop), effect sigil chip, name + live readout, enable toggle. Enabled cards expand to a 1- or 2-column grid of parameter sliders with bipolar variant for signed params.
  - Drop-target highlighting (indigo border + accent glow) when dragging.
  - Save / Delete are disabled for bundled presets; action errors land in an inline banner.
- **`ExportPresetModal.tsx`** — full-screen overlay with the preset serialised as JSON, Copy + Save .json buttons. Copy uses `navigator.clipboard`; Save .json uses a `Blob` + anchor download. Both fail gracefully when the API is unavailable.
- **Mixer A/B segmented control** is now wired to `setAbSlot` so toggling actually swaps snapshots and sends a fresh `SetChain` to the audio thread.
- **`src/App.tsx` `onMount`** now calls `refreshPresets()` alongside the device/level bootstrap.

### Changed

- `src/data/presets.ts` is now `FALLBACK_PRESETS` (5 bundled). The two original "User" defaults (Deep Warden, Glass Oracle) are no longer shipped — user presets are now created on demand by the user.
- Original `PRESETS` export remains as a deprecated alias for back-compat.

### Tests

- **Rust**: 39 → 54 tests. New: 3 bundled-preset integrity (parse, unique ids, Hollow King shape) + 11 `PresetStore` (empty dir, save / list round-trip, overwrite by id, rejects bundled tag, rejects unsafe ids, delete, NotFound, skip non-JSON, skip corrupt JSON, rewrites bundled-on-disk to user) + 1 helper (is_safe_id).
- **Frontend**: 55 → 73 tests. New: 5 audio API wrappers (`listPresets`, `saveUserPreset`, `deleteUserPreset`, `exportPresetJson`, `presetStorePath`) + 13 store action tests (refresh / fallback / use / A-B swap / save / duplicate / delete / export / reorder / clamping / resetAbSlots).

### Architecture notes

- `Preset.tag` is the single source of truth for "can I save this?". The wire format flows backend ↔ frontend without translation; serde uses the same `PascalCase` `Bundled` / `User` discriminator both sides.
- A/B compare lives entirely in the frontend — backend isn't asked to remember anything between toggles. The `SetChain` it receives is whatever's currently active. This keeps the audio thread simple and lets us extend A/B (more slots, named snapshots) without touching Rust.
- Drag-reorder is HTML5 native (`text/plain` index, no library). Dropping outside the cards just leaves the chain unchanged.

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (54 in divora-core)
- `pnpm typecheck` — pass
- `pnpm test` — pass (73)
- `pnpm tauri build --debug --no-bundle` — pass (11 MB, JS 101 KB / gzip 32 KB)

### Why it matters

The chain is now a thing you can *shape and keep*. Bundled presets give every new user a starting point; saving / duplicating turns the bundled set into a personal library; A/B compare lets users diff two takes without committing. The wire format is stable enough that future phases (preset sharing, community registry, preset export over the network) plug into the same `WirePreset` shape without revisiting persistence.

## [0.3.1] — 2026-05-28 — fix: pitch shifter passthrough

### Fixed

- **Pitch effect no longer produces audible doubling at non-unity ratios.** The Phase 3 dual-read varispeed shifter kept a 500 ms circular buffer with two read pointers separated by `HALF` (≈ 500 ms). At any non-unity ratio (so anything other than `shift = 0`), the two pointers drifted through the crossfade together, sampling buffer contents from two distinct times ~500 ms apart. With both weights at ~0.5 during the transition, listeners heard their own voice with a half-second echo of itself — the "I hear myself twice" the user reported on the Hollow King preset (which enables pitch with `shift = -5`).
- The Phase 3 CHANGELOG already documented pitch as a stub awaiting a real phase-vocoder. The previous implementation was an attempted dual-read varispeed that didn't work for the reasons above. v0.3.1 ships an honest passthrough until the real algorithm lands: the slider still moves and feeds the audio thread (chain plumbing exercised end-to-end), but no DSP is applied.

### Tests

- `pitch::tests::passthrough_at_zero_shift` — identity check at `shift = 0`.
- `pitch::tests::passthrough_at_nonzero_shift_too` — same identity check at `shift = -5` (the Hollow King default that exposed the bug).
- `pitch::tests::passthrough_across_a_sweep_of_semitones` — full ±12 st design range plus out-of-range values, every setting passes through bit-exact.
- `pitch::tests::set_param_reaches_internal_state_and_clamps_to_range` — guards that the slider still drives the parameter (and clamps out-of-range values) so the chain plumbing is exercised end-to-end.
- `pitch::tests::dc_signal_unchanged` — the most damning version of the original bug had DC input producing delayed-overlay artefacts; we now assert bit-exact passthrough of constant input.

## [0.3.0] — 2026-05-28 — Phase 3: DSP effect chain + spell circle

### Added

- **`divora-core::dsp` module** — eight effects (`NoiseGate`, `Eq`, `Distortion`, `Echo`, `Reverb`, `Robot`, `PitchShift`, `FormantShift`) implementing the new `AudioEffect` trait plus an `EffectChain` that runs them in order over a mono buffer.
- **`DspCommand` enum** — `SetChain { specs }`, `SetParam { index, key, value }`, `SetEnabled { index, enabled }`, `Clear`. Routed UI → engine thread → audio output callback via a fresh SPSC channel per stream start.
- **AudioEngine.send_dsp(cmd)** — non-blocking handle through which the Tauri shell pushes chain edits.
- **Tauri commands**: `set_effect_chain`, `set_effect_param`, `set_effect_enabled`, `clear_effect_chain`.
- **`src/audio/api.ts`** — typed wrappers + `EffectSpec` / `EffectKindWire` types.
- **Store extensions** in `src/stores/app.tsx`: `setChainParam`, `setChainEnabled`, `toggleEffectById`, `syncChain` actions; `selectedEffect` / `setSelectedEffect` signals; `createEffect`s that auto-send `SetChain` when the active preset or `engineRunning` flips, and per-param updates via the helper actions.
- **`SpellCircle` component** (`src/components/SpellCircle.tsx`) — full SVG visualization ported from the prototype: ambient core glow, pulse rings, outer decorative ring with runic tick marks (`mystical >= 0.5`), constellation dots (`mystical >= 0.9`), rotating mid-orbit dashes, orbiting spark, gradient threads from core to enabled effects (animated `dash-flow` when modulated), breathing voice core with state-coloured ring + status sigil, drifting particles (when modulated). Effect nodes are clickable (select) and double-clickable (toggle enabled).
- **`Inspector` component** — selected-rune card with sigil + name + enable toggle + description + one slider per parameter (bipolar variant for signed params). Sliders write straight through to `setChainParam` so live drag updates the audio thread on the next buffer.
- **Mixer screen upgraded** — replaces the Phase 2 spell-circle placeholder with the real `SpellCircle`; Inspector slots into the right rail.

### Quality scope for Phase 3

- Gate, EQ, distortion, echo, reverb, robot are real algorithms.
- **Pitch** ships a dual-read varispeed shifter — moves the slider, tracks the chain, audible artefacts on harmonic content. Real phase-vocoder lands later.
- **Formant** ships a parallel band-pass colouring (three filters at typical vowel formants, scaled by `2^(shift/12)`). Real LPC-based formant warp lands later.

### Tests

- **Rust**: 36 tests (was 14). New: 5 chain tests, 3 gate, 3 distortion, 2 echo, 2 reverb, 2 robot, 2 EQ, 2 pitch, 1 formant.
- **Frontend**: 55 tests (was 38). New: 5 SpellCircle math, 4 DSP API wrappers, 8 chain editing actions.

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (36)
- `pnpm typecheck` — pass
- `pnpm test` — pass (55)
- `pnpm tauri build --debug --no-bundle` — pass

### Architecture notes

- DSP chain is **owned by the audio output callback** — no locks, no contention. Param updates land on the next buffer (~5 ms at 256 frames / 48 kHz).
- `SetChain` allocates (`Box::new` per effect) in the audio callback. Acceptable for Phase 3; Phase 8 polish can move chain building off the audio thread.
- `EffectChain::apply` is the single point of mutation called from the audio callback.

### Why it matters

The chain plumbing is now real. Phase 4 lands the preset editor and A/B compare on top of the same `SetChain` / `SetParam` / `SetEnabled` surface. The spell circle is the visual that pays off the dusk-violet design direction — every later phase plugs into it (preset switching, glyph casting, etc.).

## [0.2.0] — 2026-05-28 — Phase 2: Audio passthrough + real meters

### Added

- **`divora-core::audio` module** — new audio engine subsystem with submodules:
  - `state.rs`: `EngineState` (atomic running / monitor / level bits) + `Levels` struct (RMS + peak).
  - `level.rs`: `LevelMeter` with smoothed RMS and peak-with-decay; alloc-free, designed for the audio callback.
  - `devices.rs`: `DeviceInfo` + `list_input_devices` / `list_output_devices` via cpal.
  - `engine.rs`: `AudioEngine` owning a dedicated `divora-audio` thread that hosts cpal streams (`!Send`), receives commands via `mpsc`, and shares level state via `Arc<EngineState>`.
- **Passthrough audio path**: mic → mono mixdown → SPSC ring buffer (`ringbuf::HeapRb<f32>`, ~170 ms at 48 kHz) → output device, with optional `monitor` toggle silencing the output without disturbing the input metering. Stack-allocated 4096-frame scratch buffer; zero allocation in the callback.
- **Tauri commands**: `list_audio_input_devices`, `list_audio_output_devices`, `start_audio_engine`, `stop_audio_engine`, `set_audio_monitor`, `audio_engine_status`.
- **Tauri event**: `audio-levels` emitted at ~30 Hz from a `divora-level-emitter` thread spawned during the Tauri `setup` hook.
- **`src/audio/api.ts`**: typed TypeScript wrappers for every command + a `subscribeLevels` helper.
- **Audio signals + actions in `src/stores/app.tsx`**: `audioInputs`, `audioOutputs`, `selectedInput`, `selectedOutput`, `engineRunning`, `engineMonitoring`, `engineError`, `streamInfo`, `inputLevels`, `outputLevels`. Actions: `refreshDevices`, `startEngine`, `stopEngine`, `toggleMonitor`, `setMonitor`.
- **Settings → Audio devices section**: input device picker with live confirmation `HMeter` + dB readout, output device picker, engine Start / Stop button with running-state status text, sidetone Monitor toggle, clear error banner on failure.
- **Mixer screen upgraded** from Phase 1 placeholder to real layout: preset header (glyph chip + display-font name + Bundled / User badge + active rune count + routed-via line), vertical IN / OUT `VMeter`s flanking a `breathe`-animated spell-circle placeholder, right rail with Voice status, Push-to-modulate (segmented mode toggle + Hold to test button), Monitor cards, plus contextual Engine error / Engine offline cards.
- **App bootstrap (`src/App.tsx` `onMount`)**: refresh devices → subscribe to `audio-levels` → auto-start engine when both devices are selected. Failures caught and surfaced via `engineError`.

### Tests

- **Rust**: 14 tests across `divora-core` (LevelMeter behaviour, EngineState round-trips, device enumeration smoke, AudioEngine spawn / drop / level defaults).
- **Frontend**: 38 tests total (up from 24). New: 8 audio API tests, 6 audio store action tests.

### Architecture notes

- Phase 2 supports F32-only sample format and requires matching input / output sample rates. Non-F32 formats and resampling come in later phases (`AudioEngineError::UnsupportedSampleFormat` and `::SampleRateMismatch` surface these for the UI).
- Polyphony, DSP graph, virtual mic routing all slot in between input and output in later phases without touching this architecture.
- See `docs/ARCHITECTURE.md` for the full breakdown.

### Bundle

- CSS: 26 KB (up from 25 KB; new audio-section styles).
- JS index: 71 KB (gzip 23 KB) (up from 45 KB; audio API + store).
- Debug exe: 11 MB (cpal / ringbuf / serde land in-place; the workspace's release-profile LTO keeps the exe size flat in debug too).

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (14 + 1 = 15)
- `pnpm typecheck` — pass
- `pnpm test` — pass (38)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The audio engine is now a real thing. Sliders in later phases will hook into the same `Arc<EngineState>` for per-effect parameter sweeps; the spell circle will read from the same level signals that the IN / OUT meters use today. The whole later DSP graph plugs in between input and output without changing this scaffolding.

## [0.1.0] — 2026-05-28 — Phase 1: Design system + app shell

### Added

- **Sigil component library** (~40 custom SVG icons) and **DMark** brand icon (white "D" on indigo→pink gradient), ported as faithful SolidJS components from the prototype.
- **Base component library**: Button (4 variants × 3 sizes), IconButton, Toggle (default / danger / success tones), Slider (with bipolar variant), Badge (5 semantic tones), Kbd, Segmented, Select with dropdown, Card, Panel, EmptyState, HotkeyCapture, Tooltip, VMeter, HMeter. All styled via `@layer components` in `src/styles.css`.
- **Custom frameless titlebar** (`src/shell/Titlebar.tsx`) with DMark + DivoraVoice wordmark + live status pill (Clean / Modulated / Muted with shimmer when modulated) + persistent "🔒 LOCAL · NO ACCOUNT" affirmation + min/max/close buttons. OS drag region via `data-tauri-drag-region`.
- **80 px sidebar nav rail** (`src/shell/Sidebar.tsx`) with 4 nav items (Mixer / Board / Presets / Settings), active-state 3 px gradient bar + glow, footer bolt button (replay first-run) + green shield badge.
- **Four screens** (Mixer / Soundboard / Presets / Settings) with EmptyState placeholders calling out the phase that lands real content. Settings exposes the Color mood / Accent / Motion Tweaks live so the data-attribute system is observable today.
- **SolidJS state architecture**: `AppProvider` context with `nav`, `presetId`, `preset` (memoized), `chains` (per-preset editable copies), `ui` store, `wizardOpen`, `tweaks`, `glyphs`. Computed memos `hasEnabled`, `effectiveModulated`, `status`. Push-to-modulate Space hotkey wired (in-app; global hotkey lands in Phase 3).
- **Tweaks system foundation**: `data-mood`, `data-accent`, `data-motion`, `data-motion-user` root attributes drive token-cascade swaps via CSS custom properties. Three moods (Dusk Violet / Ink + Candle / Midnight), three accents (Brand / Abyssal / Ember), three motion levels (functional / ambient / rich).
- **Reduced-motion handling**: `@media (prefers-reduced-motion: reduce)` zeros `--motion` until the user explicitly picks via Tweaks.
- **Mock data layer**: `EFFECTS` catalog (8 effects with design-spec parameter ranges + readout functions), `EFFECT_ORDER`, `PRESETS` (5 bundled + 2 user defaults), `DEVICES_IN/OUT`, `SOUNDBOARD` tiles.
- **Self-hosted fonts**: `@fontsource-variable/bricolage-grotesque`, `@fontsource-variable/space-grotesk`, `@fontsource/space-mono` (400/700). Bundled total ~100 KB woff2.
- **Full design token system** in `src/styles.css`: surfaces (6), lines (3), text (4), accent (brand gradient), semantic (success/warning/danger/info), radii (5), spacing scale (9 steps, 4 px base), shadows (3 + glow), type scale (8 steps).
- **Tailwind theme extension** (`tailwind.config.js`) exposing every token as a Tailwind utility (e.g., `bg-surface-1`, `text-text-mid`, `font-mono`).

### Changed

- **Tauri window is now frameless** (`decorations: false`); custom titlebar owns the drag region and window controls.
- **Tauri capabilities** expanded with `core:window:allow-minimize`, `core:window:allow-maximize`, `core:window:allow-unmaximize`, `core:window:allow-toggle-maximize`, `core:window:allow-is-maximized`, `core:window:allow-close`, `core:window:allow-start-dragging`.
- `src/App.tsx` rewritten from the Phase 0 placeholder to compose `AppProvider` + Titlebar + Sidebar + the active screen, with `createEffect`s that mirror the Tweaks store into root-level data attributes.
- `src/main.tsx` now imports the font CSS up front so the font swap is invisible.

### Added — design integration (landed in commit 0a674dd, also part of 0.1.0)

- `docs/mockups/` — high-fidelity design reference (README spec, HTML + React interactive prototype, screenshots) defining the "spellcraft for your voice" visual identity, spell-circle Mixer, glyph-casting easter egg, and Tweaks system.
- Detailed design system section in `docs/PLAN.md` (color tokens, type scale, components, window chrome, motion).
- Glyph casting (Phase 7), A/B preset compare (Phase 4), and user-facing Tweaks (Phase 6) added to v1 scope.

### Changed — design integration

- **Product name finalized as "DivoraVoice"** (brand parent: Divora). Updated `tauri.conf.json` productName + window title, `README.md`, `index.html` `<title>`, placeholder App.tsx + test, and Rust doc comments. Internal crate names (`divora-app`, `divora-core`) kept as is.
- **Phase plan restructured** — design system + app shell is now Phase 1; original Phase 1 (audio passthrough) becomes Phase 2; all downstream phases shifted by +1.
- Bundled preset names changed from generic archetypes to the design-spec names: **Hollow King, Static Wraith, Velvet Demon, Choir of Ash, Clean Passthrough** (+ user defaults Deep Warden, Glass Oracle).
- DSP effect catalog and parameter ranges aligned with the design spec.

### Architecture

- New folders: `src/components/`, `src/data/`, `src/shell/`, `src/screens/`, `src/stores/`.
- One CSS file (`src/styles.css`) owns tokens, base reset, utilities, and component classes via `@tailwind` directives + `@layer` blocks.
- Solid context provider pattern (`AppProvider` + `useApp`) gives every component access to shared state without prop-drilling. Tests use `renderHook(..., { wrapper: AppProvider })`.
- The Tweaks → CSS-variable cascade means mood/accent swaps re-style every Tailwind utility that references a token without rerendering Solid components.

### UX details

- The titlebar's status dot shimmers when modulated (via the `shimmer` keyframe).
- Holding `Space` while focused on the document flips the voice state. Apply vs bypass mode lives in `ui.ptmMode`.
- Settings → Appearance is the only screen with live content in Phase 1; the others are EmptyStates calling out their target phase, but the shell around them matches the design.

### Tests

- 24 frontend tests across 5 files (statusMeta semantics, effects catalog invariants, app store derivations, Sigil rendering, App shell wordmark + status + LOCAL pill).
- 1 Rust test (`divora-core` placeholder), unchanged.

### Bundle

- CSS: 24.94 KB (gzip 5.99 KB)
- JS index: 45.05 KB (gzip 15.45 KB)
- JS window module: 15.94 KB (gzip 4.08 KB)
- Fonts (woff2): Bricolage Grotesque 41 KB, Space Grotesk 22 KB, Space Mono 33 KB
- Debug exe: 11 MB (unchanged from Phase 0)

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (1)
- `pnpm typecheck` — pass
- `pnpm test` — pass (24)
- `pnpm tauri build --debug --no-bundle` — pass

### Why it matters

The design system is now a known-good foundation. Every feature phase from here plugs into the shell without re-littering it with one-offs. The Tweaks system means we can swap color moods and motion levels live, including respecting the user's OS-level reduced-motion preference. The shell matches the screenshots; only the content within each screen needs to land per phase.

## [0.0.0] — 2026-05-28

### Added

- Initial project scaffold for Divora.
- Tauri 2.x + SolidJS + TypeScript + Tailwind frontend.
- Rust workspace with `divora-core` (audio engine + DSP) and `divora-app` (Tauri shell).
- GitHub Actions CI: `cargo fmt`, `cargo clippy`, `cargo test`, `pnpm typecheck`, `pnpm test`, `cargo tauri build --debug`.
- GitHub Actions release workflow: tag-triggered, builds MSI/NSIS installers, uploads as Release artifacts.
- `docs/PLAN.md` — detailed phase-by-phase implementation plan.
- `docs/ARCHITECTURE.md` — as-built architecture notes (will grow with each phase).
- `docs/CONTRIBUTING.md` — contribution guidelines.
- `docs/MANUAL_TESTS.md` — pre-release manual test checklist.
- MIT license.
- README with project description, requirements, build steps.

### Why it matters

The scaffold gives every future phase a known-good starting point. CI is green from day one so we never have to debug "is it my code or my build" — only "is it my code." The phase-based SDLC workflow ships from this commit forward without changes.

### Architecture

- `src-tauri/` — Rust backend (Tauri shell + audio engine workspace).
- `src/` — SolidJS frontend.
- `presets/` — bundled JSON preset definitions (empty in Phase 0).
- `docs/` — living documentation.
- `.github/workflows/` — CI and release workflows.

### UX details

- Blank Divora window opens via `pnpm tauri dev`.
- App icon, title bar, basic theme (dark, system font) in place.
- No audio functionality yet.

### Tests

- One smoke test in `divora-core` (`add(2, 2) == 4` placeholder) to wire up `cargo test`.
- One smoke test in frontend (`App.test.tsx` renders without throwing) to wire up Vitest.
- E2E placeholder commented out — will be enabled in Phase 1 once there's UI to test.

### Bundle

- Debug exe: ~11 MB (`pnpm tauri build --debug --no-bundle`).
- CI builds debug-exe-only for speed; MSI/NSIS only produced by the tag-triggered release workflow.
- Release MSI/NSIS: to be measured on first tagged release.

### Pre-push checklist (local, 2026-05-28)

- `cargo fmt --check` — pass
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — pass
- `cargo test --workspace --all-features` — pass (1 rust test)
- `pnpm typecheck` — pass
- `pnpm test` — pass (2 frontend tests)
- `pnpm tauri build --debug --no-bundle` — pass
