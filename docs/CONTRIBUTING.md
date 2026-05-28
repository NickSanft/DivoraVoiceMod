# Contributing to Divora

Thanks for considering a contribution. This document covers the development workflow and house style.

## Development setup

### Prerequisites

- **Rust** stable (install via [rustup](https://rustup.rs/))
- **Node.js** 20 or newer
- **pnpm** 9 or newer (`npm install -g pnpm`)
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

## Per-phase ship workflow

Divora ships in phases. Each phase follows the **same loop**, in order:

1. **Implement** the phase's scope.
2. **Tests** — unit (DSP blocks, pure helpers) + Playwright e2e (UI smoke).
3. **Pre-push checklist** (all must pass):
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --workspace`
   - `pnpm typecheck`
   - `pnpm test`
   - `pnpm test:e2e` (when wired up)
   - `cargo tauri build --debug`
4. **Commit** with a detailed multi-line message.
5. **Push** to `main`.
6. **Watch CI** until green.
7. **Tag** `vX.Y.Z` only after CI is green.
8. **Roll into the next phase.**

**Pre-1.0 versioning rule:** minor = phase number; patch = bug fixes within a phase. Phase 1 → `v0.1.x`, Phase 2 → `v0.2.x`, etc. After `v1.0.0`, full semver applies.

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

- Subject line under 70 characters.
- Blank line, then a paragraph explaining the *why*.
- Bullet list for notable details (file moves, behavior changes, dependency additions).
- Always end with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` when Claude collaborated.

## Tests

### DSP unit tests

Every DSP effect must have a golden-WAV test:

1. Load `tests/fixtures/<effect>_input.wav`.
2. Run the effect with a documented parameter set.
3. Compare to `tests/fixtures/<effect>_expected.wav` within an `f32` epsilon (~1e-4).
4. To intentionally update the golden, run with `--bless` (a CLI flag we wire into the test binary).

### Frontend tests

- Vitest for unit and component tests.
- Playwright (via Tauri webdriver) for end-to-end smoke flows.

### Manual checklist

`docs/MANUAL_TESTS.md` is the pre-release manual test list. Run through it before tagging a release that touches audio routing or virtual mic flow.

## Filing issues

- Bugs: please include OS version, audio devices in use, and (if possible) a short capture of the problem.
- Feature requests: link to relevant `docs/PLAN.md` phase if applicable. Out-of-scope ideas welcome — we'll either pull them into a future phase or open a separate "ideas" thread.

## License

By contributing, you agree your code will be licensed under the MIT license (see [LICENSE](../LICENSE)).
