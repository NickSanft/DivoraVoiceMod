# DivoraVoice

A free, open-source, real-time voice modulator for Windows. Local-first, no account, no telemetry.

The visual identity is **"spellcraft for your voice"** — a calm utility tool wearing a dusk-lit arcane skin. See [`docs/mockups/`](docs/mockups/) for the design.

> **Status: v1.7.0** — stable. The v1.0 command + preset contract is frozen and additive-only across the 1.x line (see [`docs/STABLE-SURFACE.md`](docs/STABLE-SURFACE.md)). Since 1.0: **The Coven** voice cast (16 bundled presets), AI voice conversion, monitor-output routing, recording, system tray, a live latency readout, and loudness normalization. Full history in [`CHANGELOG.md`](CHANGELOG.md).

## Install

Each tagged release publishes both an **MSI** and an **NSIS** installer. Grab the latest from the [Releases page](https://github.com/NickSanft/DivoraVoiceMod/releases).

The installers are not yet code-signed, so Windows SmartScreen will warn before running one. Click **More info → Run anyway**. Every release's `.msi` and `-setup.exe` have SHA-256 checksums in the Release notes if you want to verify the download.

## What it does

- **Real-time microphone effects** — a live, reorderable chain of 12 effects: noise gate, **RNNoise denoiser**, pitch (phase vocoder), formant (spectrum warp), EQ, robot, distortion, echo, reverb, **chorus**, **harmonizer**, and **AI voice convert**. Each has per-effect parameters and a live **"+N ms latency"** readout that updates as you toggle effects.
- **The Coven — 16 bundled persona presets.** A browsable cast of character voices: Hollow King, Velvet Demon, Static Wraith, The Oracle, Choir of Ash, Seraph, Dirge, The Swarm, The Possessed, Leviathan, The Imp, Dispatch, Corrupted, Whisper Wraith, Deep Narrator (AI), and Clean Passthrough. Plus unlimited user presets with a full JSON editor, export/import, and **A/B compare** on the Mixer.
- **AI voice conversion** — an ONNX-Runtime `VoiceConvert` effect with a bundled streaming LLVC narrator (~13 ms) and **bring-your-own `.onnx` model** support. Degrades to passthrough (never hangs) when no model or runtime is present. See [`docs/voice-models/`](docs/voice-models/).
- **Loudness normalization** — an optional output stage (auto-gain + brick-wall limiter, zero added latency) that keeps your *perceived* level steady across presets and never clips.
- **Monitor output routing** — an independent second output, so the main send can go to VB-Cable (→ Discord / games) while you still hear yourself on headphones, with a separate monitor volume.
- **VB-Cable bridge** — DivoraVoice pours the modulated voice into `CABLE Input`; Discord / Zoom / OBS pick it up from `CABLE Output`. The first-run wizard walks you through it.
- **Folder-backed soundboard** with drag-reorder, per-tile colors, per-tile + master volume, recent folders, and **global per-tile hotkeys** that fire even when the app isn't focused.
- **Recording** — one-click capture of the modulated output (voice + soundboard) to a timestamped WAV.
- **Push-to-modulate global hotkey** (hold key → effects on, release → bypass). Configurable in Settings → Hotkeys.
- **Glyph casting** — draw a triangle / inverted triangle / square / circle on the Mixer to instantly switch to the bound preset.
- **System tray** — minimize to tray so audio keeps running in the background during calls/games.
- **First-run wizard** with VB-Cable detection + device picker + Discord routing instructions.
- **Sub-30 ms end-to-end latency** on consumer hardware (sub-26 ms even with the phase-vocoder pitch shifter active).
- **Automatic sample-rate matching** via rubato — mismatched mic/output rates no longer hard-fail.

## What it doesn't do (and won't)

- Named celebrity voice clones. Persona archetypes only. (The AI voice convert is bring-your-own-model.)
- Cloud, accounts, telemetry, upsells.
- Cross-platform (yet). Windows only for v1.

## Roadmap & known limitations

DivoraVoice is feature-complete for v1 and shipping steady post-1.0 improvements (light theme, control surfaces, an in-app update check, a "Test my setup" diagnostic, preset import, …). The detailed per-release plan lives in [docs/PLAN.md](docs/PLAN.md).

**Known limitations (today):**

- **Windows only.** macOS / Linux aren't in scope for v1 (the audio path is cpal/WASAPI; a cross-platform pass is "further out").
- **VB-Cable required** to send your voice to other apps — the app detects and prompts, but doesn't bundle it (licensing).
- **AI voice conversion** falls back to passthrough when no ONNX runtime/model is present — it never blocks the app, but the effect is simply unavailable until a model is installed.
- **Text-to-speech ("Speak")** runs fully on-device with bundled preset voices (Kokoro-82M + espeak-ng). The voice assets ship in the installer, so synthesis is available in the **built app**; running from source shows "voice not installed" until the assets are fetched into the resource dir (`scripts/fetch-voice-assets.ps1`), same as the AI voice-conversion models. **Custom voices ("Your voices"):** add your own voice by recording it in-app or importing a 20–30 s clip — the clone auto-matches the closest preset base (timbre transfer; accent comes from the base). The cloning models download on demand the first time.
- **Recording** is WAV only.
- **Soundboard** plays one folder at a time and doesn't recurse into subfolders.
- **Installers are unsigned** — Windows SmartScreen shows a warning (see [Install](#install) for the one-time "More info → Run anyway"). The in-app update check is a one-way version read (no telemetry); auto-install is intentionally not included.

**Further out (standard semver):** dynamics polish, VST3 host, OBS WebSocket, a community preset registry, and eventually cross-platform. See [docs/PLAN.md](docs/PLAN.md) for specifics.

## Requirements

- Windows 10 / 11
- A virtual audio cable, e.g. [VB-Cable](https://vb-audio.com/Cable/) (free, donationware). The app prompts you on first run if it's missing.

## Building from source

Prerequisites: Rust stable, Node 22+, pnpm 10+, **cmake** (vendored libopus + libnnnoiseless need it).

```powershell
pnpm install
pnpm tauri dev      # development run
pnpm tauri build    # release MSI / NSIS
```

CI runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, `pnpm typecheck`, `pnpm test`, and `pnpm tauri build --debug --no-bundle` on every push to `main`. See [docs/PLAN.md](docs/PLAN.md) for the per-phase ship workflow.

## Architecture

- **Frontend**: SolidJS + TypeScript + Tailwind (Vite-built).
- **Shell**: Tauri 2 (frameless window, custom titlebar).
- **Backend**: Rust — `cpal` for WASAPI I/O, `realfft` + a hand-rolled phase vocoder for pitch / formant / harmonizer, `rubato` for sample-rate matching, `nnnoiseless` for RNNoise suppression, `biquad` for EQ, `symphonia` (+ `symphonia-adapter-libopus`) for soundboard decode, and `ort` (ONNX Runtime) for the AI voice-convert effect. The audio engine, DSP, presets, and soundboard live in the standalone `divora-core` crate (unit-testable without a UI); the Tauri shell (`src-tauri`) owns the IPC surface and window.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the per-phase deep dive.

## Documentation

| Doc | What it covers |
|---|---|
| [`CHANGELOG.md`](CHANGELOG.md) | Every release, newest first. |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Per-phase deep dive into the engine, DSP, and frontend. |
| [`docs/STABLE-SURFACE.md`](docs/STABLE-SURFACE.md) | The v1.0 back-compat contract (commands, events, wire shapes, preset schema). |
| [`docs/CONTRIBUTING.md`](docs/CONTRIBUTING.md) | Dev setup, house style, and the per-release workflow. |
| [`docs/voice-models/`](docs/voice-models/) | How to produce and install an ONNX voice-conversion model. |
| [`docs/MANUAL_TESTS.md`](docs/MANUAL_TESTS.md) | Pre-release manual test checklist. |
| [`docs/PLAN.md`](docs/PLAN.md) | The original phase-by-phase implementation plan. |

## Contributing

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

Bug reports and PRs welcome. Pre-release manual test pass: [`docs/MANUAL_TESTS.md`](docs/MANUAL_TESTS.md).

## License

MIT — see [LICENSE](LICENSE).

Third-party licenses worth calling out: `nnnoiseless` is BSD-3-Clause (Xiph RNNoise port); `symphonia-adapter-libopus` bundles libopus (BSD); `rubato`, `realfft`, and `rustfft` are MIT; ONNX Runtime (`ort`) is MIT; the bundled LLVC narrator voice is derived from KoeAI's MIT-licensed LLVC.

**Text-to-speech ("Speak"):** synthesis uses [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) (Apache-2.0); text→phoneme conversion uses [**espeak-ng**](https://github.com/espeak-ng/espeak-ng) (**GPL-3.0**). espeak-ng is **not linked into DivoraVoice** — its binary (`espeak-ng.exe` + `libespeak-ng.dll` + `espeak-ng-data`) is bundled as a **separate, arm's-length component invoked via subprocess** (FSF "mere aggregation"), so DivoraVoice's own code stays MIT. The corresponding espeak-ng source is available under GPL-3.0 at <https://github.com/espeak-ng/espeak-ng> (the v1.52.0 release we bundle). **Voice cloning** ("Your voices", v1.20.0) uses [OpenVoice v2](https://github.com/myshell-ai/OpenVoice) (**MIT**) for tone-color conversion — its ONNX models are downloaded on demand (v1.21.0), not bundled, so they don't bloat the installer. No new copyleft.
