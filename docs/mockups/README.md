# Handoff: DivoraVoice — Real-Time Voice Modulator (Desktop)

## Overview
DivoraVoice is a **free, open-source, local-first real-time voice modulator for Windows**. It captures the
user's microphone, applies a chain of DSP effects (pitch, formant, reverb, EQ, robot, distortion, echo, noise
gate), and routes the modulated audio to other apps (Discord, Zoom, OBS, games) through a virtual audio cable
(VB-Cable). It also has a soundboard, a preset system, and a first-run setup wizard.

The product's visual identity is **"spellcraft for your voice"** — a calm, precise utility tool (Linear/Raycast
bones) wearing a dusk-lit arcane skin. The signature element is the **Mixer's spell circle**: the user's voice
is a glowing core, and effects orbit it as sigil-nodes connected by threads of light.

Stack the real app targets: **Tauri 2 + SolidJS + Tailwind**. Window: **1100×720** default, 900×600 minimum.
Dark theme is the default. **No account, no cloud, no telemetry** — this is a brand pillar and must be reflected
in the UI (persistent "LOCAL · NO ACCOUNT" affirmation, an "On-device" shield in the sidebar, a privacy-forward
About panel).

## About the Design Files
The files in `prototype/` are a **design reference built in HTML + React (via in-browser Babel)**. They are a
high-fidelity, fully interactive prototype demonstrating the intended look, motion, and behavior — **not
production code to copy line-for-line**. The implementation task is to **recreate these designs in the target
stack (Tauri + SolidJS + Tailwind)** using that environment's idioms (Solid signals/stores instead of React
hooks, Tailwind utility classes / CSS variables instead of inline style objects, real Web Audio / Rust DSP
instead of the simulated meters). Treat the HTML as the source of truth for layout, spacing, color, type, and
interaction.

Open `prototype/Divora.html` in any modern browser to explore the live reference. Use the toolbar's **Tweaks**
panel to see the supported visual variations.

## Fidelity
**High-fidelity (hifi).** Final colors, typography, spacing, iconography, motion, and interactions are all
specified here and in the prototype. Recreate pixel-faithfully, then swap the simulated audio for real DSP.

---

## Design Tokens

All tokens live in `prototype/divora/styles.css` under `:root`. Reproduce them as Tailwind theme tokens / CSS
custom properties.

### Color — Surfaces (default "Dusk Violet" mood; violet-tinted near-blacks)
| Token | Hex | Use |
|---|---|---|
| `--bg` | `#07060d` | Behind the window / stage |
| `--surface-0` | `#0d0a16` | App canvas |
| `--surface-1` | `#14101f` | Panels, sidebar, titlebar |
| `--surface-2` | `#1a1528` | Cards |
| `--surface-3` | `#221b34` | Raised / hover |
| `--surface-4` | `#2c2442` | Active / pressed, toggle track |

### Color — Lines
| Token | Value |
|---|---|
| `--line` | `rgba(168,150,220,0.10)` |
| `--line-strong` | `rgba(168,150,220,0.18)` |
| `--line-glow` | `rgba(124,92,246,0.40)` |

### Color — Text
| Token | Hex |
|---|---|
| `--text-hi` | `#ECE7F8` |
| `--text-mid` | `#A99FC4` |
| `--text-lo` | `#6E6590` |
| `--text-dim` | `#4b4368` |

### Color — Accent identity (the brand gradient = "alive / modulating"; use sparingly)
| Token | Value |
|---|---|
| `--indigo` | `#6D5BF0` (brightened brand indigo for dark UI) |
| `--indigo-deep` | `#4F46E5` (true brand indigo, the app icon) |
| `--pink` | `#EC4899` |
| `--pink-deep` | `#DB2777` (true brand pink, the app icon) |
| `--grad` | `linear-gradient(120deg, #4F46E5 0%, #7C5CF6 42%, #DB2777 100%)` |
| `--grad-soft` | `linear-gradient(120deg, rgba(79,70,229,0.22), rgba(219,39,119,0.22))` |

The app icon (already exists) is a white "D" on this indigo→pink gradient; the gradient is the one true accent.

