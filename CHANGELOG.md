# Changelog

All notable changes to Divora are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning is **phase = minor** until v1.0 (see [docs/PLAN.md](docs/PLAN.md)).

## [Unreleased]

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
