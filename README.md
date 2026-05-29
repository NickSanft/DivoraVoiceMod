# DivoraVoice

A free, open-source, real-time voice modulator for Windows. Local-first, no account, no telemetry.

The visual identity is **"spellcraft for your voice"** — a calm utility tool wearing a dusk-lit arcane skin. See [`docs/mockups/`](docs/mockups/) for the design.

> **Status: v0.10** — feature complete for v1. Pitch, formant, EQ, robot, distortion, echo, reverb, noise gate, **RNNoise denoiser**, soundboard, A/B compare, glyph casting, first-run wizard, global hotkeys, VB-Cable bridge, sample-rate matching, full Settings, all shipped.

## Install

Each tagged release publishes both an **MSI** and an **NSIS** installer. Grab the latest from the [Releases page](https://github.com/NickSanft/DivoraVoiceMod/releases).

Until we have an EV code-signing certificate (planned for the v1.0 cycle), Windows SmartScreen will warn before running an unsigned installer. Click **More info → Run anyway**. Every release's `.msi` and `-setup.exe` have SHA-256 checksums in the Release notes if you want to verify the download.

## What it does

- **Real-time microphone effects** — pitch (phase vocoder), formant (spectrum warp), EQ, robot, distortion, echo, reverb, noise gate, **RNNoise denoiser** (new in v0.10).
- **5 bundled persona presets** — Hollow King, Static Wraith, Velvet Demon, Choir of Ash, Clean Passthrough — with full JSON editor and export.
- **A/B preset compare** on the Mixer header.
- **Folder-backed soundboard** with drag-reorder, per-tile colors, recent folders, and **global per-tile hotkeys** that fire even when the app isn't focused.
- **Self-monitoring** (sidetone) so you can hear yourself while tuning.
- **VB-Cable bridge** — DivoraVoice pours the modulated voice into `CABLE Input`; Discord / Zoom / OBS pick it up from `CABLE Output`. The first-run wizard walks you through it.
- **Push-to-modulate global hotkey** (hold key → effects on, release → bypass). Configurable in Settings → Hotkeys.
- **Glyph casting** — draw a triangle / inverted triangle / square / circle on the Mixer to instantly switch to the bound preset.
- **First-run wizard** with VB-Cable detection + device picker + Discord routing instructions.
- **Sub-30 ms end-to-end latency** on consumer hardware (sub-26 ms even with the phase-vocoder pitch shifter active).
- **Automatic sample-rate matching** via rubato — mismatched mic/output rates no longer hard-fail.

## What it doesn't do (and won't)

- Named celebrity voice clones. Persona archetypes only. You can bring your own ONNX model later.
- Cloud, accounts, telemetry, upsells.
- Cross-platform (yet). Windows only for v1.

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
- **Backend**: Rust — `cpal` for WASAPI I/O, `realfft` + a hand-rolled phase vocoder for pitch / formant, `rubato` for sample-rate matching, `nnnoiseless` for RNNoise suppression, `biquad` for EQ, `symphonia` (+ `symphonia-adapter-libopus`) for soundboard decode.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the per-phase deep dive.

## Contributing

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

Bug reports and PRs welcome. Pre-release manual test pass: [`docs/MANUAL_TESTS.md`](docs/MANUAL_TESTS.md).

## License

MIT — see [LICENSE](LICENSE).

Third-party licenses worth calling out: `nnnoiseless` is BSD-3-Clause (Xiph RNNoise port); `symphonia-adapter-libopus` bundles libopus (BSD); `rubato`, `realfft`, and `rustfft` are MIT.