### Color — Semantic
| Token | Hex | Meaning |
|---|---|---|
| `--success` | `#34D9A0` | armed / ok / "no telemetry" affirmations (emerald) |
| `--warning` | `#E9B14C` | candlelit gold — VB-Cable missing, caution |
| `--danger` | `#F2567A` | crimson-rose — muted, delete, panic |
| `--info` | `#58C6F2` | cyan — hints |
| each has a `-bg` variant at ~12% alpha, plus `--accent-bg: rgba(124,92,246,0.14)` |

### Radii
`--r-sm 7px`, `--r-md 11px`, `--r-lg 16px`, `--r-xl 22px`, `--r-pill 999px`

### Spacing scale (4px base)
`--s1 4 · s2 8 · s3 12 · s4 16 · s5 20 · s6 24 · s7 32 · s8 40 · s9 56`

### Shadows / glows
- `--shadow-1` `0 1px 2px rgba(0,0,0,.4)`
- `--shadow-2` `0 6px 22px rgba(0,0,0,.45)`
- `--shadow-3` `0 18px 50px rgba(0,0,0,.55)`
- `--glow-accent` `0 0 0 1px rgba(124,92,246,.35), 0 0 30px rgba(124,92,246,.35)`

### Typography
| Role | Family | Notes |
|---|---|---|
| Display (headings, preset names, big moments) | **Bricolage Grotesque** | weights 400–800 |
| UI / body / tabular numbers | **Space Grotesk** | weights 400–700; use `font-variant-numeric: tabular-nums` for meters |
| Mono (technical readouts, eyebrows, hotkeys, badges) | **Space Mono** | 400/700 |

Type scale: `--t-display 34 · t-h1 24 · t-h2 19 · t-h3 16 · t-body 14 · t-sm 12.5 · t-xs 11 · t-micro 9.5` (px).
**Never use Inter/Roboto/Arial.** Minimum on-screen text ~11px (eyebrows/mono labels); body 13–14px.

Helper classes worth recreating: `.eyebrow` (mono, 9.5px, 0.22em tracking, uppercase, `--text-lo`),
`.gradtext` (gradient-clipped text), `.mono`, `.tnum`.

---

## Iconography — the Sigil system
All icons are **custom geometric "arcane" SVGs**, not a stock icon set. They live in `prototype/divora/sigils.jsx`
as a `Sigil` component (`<Sigil name="…" size={n} />`) keyed by name, all stroked with `currentColor` on a
24×24 viewBox. Recreate as a Solid component or inline SVGs. Names in use:

- **Effects:** `pitch, formant, reverb, eq, robot, distortion, echo, gate`
- **Nav:** `mixer` (orbit circle), `soundboard` (2×2), `presets` (stacked diamonds), `settings` (rune wheel)
- **Status:** `clean` (ring+dot), `modulated` (radiant starburst), `muted` (crossed circle), `monitor` (headphones), `mic`, `output` (speaker)
- **Brand/identity:** `lock`, `shield`, `bolt`, `ab`, `wave`, `eye`, `keyboard`, `refresh`
- **Utility:** `play, stop, chevronR/D/L, check, x, plus, search, drag, folder, download, external, trash, copy, info, warning, github`

There is also a `DMark` component — the white "D" on the gradient rounded square (the app icon), used in the
titlebar, sidebar, wizard, and About.

---

## App Shell

**Window** = a custom frameless desktop window, 1100×720. Three regions stacked:

1. **Titlebar** (height 40, `--surface-1`, bottom hairline): `DMark` (20px) + "DivoraVoice" wordmark
   (Bricolage 15px) + a thin divider + a live status pill (colored dot + "Clean/Modulated/Muted", dot shimmers
   when modulated). Right side: a mono "🔒 LOCAL · NO ACCOUNT" affirmation + Windows min/max/close controls.
   This bar is the OS drag region.
2. **Sidebar** (width 80, icon rail, `--surface-1`, right hairline): four nav items (Mixer, Board, Presets,
   Settings), each a stacked sigil + uppercase mono micro-label. Active item: `--surface-3` bg, indigo icon with
   glow, and a 3px gradient bar pinned to the left edge. Footer: a "replay first-run" bolt button + a green
   shield badge ("Local-first · no telemetry").
