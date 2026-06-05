# Contributing to DivoraVoice

Thanks for considering a contribution. This document covers the development workflow and house style.

## Development setup

### Prerequisites

- **Rust** stable (install via [rustup](https://rustup.rs/))
- **Node.js** 22 or newer
- **pnpm** 10 or newer (`npm install -g pnpm`)
- **cmake** — `symphonia-adapter-libopus` (→ `opusic-sys`) and `nnnoiseless` vendor C sources built via cmake
- **Windows 10/11** (we're Windows-only for now)
- **VB-Cable** installed (for end-to-end manual testing of the virtual mic flow)

### First-time setup

```powershell
pnpm install
pnpm tauri dev
```

`pnpm tauri dev` builds the Rust backend, starts the Vite dev server for the frontend, and launches the Tauri window. Hot reload works for both Rust and frontend code.

## Project layout

See `docs/ARCHITECTURE.md` for a full breakdown. The short version:

- `src-tauri/` — Rust backend. Audio engine, DSP effects, Tauri command handlers.
- `src/` — SolidJS frontend.
- `presets/` — JSON preset definitions bundled into the binary.
- `docs/` — design docs and operational notes.

## Per-release ship workflow

DivoraVoice ships one tagged release at a time. Each follows the **same loop**, in order:

1. **Implement** the scope.
2. **Tests** — Rust unit tests (DSP blocks, pure helpers) + Vitest unit/component tests for the frontend.
3. **Pre-push checklist** (all must pass — this mirrors [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)):
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `cargo test --workspace --all-features`
   - `pnpm typecheck`
   - `pnpm test`
   - `pnpm tauri build --debug --no-bundle`
4. **Update [`CHANGELOG.md`](../CHANGELOG.md)** — a dated entry with Added / Changed / Fixed, the test deltas, and the local pre-push results.
5. **Commit** with a detailed multi-line message.
6. **Push** to `main`.
7. **Watch CI** until green.
8. **Tag** `vX.Y.Z` only after CI is green.

**Versioning.** Pre-1.0, minor = phase number (Phase 1 → `v0.1.x`, etc.). **From `v1.0.0` on, full semver applies**, and the contracts in [`docs/STABLE-SURFACE.md`](STABLE-SURFACE.md) change in an **additive-only** way across the 1.x line (guarded by serialization tests in CI).

## Code style

### Rust

- `cargo fmt` formatting is enforced in CI.
- Clippy runs with `-D warnings`. Don't suppress lints without a justifying comment.
- No allocations on the audio callback thread. (Audit any `Vec::push`, `Box::new`, etc. inside the audio callback.)
- Audio thread = no locks. Use `triple_buffer`, atomic types, or SPSC ring buffers (`ringbuf` crate) for state shared with non-realtime threads.

### TypeScript

- Strict mode enabled. No `any` without a justifying `// HACK:` comment.
- Components use SolidJS function-component style with `createSignal`/`createMemo` for state.
- Tailwind for styling. No CSS modules unless absolutely necessary.

### Commit messages

- Subject line under 70 characters. Conventional-commit style, e.g. `feat(v1.7.0): loudness normalization (auto-gain + limiter)`.
- Blank line, then a paragraph explaining the *why*.
- Bullet list for notable details (file moves, behavior changes, dependency additions).
- When Claude collaborated, end with a `Co-Authored-By: Claude <model> <noreply@anthropic.com>` trailer naming the model used.

## Tests

### Rust / DSP unit tests

Each DSP effect carries its own `#[cfg(test)]` module asserting **algebraic / property** invariants rather than golden WAVs — e.g. passthrough at mix 0, energy added once warm, output stays finite on NaN / extreme input, the wet tail adds no dry-path latency, behavior across sample-rate changes. Co-locate new tests with the effect they cover (e.g. `divora-core/src/dsp/reverb.rs`). Run the whole workspace with `cargo test --workspace --all-features`.

The engine and audio-device tests skip live cpal/WASAPI enumeration when `CI=true` (the headless runner can fault under parallel enumeration); real hardware runs them.

There is also one `#[ignore]`d LLVC integration test that threads the real ONNX model — run it locally with the runtime DLL present.

### Frontend tests

- **Vitest** for unit and component tests (`pnpm test`). Store derivations, command wrappers, and design-system components each have specs co-located as `*.test.ts(x)`.

### Manual checklist

[`docs/MANUAL_TESTS.md`](MANUAL_TESTS.md) is the pre-release manual test list — the device-level paths (audio routing, virtual mic, tray, recording) that can't run headless. Walk through it before tagging a release that touches those flows.

## Filing issues

- Bugs: please include OS version, audio devices in use, and (if possible) a short capture of the problem.
- Feature requests: link to relevant `docs/PLAN.md` phase if applicable. Out-of-scope ideas welcome — we'll either pull them into a future phase or open a separate "ideas" thread.

## License

By contributing, you agree your code will be licensed under the MIT license (see [LICENSE](../LICENSE)).
