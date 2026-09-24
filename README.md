# DivoraVoice

A free, open-source, real-time voice modulator **and on-device text-to-speech** for Windows. Local-first, no account, no telemetry.

The visual identity is **"spellcraft for your voice"** — a calm utility tool wearing a dusk-lit arcane skin. See [`docs/mockups/`](docs/mockups/) for the design.

> **Status: v1.48.0** — stable and shipping steady improvements. The v1.0 command + preset contract is frozen and additive-only across the 1.x line (see [`docs/STABLE-SURFACE.md`](docs/STABLE-SURFACE.md)). Since 1.0: on-device **text-to-speech ("Speak")**, **voice cloning** (accent-preserving *and* timbre-only), a **24-voice preset cast** (The Coven + Minecraft-mob and vintage-radio characters), a **20-effect** real-time chain, **MIDI / Stream Deck** control surfaces, a **stream overlay**, and **reactive effects** that follow your voice, **tap-to-audition** voice previews, and reliability hardening (audio device-loss recovery, soundboard-freeze fix). Full history in [`CHANGELOG.md`](CHANGELOG.md).

## Install

Each tagged release publishes both an **MSI** and an **NSIS** installer. Grab the latest from the [Releases page](https://github.com/NickSanft/DivoraVoiceMod/releases).

The installers are not yet code-signed, so Windows SmartScreen will warn before running one. Click **More info → Run anyway**.

Each release lists **SHA-256 checksums** for the `.msi` and `-setup.exe` in its notes and attaches a `SHA256SUMS.txt`, so you can verify a download (`Get-FileHash <file> -Algorithm SHA256` in PowerShell, or `sha256sum -c SHA256SUMS.txt`).

## What it does

- **Real-time microphone effects** — a live, reorderable chain of **20 effects**:
  - *Cleanup & dynamics* — noise gate (a soft downward expander), **RNNoise denoiser**, compressor, de-esser.
  - *Pitch & tone* — pitch (phase vocoder), formant (spectrum warp), 3-band EQ, radio band-pass.
  - *Character & space* — robot, distortion, **bitcrusher**, echo, reverb, chorus, harmonizer, tremolo, warble, breath (whisperizer), and vintage noise.
  - *AI* — **voice convert** (ONNX).

  Each effect has its own parameters and a live **"+N ms latency"** readout that updates as you toggle effects.
- **On-device text-to-speech — "Speak."** Type text, pick a preset US/UK voice, and synthesize speech fully **on-device** (Kokoro-82M + espeak-ng) that plays through the output and mixes into your call/stream like the live mic. Includes a **saved-clips library** (replay, reuse, or load a clip back into the editor), a volume/preview control, an independent monitor toggle, and per-take progress.
- **Critter Chatter.** Three playful babble voices for Speak — **Bright**, **Mellow**, and **Gruff** — that "talk" in one quick synthesized syllable per written character, like the chatter of small game characters. Fully procedural: no model, no download, renders near-instantly, and works on every install.
- **Voice cloning — "Your voices."** Add your own voice two ways: **record** a short on-screen sentence for an **accent-preserving** clone (VoxCPM reproduces your timbre *and* accent), or **import** a 20–30 s clip for a quick **timbre-only** clone (OpenVoice, recolors a preset toward you and auto-picks the closest base). Multi-take **best-of-N** reranking (Fast / Balanced / Best) picks the closest result, and you can **rename** cloned voices inline. Both engines' models download on demand (accent ~1.6 GB, timbre ~157 MB), so the installer stays small. Optional experimental **GPU (DirectML)** acceleration.
- **The Coven — 24 bundled persona presets.** A browsable cast of character voices:
  - *The Coven (14):* Hollow King, Static Wraith, Velvet Demon, Choir of Ash, The Oracle, Seraph, Dirge, The Swarm, The Possessed, Leviathan, The Imp, Dispatch, Corrupted, Whisper Wraith.
  - *Character voices (8):* Spirit Radio and Parlor Augur (vintage radio); Villager, Creeper, Zombie, Enderman, and Ghast (Minecraft-mob); Arcade Cabinet (retro game).
  - *AI (1):* Deep Narrator. *Plus* Clean Passthrough.

  On top of the bundled cast: unlimited user presets with a full JSON editor, export/import, and **A/B compare** on the Mixer.
- **AI voice conversion** — an ONNX-Runtime `VoiceConvert` effect with a bundled streaming LLVC narrator (~13 ms) and **bring-your-own `.onnx` model** support. Degrades to passthrough (never hangs) when no model or runtime is present. See [`docs/voice-models/`](docs/voice-models/).
- **Voice reading** — an optional Mixer panel (off by default) that measures the **acoustic properties of the signal** and shows them two ways: **your voice**, read from the dry microphone before any effect, and **after effects**, read from the chain's output — what the call actually hears. It shows five numbers per half — pitch, pitch range, level, pace and tone — and a short phrase describing the sound ("bright, wide range, fast"), which is also where level movement appears. It answers "am I going flat on this stream?" and "is my demon voice still landing?" without guessing at anything. During a pause it **holds** the last measured window and says so rather than quietly decaying; muted, engine-stopped and quiet-but-present are three separate states with three separate messages. The per-session baseline it compares against never leaves memory, and nothing appears on the stream overlay.
- **Loudness normalization** — an optional output stage (auto-gain + brick-wall limiter, zero added latency) that keeps your *perceived* level steady across presets and never clips.
- **Monitor output routing** — an independent second output, so the main send can go to VB-Cable (→ Discord / games) while you still hear yourself on headphones, with a separate monitor volume.
- **VB-Cable bridge** — DivoraVoice pours the modulated voice into `CABLE Input`; Discord / Zoom / OBS pick it up from `CABLE Output`. The first-run wizard walks you through it.
- **Folder-backed soundboard** with drag-reorder, per-tile colors, per-tile + master volume, recent folders, and **global per-tile hotkeys** that fire even when the app isn't focused.
- **Hardware control surfaces** — **MIDI input with MIDI-learn**, plus a zero-plugin **Stream Deck** path, to trigger presets and actions from physical controls.
- **Recording** — one-click capture of the modulated output (voice + soundboard) to a timestamped WAV.
- **Push-to-modulate global hotkey** (hold key → effects on, release → bypass). Configurable in Settings → Hotkeys.
- **Glyph casting** — draw a triangle / inverted triangle / square / circle on the Mixer to instantly switch to the bound preset, and **record your own glyphs** bound to any action.
- **Spell-circle stream overlay** — a transparent / chroma-key, OBS-ready always-on-top window.
- **System tray** — minimize to tray so audio keeps running in the background during calls/games.
- **First-run wizard** with VB-Cable detection + device picker + Discord routing instructions, plus a **"Test my setup"** routing diagnostic and **guided mic calibration** that auto-sets the noise gate.
- **Light theme** alongside the arcane color moods, and a **privacy-respecting in-app update check** (one-way version read, no telemetry).
- **What’s new** — after an update, a dismissible banner opens release notes covering everything since the version you were last on. The notes are compiled into the app, so the panel downloads nothing and always matches the build you’re running.
- **Sub-30 ms end-to-end latency** on consumer hardware (sub-26 ms even with the phase-vocoder pitch shifter active).
- **Automatic sample-rate matching** via rubato — mismatched mic/output rates no longer hard-fail.

## What it doesn't do (and won't)

- Named celebrity voice clones. Persona archetypes only — cloning ("Your voices") clones **your own** recorded voice, and AI voice convert is bring-your-own-model.
- **Emotion labels.** "Voice reading" reports what a *signal* is — loud, flat, wide, fast, bright — and will never report what a person *is*. Not squeamishness: the axis people mean by "what emotion is this" is carried by the words, not the voice (strip prosody and a model's valence score largely survives; strip the words and its arousal score collapses), and this app pitch-shifts, bitcrushes and ring-modulates on top of that — a formant shift alone costs a published four-class emotion model 18 points of accuracy. On a modulated voice the claim would be indefensible, and under EU AI Act Recital 18 the word on screen is the whole difference between a meter and a regulated emotion-recognition system. The vocabulary is a closed list in `divora-core/src/dsp/reading.rs`, guarded by a test.
- Cloud, accounts, telemetry, upsells.
- Cross-platform (yet). Windows only for v1.

## Roadmap & known limitations

DivoraVoice is feature-complete for v1 and shipping steady post-1.0 releases. Major work since 1.0 includes on-device TTS ("Speak"), voice cloning (VoxCPM accent-preserving + OpenVoice timbre), the Minecraft-mob and vintage-radio voice cast, eight new DSP effects (compressor, de-esser, tremolo, warble, breath, radio band-pass, vintage noise, bitcrusher), MIDI / Stream Deck control surfaces, the stream overlay, and reliability hardening. The detailed per-release plan lives in [docs/PLAN.md](docs/PLAN.md).

**Known limitations (today):**

- **Windows only.** macOS / Linux aren't in scope for v1 (the audio path is cpal/WASAPI; a cross-platform pass is "further out").
- **VB-Cable required** to send your voice to other apps — the app detects and prompts, but doesn't bundle it (licensing).
- **AI voice conversion** falls back to passthrough when no ONNX runtime/model is present — it never blocks the app, but the effect is simply unavailable until a model is installed.
- **Text-to-speech ("Speak")** runs fully on-device with bundled preset voices (Kokoro-82M + espeak-ng). The voice assets ship in the installer, so synthesis is available in the **built app**; running from source shows "voice not installed" until the assets are fetched into the resource dir (`scripts/fetch-voice-assets.ps1`), same as the AI voice-conversion models.
- **Custom voices ("Your voices")** — the cloning models (accent ~1.6 GB, timbre ~157 MB) download on demand the first time, not bundled, so the installer stays small.
- **Recording** is WAV only.
- **Soundboard** plays one folder at a time and doesn't recurse into subfolders.
- **Installers are unsigned** — Windows SmartScreen shows a warning (see [Install](#install) for the one-time "More info → Run anyway"). The in-app update check is a one-way version read (no telemetry); auto-install is intentionally not included.

**Further out (standard semver):** VST3 host, OBS WebSocket, a community preset registry, and eventually cross-platform. See [docs/PLAN.md](docs/PLAN.md) for specifics.

## Requirements

- Windows 10 / 11
- A virtual audio cable, e.g. [VB-Cable](https://vb-audio.com/Cable/) (free, donationware). The app prompts you on first run if it's missing.

## Building from source

Prerequisites: Rust stable (MSRV 1.80), Node 22+, pnpm 10+, **cmake** (needed to build the vendored libopus C source pulled in by symphonia's Opus adapter).

```powershell
pnpm install
pnpm tauri dev      # development run
pnpm tauri build    # release MSI / NSIS
```

CI runs on every push to `main` across four jobs (Windows):

- **Frontend** — `pnpm typecheck`, `pnpm test`
- **E2E** — `pnpm test:e2e` (Playwright)
- **Rust** — `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`
- **Build** — `pnpm tauri build --debug --no-bundle`

See [docs/PLAN.md](docs/PLAN.md) for the per-phase ship workflow.

## Architecture

- **Frontend**: SolidJS + TypeScript + Tailwind (Vite-built).
- **Shell**: Tauri 2 (frameless window, custom titlebar).
- **Backend**: Rust — `cpal` for WASAPI I/O, `realfft` + a hand-rolled phase vocoder for pitch / formant / harmonizer, `rubato` for sample-rate matching, `nnnoiseless` for RNNoise suppression, `biquad` for EQ, and `symphonia` (+ `symphonia-adapter-libopus`) for soundboard decode. `ort` (ONNX Runtime, loaded dynamically) powers the AI voice-convert effect and the on-device cloning models; text-to-speech runs Kokoro-82M + espeak-ng on-device. Supporting crates include `tokenizers` (VoxCPM BPE), `ndarray` (ONNX tensor I/O), `hound` (WAV recording + saved Speak clips), `midir` (MIDI control surfaces), and `ureq` (on-demand model download). The audio engine, DSP, presets, and soundboard live in the standalone `divora-core` crate (unit-testable without a UI); the Tauri shell (`src-tauri`) owns the IPC surface and window.

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

Critter Chatter is inspired by the babble speech of games like Animal Crossing; DivoraVoice is not affiliated with or endorsed by Nintendo, and uses no game audio.

**Text-to-speech ("Speak"):** synthesis uses [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) (Apache-2.0); text→phoneme conversion uses [**espeak-ng**](https://github.com/espeak-ng/espeak-ng) (**GPL-3.0**). espeak-ng is **not linked into DivoraVoice** — its binary (`espeak-ng.exe` + `libespeak-ng.dll` + `espeak-ng-data`) is bundled as a **separate, arm's-length component invoked via subprocess** (FSF "mere aggregation"), so DivoraVoice's own code stays MIT. The corresponding espeak-ng source is available under GPL-3.0 at <https://github.com/espeak-ng/espeak-ng> (the v1.52.0 release we bundle). **Voice cloning** ("Your voices") uses two permissive engines, both downloaded on demand (not bundled): [OpenVoice v2](https://github.com/myshell-ai/OpenVoice) (**MIT**) for timbre-only tone-color conversion (v1.20.0), and [VoxCPM-0.5B](https://github.com/OpenBMB/VoxCPM) (**Apache-2.0**, code + weights) for accent-preserving cloning (v1.24.0). Accent clones are reranked best-of-N (v1.25.0) by a [WeSpeaker](https://huggingface.co/onnx-community/wespeaker-voxceleb-resnet34-LM) VoxCeleb ResNet34-LM speaker-verification model (**CC-BY-4.0**, attribution-only), also downloaded on demand. No new copyleft.