3. **Content** = the active screen (fills remaining space).

---

## Screens / Views

### 1. Mixer (default / hero)
**Purpose:** see and shape the active preset's effect chain; control voice state and routing.

**Layout** (`padding 20px 24px`, flex column):
- **Header row:** preset glyph chip (38px, preset color) · preset name (Bricolage 26px) · tag badge
  ("Bundled"/"User") · subtitle ("N of M runes active · routed to CABLE Input"). Right: "Compare" label + an
  **A/B segmented toggle** (gradient when active) + a mute icon-button.
- **Main row** (flex, stretch): `[IN meter] [spell circle, flex:1, centered] [OUT meter] [right rail 290px]`.
- **Input/Output meters:** vertical "energy columns" (12px wide, 320px tall), segmented fill that runs
  emerald→gold→crimson bottom-to-top, with a **peak-hold cap** (white 2px line). dB readout below in mono.
  Eyebrow label "IN"/"OUT" above.

**The spell circle** (signature element — see `prototype/divora/spell_circle.jsx`): a 440px square containing,
center-out: an ambient radial glow; emanating pulse rings; an outer decorative ring with runic tick marks; a
rotating mid "orbit" ring of dashes; an orbiting spark; threads from the core to each enabled effect; the
**voice core** (a 132px disc showing the status sigil + label: Clean / Modulated / Muted); and the **effect
nodes** placed evenly around the orbit (50px sigil discs with the effect name + a live parameter readout
beneath). Enabled nodes glow indigo and are threaded to the core; disabled nodes are dim/dashed. Clicking a node
selects it (inspector in the right rail); double-click toggles enabled.

**Right rail (290px), stacked cards:**
- **Voice status card** — big sigil + "Clean/Modulated/Muted" + helper text, tinted by state, with a shimmering status dot.
- **Push-to-modulate card** — eyebrow + bound key (`Kbd` "Space"); a segmented "Hold to apply / Hold to bypass"
  mode toggle; a large **hold button** ("Hold to test" → "APPLYING"/"BYPASSED" while held). Holding **Space**
  (or pressing the button) drives the modulation live.
- **Monitor card** — sigil + label "Hear yourself in headphones" + a toggle.
- **Selected-rune inspector** — the selected effect's sigil + name + enable toggle + description + its parameter
  sliders (bipolar sliders have a center tick).

**Voice state logic:** `effectiveModulated = (ptmMode === "apply") ? pressed : !pressed`.
`status = muted ? "muted" : (anyEffectEnabled && effectiveModulated) ? "modulated" : "clean"`. State drives the
core visuals, the status pill in the titlebar, and the Voice status card.

### 2. Soundboard
**Purpose:** trigger sound clips into the modulated output.
**Layout:** header (eyebrow "Soundboard" + folder name with a folder sigil + "change folder" ghost button);
right side a search input (260px) + a **danger "Stop all (n)"** button (disabled when nothing plays). Below, a
**4-column grid** of clip tiles (each 120px tall, `--surface-2`, radius 14). Tile contents: a big emoji top-left,
a hotkey `Kbd` badge top-right (if assigned), and at the bottom the label + a color dot + duration (mono). A
final dashed **"Add clip"** tile. **Playing feedback:** the tile gets a colored border + glow, the emoji scales
up, a circular **progress ring** (SVG, stroke-dashoffset) draws in the top-right, and a "PLAYING" mono tag +
countdown appear. Empty/no-match state uses the shared EmptyState.

