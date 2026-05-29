# Manual Pre-Release Test Checklist

Run this checklist before tagging any release that touches audio capture, output routing, or virtual mic detection. CI cannot exercise these flows because they require real hardware and live calls.

## Setup

- [ ] Fresh install on a clean machine (or wipe `%APPDATA%\DivoraVoice\`).
- [ ] VB-Cable installed.
- [ ] At least one wired headset connected.
- [ ] Discord, Zoom, and OBS Studio installed.

## Smoke

- [ ] App launches without console errors.
- [ ] All UI tabs reachable (Mixer / Soundboard / Presets / Settings).
- [ ] No background CPU usage above ~3% while idle.

## Audio passthrough (Phase 2)

- [ ] Open Settings → Audio devices on first launch.
- [ ] Both input and output device pickers populate with at least one device each.
- [ ] Default devices are pre-selected (marked with "default" in the sub-label).
- [ ] Engine shows "Stopped" or auto-started "Running at N Hz" depending on hardware compatibility.
- [ ] Click Start (if stopped). Status changes to "Running at <rate> Hz" with no error banner.
- [ ] Input level meter (HMeter under "Input level") moves when you speak into the mic.
- [ ] Switch input device while running → engine restarts with the new device without crashing.
- [ ] Switch output device while running → engine restarts; sound flows from the new output.
- [ ] Toggle Monitor off → speaker / headphones go silent within ~50 ms; IN meter still moves; OUT meter falls to zero.
- [ ] Toggle Monitor on → sound resumes within ~50 ms.
- [ ] Click Stop → engine reports "Stopped"; IN and OUT meters drop to zero.
- [ ] Round-trip latency feels under 30 ms (no perceptible echo of own voice when monitor is on).
- [ ] Open Mixer → both vertical IN and OUT meters move while engine is running; peak-hold caps decay over ~1 s.
- [ ] When engine is stopped, Mixer shows "Engine offline" card with a clickable Settings link.
- [ ] Sample-rate mismatch test: pick devices with different default sample rates (e.g., 44.1 kHz mic + 48 kHz output) → start fails with a clear "sample-rate mismatch" error message.

## Effects + Spell Circle (Phase 3)

- [ ] Open Mixer; spell circle is drawn with all effects orbiting the voice core.
- [ ] Enabled effects show light-gradient threads to the core; disabled effects show dashed grey threads.
- [ ] Click an effect node → it gets a focus ring + the right-rail Inspector updates to show its sigil + name + parameters.
- [ ] Double-click an effect node → enabled flag flips; the thread style switches accordingly.
- [ ] Voice core says "Clean" by default; hold Space → flips to "Modulated"; the core breathes and threads animate dash-flow; particles drift up.
- [ ] In the Inspector, drag the Gate threshold slider → audible change within ~5 ms; OUT meter responds.
- [ ] In the Inspector, drag the EQ Low slider → audible bass boost / cut.
- [ ] In the Inspector, drag the Distortion drive → audible saturation that doesn't clip past unity.
- [ ] In the Inspector, drag the Reverb mix → tail builds smoothly.
- [ ] In the Inspector, drag the Echo time → repeats spacing changes.
- [ ] In the Inspector, drag the Robot mix → voice gains the carrier tone.
- [ ] Toggle an effect off in the Inspector → instant bypass; sound passes through cleanly.
- [ ] Switch Tweaks → Mystical between subtle / balanced / rich → outer ring + tick marks + constellation dots appear/disappear.
- [ ] Switch Tweaks → Motion to functional → animations stop; rich → animations run.
- [ ] Switch the Color mood → spell circle colours follow.

## Presets (Phase 4)

- [ ] Default preset list loads on first run — 5 bundled presets visible in the left list under "Bundled · 5", "User · 0".
- [ ] Clicking a different preset in the list immediately switches the active preset (left dot moves; Mixer header updates).
- [ ] Switching between presets is glitch-free (no clicks, no momentary silence — DSP graph swap on next buffer).
- [ ] Right editor shows the active preset's header (glyph chip, name, Bundled/User badge, "In use" badge, description) and chain cards.
- [ ] Each ChainCard shows the effect's sigil, name, current readout, and an enable toggle.
- [ ] Enabling a chain card expands it to show parameter sliders; disabling collapses them.
- [ ] Drag a chain card's handle onto another card → drop target highlights indigo → release reorders the chain and the audio engine updates within one buffer.
- [ ] Save button is disabled on Bundled presets.
- [ ] Delete button is disabled on Bundled presets.
- [ ] Duplicate on a Bundled preset creates a User preset with " Copy" appended; switches to it; left list shows it under "User".
- [ ] Edit a duplicated preset (move a slider), click Save → preset persists; restart app → changes survive.
- [ ] Delete a user preset → file disappears from `%APPDATA%\DivoraVoice\presets\`; left list updates; active preset falls back to the first remaining one.
- [ ] Manually replacing a bundled JSON file via a build does not overwrite any user preset.
- [ ] Manually corrupting a user preset JSON does not crash the app — it's silently skipped from the list.

## A/B compare (Phase 4)

- [ ] On the Mixer, the A/B segmented control starts on A.
- [ ] Edit some parameters in slot A.
- [ ] Click B → audio reverts to the unedited chain; visually the spell circle's threads + Inspector update.
- [ ] Edit different parameters in slot B.
- [ ] Click A → the original slot A edits return; slot B's are now banked.
- [ ] Switching presets resets A/B back to A with both slots equal to the new preset's chain.

## Export JSON (Phase 4)

- [ ] Click Export JSON on the Presets editor → modal opens with the preset serialised as pretty JSON.
- [ ] Click Copy → toast / inline indicator confirms; pasting into a text editor produces the exact JSON.
- [ ] Click Save .json → browser/system save dialog with default file name = preset name; file content matches the modal.
- [ ] Close the modal (× or backdrop click) → modal dismisses.

## Soundboard (Phase 5)

- [ ] Open the Soundboard screen on first run → empty-state "No folder picked yet" shows with a Pick a folder button.
- [ ] Click Pick a folder → native OS folder picker opens.
- [ ] Pick a folder with ~20 audio files (mixed mp3, wav, ogg, flac) → every supported file appears as a tile with the file's stem as the label.
- [ ] Tiles are sorted alphabetically.
- [ ] Tiles for unsupported files (e.g. `.txt`, `.png`) do not appear.
- [ ] The folder path is shown in the header next to the folder sigil.
- [ ] Click a tile → backend decodes (first play), playback starts, tile gets a coloured border + glow, emoji scales up, a circular progress ring fills clockwise, a `Xs` countdown shows in the corner.
- [ ] Clip plays through the engine output (mixed with the modulated mic if the engine is running).
- [ ] Click the same tile again before the first play finishes → cached buffer plays a second voice; both should be audible (8+ voices supported).
- [ ] Trigger 8 clips simultaneously → all play without dropouts; the Stop-all button shows "Stop all (8)".
- [ ] Click Stop-all → every voice silences within ~5 ms; the Stop-all button becomes disabled; all tiles reset to idle.
- [ ] Search "bell" (or whatever matches) → grid filters to matching labels.
- [ ] Clear the search → all tiles return.
- [ ] Switch folders via Change folder → previous tiles disappear, new ones appear.
- [ ] Change folder to a directory with no audio files → "No audio files in this folder" empty state shows.
- [ ] Delete a clip file from the folder while DivoraVoice is open → next folder scan removes it from the grid; queued plays of that id continue to play from cache until they finish.
- [ ] (Phase 5 limitation) Binding a tile hotkey only fires while DivoraVoice is the focused window. Global hotkeys arrive in Phase 6.

## Soundboard — sample-rate handling

- [ ] Drop a 44.1 kHz WAV into the folder, play it on a 48 kHz engine → playback pitch matches the original (not chipmunked).
- [ ] Drop a 22.05 kHz file → playback still pitch-correct, with mild interpolation softness.

## Virtual mic (Phase 6)

- [ ] Settings → Virtual microphone with VB-Cable **missing** → gold warning banner reads "VB-Cable not detected", Download button visible.
- [ ] Click Download → default browser opens https://vb-audio.com/Cable/.
- [ ] Install VB-Cable, click Re-scan → banner flips to emerald "VB-Cable detected" within ~1 s; routing hint lists `CABLE Input (VB-Audio …)` and `CABLE Output (VB-Audio …)` by name.
- [ ] Three call-app instruction cards (Discord / Zoom / OBS) appear under the banner with the exact menu path.
- [ ] Audio devices section now lists CABLE Input as a selectable output device — picking it routes the modulated voice into the cable.
- [ ] In Discord call, set input device to "CABLE Output (VB-Audio Virtual Cable)" → other party hears modulated voice.
- [ ] In Zoom, same test, with screen share active.
- [ ] In OBS, add "Audio Input Capture" of CABLE Output → OBS audio meter moves while speaking.
- [ ] Uninstall VB-Cable, click Re-scan → banner flips back to gold "VB-Cable not detected"; instruction cards disappear; Download button returns.

## Hotkeys (Phase 6)

- [ ] Settings → Hotkeys shows 3 rows: Push-to-modulate (defaulted to Space), Panic, Toggle monitor.
- [ ] Click the Push-to-modulate field → "Press a key… (Esc to cancel)" hint shows.
- [ ] Press a chord (e.g., Ctrl+Shift+P) → chip set updates to show `Ctrl` `Shift` `P` and the binding is registered with the backend.
- [ ] Focus a different app (e.g., Notepad), press the bound chord → DivoraVoice's Mixer status flips to Modulated while held.
- [ ] Release the chord → Mixer status returns to Clean within one buffer.
- [ ] Bind Panic to a key (e.g., F8). Trigger 4 soundboard clips. Focus another app. Press F8 → all clips stop immediately.
- [ ] Bind Toggle monitor to a key (e.g., F9). Focus another app. Press F9 → engine monitor toggles in DivoraVoice; Settings reflects the new state on next focus.
- [ ] Click Clear next to any binding → chip set clears, accelerator becomes empty, the OS hotkey stops firing.
- [ ] Close and reopen the app → bindings survive (re-registered via `syncHotkeyBindings`).
- [ ] Bind PTM to an invalid accelerator (e.g., a single modifier) → the in-app field accepts it but the backend log records the registration failure; nothing crashes.

## Push-to-modulate (focus-time fallback)

- [ ] With DivoraVoice focused and PTM = Space, hold Space → status flips to Modulated; the Spell circle threads animate; particles drift.
- [ ] Release Space → status flips back to Clean within one buffer (~5 ms).
- [ ] Rebind PTM to a different key (e.g., Ctrl+M) in Settings → the in-app fallback honours the new key immediately (no app restart needed).
- [ ] Invert mode (`ui.ptmMode = "bypass"`) inverts: PTM held = Clean; idle = Modulated.

## Glyph casting bindings (Phase 6)

- [ ] Settings → Glyph casting shows 4 rows: Triangle, Inverted triangle, Square, Circle.
- [ ] Each row has a glyph chip on the left + a preset dropdown filled with every preset (Bundled + User).
- [ ] Change Triangle's binding to "Static Wraith" → `app.glyphs.triangle` updates; restart preserves it.
- [ ] (Phase 7) Casting the glyph on the Mixer switches the active preset to the bound id.

## Appearance — Mystical / Grain / Vignette (Phase 6)

- [ ] Mystical · subtle → outer decorative ring + tick marks + constellation dots fade from the Mixer spell circle.
- [ ] Mystical · balanced → middle level (no constellation dots, outer ring still present).
- [ ] Mystical · rich → full set of arcane glyph decorations visible.
- [ ] Parchment grain on → faint paper-noise overlay on the surface backgrounds.
- [ ] Parchment grain off → clean surfaces, no overlay.
- [ ] Vignette on → soft darkening around the window edges.
- [ ] Vignette off → uniform surface brightness.

## Pitch shifter (Phase 9)

- [ ] Pick the **Hollow King** preset on the Mixer (defaults to pitch −5 st).
- [ ] Start the engine, speak. Your voice should sound roughly half an octave lower **without** the "hearing yourself twice" doubling that v0.3.0 had.
- [ ] Switch to **Static Wraith** (pitch +2 st). Voice should sound *slightly* higher; no doubling, no audible warbling on sustained vowels.
- [ ] In Presets editor, open any preset with pitch enabled; sweep the slider from −12 to +12 in 1-st increments. Each change should produce an audible pitch step without crackles.
- [ ] Disable pitch (toggle off). Voice should immediately return to original pitch — no STFT latency, no reconstruction noise.

## Formant shifter (Phase 9)

- [ ] In Presets editor, open Velvet Demon (formant −5). Speak — voice should sound *darker / heavier* but the fundamental should not move.
- [ ] Switch to Choir of Ash (formant +4). Voice should sound *brighter / lighter* without moving up in pitch.
- [ ] Sweep formant from −10 to +10 on a single preset. The colour change should be smooth; no clicks, no NaN / silence dropouts.
- [ ] Combine pitch and formant on the same chain (Hollow King: pitch −5, formant −3). Each stage should compose without interfering: the pitch goes down AND the formant darkens, separately.

## Sample-rate mismatch (Phase 9)

- [ ] Pick an input device at 48 kHz (most USB mics) and an output device at 44.1 kHz (laptop speakers often are). v0.8.x would have refused to start with `SampleRateMismatch`. v0.9 should start cleanly and you should hear yourself in monitor mode.
- [ ] Switch input to a 44.1 kHz device with the output still 44.1 kHz — engine should restart without going through the resampler (no extra latency).
- [ ] Switch back to a 48 kHz input — engine should restart with the resampler engaged; latency should still feel sub-30 ms.
- [ ] Engine status pill at the top should read "Running at <input_rate> Hz" — i.e. the engine rate is the input rate.

## Cast cursor alignment (Phase 8)

- [ ] On the Mixer, click Cast (or press G).
- [ ] Start dragging from anywhere on the overlay. The glowing trace stays directly under the cursor — no left/down offset, no titlebar/sidebar drift.
- [ ] Drag a clear square. The classifier resolves it to "square" and the bound preset switches.
- [ ] Resize the window mid-cast (or before opening) — the trace still tracks the cursor without re-snap.
- [ ] Open cast at the top of the window, drag near the very top: the trace starts at the cursor (not 36 px below it).

## Soundboard tile reordering (Phase 8)

- [ ] Pick a folder with ≥ 6 audio clips. Tiles appear sorted alphabetically.
- [ ] Drag tile A onto tile D — the drop target shows a dashed border + accent glow.
- [ ] Release: the order updates immediately (A is now in D's slot, D and intermediate tiles shift up).
- [ ] Restart the app. The custom order survives.
- [ ] Drop a new audio file into the folder, click Change folder (re-pick the same folder). The new file appears at the end of the grid; the saved order isn't disturbed.
- [ ] Hover or hold-drag does not start playback — clicks alone trigger play.

## Per-tile colors (Phase 8)

- [ ] Right-click a tile. A small palette popover appears at the click point with 8 swatches.
- [ ] Click "Pink" → the tile's dot, glowing border (when playing), and progress ring all switch to pink immediately.
- [ ] Right-click again, click "Reset to default" → tile returns to the per-id hashed default colour.
- [ ] Click outside the popover → it dismisses without changing anything.
- [ ] Restart the app — color choices survive.

## Recent folders (Phase 8)

- [ ] Pick three different soundboard folders in sequence. The most-recently-picked one is active.
- [ ] Header now shows a "Recent" ghost button next to "Change folder".
- [ ] Click Recent — dropdown lists the three folders, most-recent at top.
- [ ] Click any entry — the folder switches and the scan runs (without invoking the native dialog).
- [ ] Click the × next to a folder in the dropdown — that entry vanishes.
- [ ] Pick a 6th folder — the oldest of the previous 5 falls off the bottom (cap = 5).
- [ ] Restart — the recent list survives.

## Global tile hotkeys (Phase 8)

- [ ] Pick a soundboard folder with a clip.
- [ ] Click the tile and bind a global hotkey (e.g. Ctrl+F1) — the Kbd chip appears on the tile.
- [ ] Alt-tab to Notepad (or any other focused app). Press Ctrl+F1. The clip plays through the engine (and into VB-Cable if installed).
- [ ] Restart the app. Without re-binding, press Ctrl+F1 again from Notepad — clip still plays (the binding was re-registered on `syncHotkeyBindings`).
- [ ] Clear the hotkey from the tile — pressing Ctrl+F1 from Notepad no longer fires the clip.
- [ ] No double-fire: with DivoraVoice focused, pressing the bound hotkey plays the clip exactly once.

## About (Phase 6)

- [ ] Settings → About shows the DMark + "DivoraVoice v0.6.0" + "MIT License · Tauri + SolidJS".
- [ ] Click View on GitHub → default browser opens https://github.com/NickSanft/DivoraVoiceMod.
- [ ] Three pillar cards visible: No telemetry, No account, Free forever.
- [ ] Click Replay setup → wizard ceremony re-opens.

## First-run wizard (Phase 7)

- [ ] Fresh install (or wipe `localStorage.divora.wizardSeen`). Launch DivoraVoice → the wizard appears as a full-screen overlay over the Mixer.
- [ ] The ceremonial left rail shows: DMark + "DivoraVoice", a breathing sigil in the middle that updates icon per step (clean → output → mic → modulated), and a step pill list with the current step highlighted.
- [ ] Step 1 (Welcome): "Your voice, transmuted in real time." display headline + 4 pillar cards (Local-first, Private, Free, Real-time). Continue button advances.
- [ ] Step 2 (Virtual cable): VB-Cable detection card. Without VB-Cable, banner is gold "VB-Cable not found" + Download button. With VB-Cable installed, banner is emerald "VB-Cable is installed" + routing hint listing the cable's device name.
- [ ] Step 2: Click Download → default browser opens https://vb-audio.com/Cable/.
- [ ] Step 2: Install VB-Cable then click Re-scan → banner flips to emerald.
- [ ] Step 3 (Devices): Microphone and Modulated-out selects appear with the same devices as Settings → Audio devices. Hearing-you HMeter responds to mic input. Switching devices here affects the live engine.
- [ ] Step 4 (Ready): emerald checkmark, "You're ready." headline, Discord routing card with 3 numbered steps. "Enter Divora" button finishes.
- [ ] Click Skip setup on any step → wizard closes, `localStorage.divora.wizardSeen = "true"`.
- [ ] Click Enter Divora on the last step → wizard closes, `divora.wizardSeen = "true"`.
- [ ] Restart the app → the wizard does NOT re-appear.
- [ ] Settings → About → Replay setup → wizard re-appears at Step 1.
- [ ] Back button on any step > 1 walks the user backward.
- [ ] Escape key from the wizard does NOT close it (use Skip / Enter Divora instead — this is intentional to avoid accidental dismissal).

## Glyph casting (Phase 7)

- [ ] In Settings → Glyph casting, bind:
  - Triangle → Static Wraith
  - Inverted triangle → Hollow King
  - Square → Velvet Demon
  - Circle → Clean Passthrough
- [ ] On the Mixer, the preset header shows a "Cast" ghost button next to the Compare A/B toggle.
- [ ] Click Cast → a dusk-violet veil covers the Mixer; a floating card reads "Cast a glyph — press and drag to trace one of: triangle, inverted triangle, square, or circle. Releases switches to the bound preset. Press Esc to cancel."
- [ ] Press and drag a triangle (apex up): on release, active preset becomes Static Wraith; a "triangle → Static Wraith" flash toast pops at the bottom of the Mixer for ~2.4 s.
- [ ] Press and drag an inverted triangle (apex down): preset becomes Hollow King; toast confirms.
- [ ] Press and drag a square: preset becomes Velvet Demon.
- [ ] Press and drag a circle: preset becomes Clean Passthrough.
- [ ] Draw something ambiguous (e.g. an open curve, a tight scribble): toast reads "Glyph not recognised — try again". The active preset does not change.
- [ ] Unbind triangle in Settings (clear the dropdown) then cast a triangle: toast reads "No preset bound to triangle".
- [ ] Press Esc during a cast → overlay dismisses without classifying.
- [ ] Click outside any active drag (without dragging) → cast overlay stays open (the user can try again).
- [ ] Press G with the Mixer focused and no input field active → cast overlay opens, same as the button.
- [ ] Press G while typing in the Soundboard search box → does NOT open the overlay (text input takes precedence).
- [ ] Trace a glyph that goes off-screen: classification still works (only the visible-ish portion is captured).
- [ ] Cast back-to-back: each cast resets state cleanly; the overlay opens fresh every time.

## Stress

- [ ] Switch presets rapidly (1 per second) for 30 seconds — no crashes, no leaks.
- [ ] Play a soundboard clip while changing input device — recovers gracefully.
- [ ] Pull headset cable mid-call — app shows device-disconnected state without crashing.

## Shutdown

- [ ] Closing the window stops all audio streams within 1 second.
- [ ] No orphaned device handles (check Sound Settings → app should disappear from Volume Mixer).
- [ ] Reopening picks up the same device selection.
