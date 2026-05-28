# Changelog

All notable changes to Divora are documented here. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning is **phase = minor** until v1.0 (see [docs/PLAN.md](docs/PLAN.md)).

## [Unreleased]

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