### 3. Presets (browse + edit)
**Purpose:** browse bundled/user presets and edit their effect chain.
**Layout:** two columns. **Left list (248px, `--surface-1`):** "Presets" title + a new-preset icon button; the
list grouped "Bundled · N" / "User · N"; each row = color glyph chip + name + "N runes" + a green "in use" dot
on the active preset. **Right editor:** header (color glyph chip 44px + name + tag + "in use" badge +
description + a primary **"Use"** button). An action row: eyebrow ("Effect chain · n/m active · drag to reorder")
+ ghost buttons Duplicate / Export JSON / Save as… / a **danger** Delete. Then the **chain editor** — a vertical
list of effect cards. Each card: drag handle, effect sigil chip, name + readout, enable toggle, remove (×); when
enabled it expands to a 1- or 2-column grid of parameter sliders. Cards are **drag-reorderable** (HTML5 drag;
drop target highlights indigo). An "Add rune to chain" dashed button opens a dropdown of effects not yet in the
chain. **Export JSON** opens a modal showing the preset serialized as JSON with Copy / Save .json actions.

### 4. Settings
Single scrolling column (max-width 680, centered). Sections, each = sigil + display title + optional desc + a
panel of rows:
- **Audio devices** — Input device picker (custom `Select`), an **Input confirmation** horizontal meter (live),
  Output device picker.
- **Virtual microphone** — VB-Cable **detection card**: detected (emerald check, version, "Re-scan" button) vs
  missing (gold warning, "why" copy, primary "Download" w/ external icon). A tiny "⌁ preview missing/detected
  state" dev link toggles the demo state. Then 3 app cards (Discord/Zoom/OBS) with **screenshot placeholders**
  (the real app should drop in actual screenshots) + the menu path, and an info line "choose `CABLE Output` as
  the microphone."
- **Hotkeys** — three `HotkeyCapture` rows (Push to modulate, Panic, Toggle monitor). Capture shows pressed keys
  as `Kbd` chips; clicking arms a "Press a key…" capture state.
- **Glyph casting** — see "Easter egg / feature" below; 4 rows (Triangle, Inverted triangle, Square, Circle),
  each a shape glyph + a preset dropdown.
- **Appearance** — Theme segmented (Dark active; Light = "soon"); Parchment grain toggle (adds a noise overlay).
- **About DivoraVoice** — DMark + version + "MIT License · Tauri + SolidJS" + GitHub button; three pillar cards
  (No telemetry / No account / Free forever); a "Replay setup" button.

### 5. First-run wizard (4 steps)
Full-window overlay. **Left ceremonial panel (372px):** gradient wash + a slowly rotating sigil ring + DMark
wordmark + a breathing core glyph (changes per step) + a vertical **stepper** (Welcome → Virtual cable → Devices
→ Ready) with check marks on completed steps. **Right content:** per-step copy.
1. **Welcome** — headline "Your voice, transmuted in real time." + the four brand-pillar cards (Local-first,
   Private, Free, Real-time).
2. **Virtual cable** — explains VB-Cable; detection card (detected/missing, with a download link when missing).
3. **Devices** — input picker + live "hearing you" meter + output picker + a "send to CABLE Input" hint.
4. **Ready** — success check + "route into Discord" numbered card.
Footer: "Skip setup" + Back + a primary Continue / "Enter DivoraVoice".

---

## Components (the design system)
Reproduce these as reusable components (see `prototype/divora/components.jsx` + `components.css`):
- **Button** — variants `primary` (gradient, glow), `secondary` (surface-3 + border), `ghost`, `danger`
  (+ `.solid`); sizes sm / default / lg; optional leading/trailing sigil; focus ring `rgba(124,92,246,.4)`.
- **IconButton** — 34px square, hover surface, `active` = accent; optional tooltip.
- **Toggle** — 40×23 switch, knob slides; gradient when on (also `danger`/`success` tones).
- **Slider** — 4px track, gradient fill, 14px white thumb with soft halo (grows on hover); `bipolar` variant
  fills from center with a center tick.
- **Badge** — mono uppercase pill; tones default/accent/success/warning/danger/info. **Kbd** — keycap chip.
- **Segmented** — small tab group; selected = surface-4 (or gradient with `accent`).
- **Vertical & Horizontal level meters** — emerald→gold→crimson fills with a white peak-hold cap.
- **Select / Device picker** — 42px field (sigil + main + sub + chevron) opening a styled dropdown with check on
  the selected option.
