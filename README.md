# DivoraVoice

A free, open-source real-time voice modulator for Windows. Local-first, no account, no telemetry.

The visual identity is **"spellcraft for your voice"** — a calm utility tool wearing a dusk-lit arcane skin. See [`docs/mockups/`](docs/mockups/) for the design.

> **Status:** Phase 0 — scaffold. Not yet usable. See [docs/PLAN.md](docs/PLAN.md) for the roadmap.

## What it does (target v1)

- Real-time microphone effects: pitch, formant, reverb, EQ, robot, distortion, echo, noise gate
- 15+ bundled "persona" presets (Deep Narrator, Helium, Robot, Phone Call, Stadium PA, etc.)
- Folder-based soundboard with optional per-tile hotkeys
- Self-monitoring sidetone so you can hear yourself
- Output routed to a virtual mic so Discord, Zoom, OBS, games see your modulated voice
- Push-to-modulate global hotkey (hold key → effects on, release → bypass)
- Sub-30 ms end-to-end latency on consumer hardware

## What it doesn't do (and won't)

- Named celebrity voice clones. Persona archetypes only. You can bring your own ONNX model later.
- Cloud, accounts, telemetry, upsells.
- Cross-platform (yet). Windows only for v1.

## Requirements

- Windows 10/11
- A virtual audio cable, e.g. [VB-Cable](https://vb-audio.com/Cable/) (free, donationware). The app will prompt you on first run if it's missing.

## Building from source

Prerequisites: Rust stable, Node 20+, pnpm 9+.

```powershell
pnpm install
pnpm tauri dev      # development
pnpm tauri build    # release MSI/NSIS
```

## Contributing

See [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

## License

MIT — see [LICENSE](LICENSE).

## SmartScreen note

Until we have an EV code-signing certificate (planned for v1.0+), Windows SmartScreen will warn before running unsigned installers. Click "More info" → "Run anyway". SHA256 checksums are published on every Release page so you can verify the download.
