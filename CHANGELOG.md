# Changelog

All notable changes to Divora are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning is **phase = minor** until v1.0 (see [docs/PLAN.md](docs/PLAN.md)).

## [Unreleased]

### Added

- `docs/mockups/` — high-fidelity design reference (README spec, HTML + React interactive prototype, screenshots) defining the "spellcraft for your voice" visual identity, spell-circle Mixer, glyph-casting easter egg, and Tweaks system.
- Detailed design system section in `docs/PLAN.md` (color tokens, type scale, components, window chrome, motion).
- Glyph casting (Phase 7), A/B preset compare (Phase 4), and user-facing Tweaks (Phase 6) added to v1 scope.

### Changed

- **Product name finalized as "DivoraVoice"** (brand parent: Divora). Updated `tauri.conf.json` productName + window title, `README.md`, `index.html` `<title>`, placeholder App.tsx + test, and Rust doc comments. Internal crate names (`divora-app`, `divora-core`) kept as is.
- **Phase plan restructured** — design system + app shell is now Phase 1; original Phase 1 (audio passthrough) becomes Phase 2; all downstream phases shifted by +1.
- Bundled preset names changed from generic archetypes (e.g. "Deep Narrator") to the design-spec names: **Hollow King, Static Wraith, Velvet Demon, Choir of Ash, Clean Passthrough**.
- DSP effect catalog and parameter ranges aligned with the design spec.
- Placeholder `src/styles.css` swapped Inter (forbidden by design) for system-ui until Phase 1 ships Bricolage / Space Grotesk / Space Mono.
- Placeholder background swapped from `#09090b` (zinc-950) to `#07060d` (dusk-violet `--bg`).

### Why it matters

The design mockups raised the visual ambition substantially. Restructuring the phase plan now (rather than discovering the shell's complexity mid-Phase 1) keeps every future phase a single tight unit of work that plugs into a known-good shell.

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