- **HotkeyCapture** — dashed field; click → "capturing" (shimmer) → captures the next keychord into `Kbd` chips.
- **EmptyState** — dashed-ring glyph + display title + helper copy + optional action.
- **Tooltip**, **Card**/**Panel** surfaces.

---

## Interactions & Behavior
- **Sidebar nav** swaps screens (keep state per screen). Active indicator = gradient bar + glow.
- **A/B compare** on the Mixer toggles between two stored states of the preset (wire to real A/B snapshots).
- **Push-to-modulate**: holding the bound key (default **Space**) sets `pressed`; combined with the apply/bypass
  mode it flips voice state. Bind globally in the real app (works when backgrounded).
- **Sliders / toggles** edit the live effect chain; changes reflect immediately in node readouts, the inspector,
  and (in production) the audio graph.
- **Soundboard**: clicking a tile starts playback (progress ring animates over the clip duration, then clears);
  "Stop all" stops everything (this is also the global "panic").
- **Presets editor**: drag-reorder chain cards; enable/disable per effect; add/remove effects; export JSON.
- **Meters** are driven by a smooth pseudo-audio generator in the prototype (`useLevel`); replace with real
  RMS/peak from the audio pipeline. Keep the peak-hold decay behavior.

### Motion (default "rich"; controllable via Tweaks)
- Voice core **breathes** and glows when modulated; threads animate a dash-flow; the orbit ring + outer ring +
  an orbiting spark slowly rotate; **pulse rings** emanate from the core; particles drift up when modulated.
- Status dot shimmers when modulated. Slider thumb halo grows on hover. Cards lift slightly on hover.
- All durations scale by a `motion` factor (functional=0 → static, ambient=0.6, rich=1). Honor
  `prefers-reduced-motion` in production by mapping it to "functional".

### Tweaks (visual-style variations — prototype affordance)
Exposed in the prototype's Tweaks panel; in production these map to app settings/themes:
- **Mystical level** (subtle / balanced / rich): how much arcane decoration the spell circle draws (outer ring,
  tick marks, constellation dots).
- **Motion** (functional / ambient / rich): animation intensity (see above).
- **Color mood** (Dusk Violet / Ink+Candle / Midnight): swaps the surface/line palette via a `data-mood`
  attribute on the root (see the `:root[data-mood="…"]` blocks in `styles.css`).
- **Accent** (Brand / Abyssal / Ember): swaps `--grad`.
- **Texture** (Parchment grain on/off, Vignette on/off): overlays.

### Easter egg / feature — Glyph casting
The background listens (passively, never blocking UI) for a left-click **drag**. While dragging on empty space it
emits a **trail of fading sparks** (canvas particle system, `prototype/divora/spark_canvas.jsx`). If the drag
forms a recognized **closed shape**, it **casts the bound preset**: a burst of sparks traces the shape, the
preset name blooms in its color ("◆ SPELL CAST ◆"), and the app switches to that preset on the Mixer.
- Recognized glyphs: **Triangle ▲**, **Inverted triangle ▽**, **Square ▢**, **Circle ◯** (detected via
  Ramer–Douglas–Peucker corner counting + radial-uniformity for the circle).
- Bindings are user-editable in **Settings → Glyph casting**. Defaults: Triangle → *Velvet Demon*,
  Inverted triangle → *Glass Oracle*, Square → *Hollow King*, Circle → *Clean Passthrough*.
- The spark + reveal are drawn on a single `<canvas>` (pointer-events: none, z-index 58) via requestAnimationFrame.

---

## State Management
Top-level app state (see `prototype/divora/app.jsx`):
- `nav` — active screen.
- `presetId` + `chains` — the active preset and an editable copy of every preset's effect chain
  (`{ id, enabled, vals:{…} }[]`). Data lives in `prototype/divora/data.jsx` (`EFFECTS`, `EFFECT_ORDER`,
  `PRESETS`, `DEVICES_IN`, `DEVICES_OUT`, `SOUNDBOARD`).
- `ui` — `{ muted, monitor, ab, ptmMode, ptmKey, pressed }`.
- `wizard` — first-run overlay open/closed.
- `tweaks` — visual variation knobs (→ production theme/settings).
- `glyphs` — the shape→preset binding map for glyph casting.
- In production, add: device selection, VB-Cable detection status, hotkey bindings, soundboard folder + playing
  clips, and the real audio engine state. Persist user data locally (no cloud).

### Effect catalog (parameters)
| Effect | Params (min…max unit, default) |
|---|---|
| Noise Gate | threshold −80…−20 dB (−52) |
| Pitch | shift −12…+12 st (0, bipolar) |
| Formant | shift −10…+10 (0, bipolar) |
| EQ | low/mid/high −12…+12 dB each (0, bipolar) |
| Robot | carrier 40…400 Hz (120), mix 0…100% (70) |
| Distortion | drive 0…100% (35) |
| Echo | time 40…800 ms (240), feedback 0…90% (35) |
| Reverb | size 0…100% (40), mix 0…100% (25) |

Bundled presets: **Hollow King, Static Wraith, Velvet Demon, Choir of Ash, Clean Passthrough**; user presets:
**Deep Warden, Glass Oracle** (each with a color, glyph, description, and chain — see `data.jsx`).

---

## Assets
- **App icon**: white "D" on the indigo→pink gradient (`#4F46E5` → `#DB2777`) — already exists; recreated here as
  the `DMark` SVG component.
- **All other icons**: the custom `Sigil` SVG set (no third-party icon library).
- **Fonts**: Bricolage Grotesque, Space Grotesk, Space Mono (Google Fonts; self-host in production).
- **Emoji** are used only on soundboard tiles (sample content; user-configurable).
- **Settings → Virtual microphone** uses three *screenshot placeholders* — the real app should supply actual
  screenshots of Discord/Zoom/OBS mic settings.
- No raster images are required by the chrome; everything else is vector/CSS.

## Screenshots
The `screenshots/` folder contains high-res renders of the current design (default tweaks: Dusk Violet mood,
brand accent, rich motion):
- `01-mixer-clean.png` — Mixer, voice **Clean** (bypassed).
- `02-mixer-modulated.png` — Mixer, voice **Modulated** (push-to-modulate held): lit threads, particles, status flip.
- `03-soundboard.png` — Soundboard grid with two clips playing (progress rings + glow) and "Stop all".
- `04-presets.png` — Presets list + chain editor for Hollow King.
- `05-settings-devices.png` — Settings: Audio devices + Virtual microphone (VB-Cable detected).
- `07-wizard-welcome.png` — First-run wizard, step 1 (brand pillars + ceremonial panel).
- `08-glyph-cast.png` — Glyph casting: a drawn **square** conjures **Hollow King** ("◆ SPELL CAST ◆").

(The Settings → Glyph casting binding UI is described under "Easter egg / feature"; it sits below the Hotkeys
section on the Settings page.)

## Files (in `prototype/`)
- `Divora.html` — entry; loads fonts, styles, and the React/Babel scripts, then mounts.
- `divora/styles.css` — tokens, base, stage scaling, keyframes, grain/vignette overlays.
- `divora/components.css` — component styles (buttons, sliders, meters, select, etc.).
- `divora/sigils.jsx` — `Sigil` icon set + `DMark`.
- `divora/components.jsx` — shared components + the `useLevel` meter hook.
- `divora/data.jsx` — effect catalog, presets, devices, soundboard data.
- `divora/spell_circle.jsx` — the Mixer's spell-circle visualization.
- `divora/screen_mixer.jsx` / `screen_soundboard.jsx` / `screen_presets.jsx` / `screen_settings.jsx` / `screen_wizard.jsx` — the five screens.
- `divora/spark_canvas.jsx` — background spark trail + glyph-casting recognizer + on-canvas reveal.
- `divora/app.jsx` — titlebar, sidebar, routing, top-level state, mount.
- `divora/tweaks.jsx` + `divora/tweaks-panel.jsx` — the Tweaks panel (prototype-only affordance).

> Note: the prototype is React-with-inline-JSX transpiled in the browser for convenience. The target stack is
> Tauri + SolidJS + Tailwind — port the structure and styling, don't ship the in-browser Babel setup.
