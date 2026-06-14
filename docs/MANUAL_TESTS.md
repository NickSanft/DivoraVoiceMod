# Manual Pre-Release Test Checklist

Run this checklist before tagging any release that touches audio capture, output routing, or virtual mic detection. CI cannot exercise these flows because they require real hardware and live calls.

> **v1.0 release gate:** before cutting `v1.0.0`, the *entire* checklist must pass on a clean Windows machine — it's the manual test pass the roadmap calls for. The per-feature sections below cover Phases 2–16; the [STABLE-SURFACE.md](STABLE-SURFACE.md) contract is what we're committing to keep working across the 1.x line.

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

## Monitor volume (v1.6.0)

- [ ] Pick a **separate monitor device** (Settings → Audio, "Hear yourself" / monitor picker — e.g. your headphones, distinct from the stream output) and start the engine with Monitor on.
- [ ] On the Mixer, the Monitor card shows a **Volume** slider with a `%` readout (defaults to 100 %).
- [ ] Drag the slider up → you hear yourself **louder** in the monitor device; drag down toward 0 % → your sidetone fades to silence. The stream/main output level is unaffected.
- [ ] The slider is disabled (greyed) when Monitor is toggled off.
- [ ] Set a non-default volume (e.g. 150 %), restart the app → the slider restores to 150 % and the level matches on the next engine start.
- [ ] When engine is stopped, Mixer shows "Engine offline" card with a clickable Settings link.
- [ ] Sample-rate mismatch test: pick devices with different default sample rates (e.g., 44.1 kHz mic + 48 kHz output) → start fails with a clear "sample-rate mismatch" error message.

## Loudness normalization (v1.7.0)

- [ ] On the Mixer, the **Loudness** card shows an enable toggle (off by default) and a **Target** slider (−30…−6 dB, default −18 dB).
- [ ] Start the engine, enable Loudness, and speak: a live **"Auto gain"** readout appears showing the makeup gain (e.g. `+4.2 dB`); it's hidden while the card is off or the engine is stopped.
- [ ] Pick a **quiet** preset (e.g. Whisper Wraith) → the auto gain reads positive (boosting). Switch to a **loud/deep** preset (e.g. Leviathan) → the auto gain drops (or goes negative), and the *perceived* output level stays roughly the same across the switch. With Loudness **off**, the same switch produces an obvious level jump.
- [ ] Drag the Target slider toward −6 dB → output gets louder; toward −30 dB → quieter. Slider is disabled (greyed) while the card is off.
- [ ] Speak loudly / shout with Loudness on → output never audibly clips (the limiter + ceiling hold it under full scale); the OUT meter peak stays at/below the top.
- [ ] Silence test: stop talking with Loudness on → background hiss is **not** cranked up (the gain holds rather than chasing silence).
- [ ] Enable Loudness + set a target (e.g. −12 dB), restart the app → the toggle and target restore, and re-apply on the next engine start.
- [ ] Soundboard interaction: play a clip while talking with Loudness on → the clip plays at its own (master-gain) level; it isn't auto-leveled with the voice.

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

- [ ] Default preset list loads on first run — the bundled presets visible in the left list under "Bundled · N", "User · 0" (N = the bundled count, currently 16 incl. the cast + Clean Passthrough + Deep Narrator).
- [ ] (v1.6.0) Clicking a different preset in the list **previews** it — the row highlights and the right editor shows it — but does **not** change the active voice: the green dot and the Mixer header stay on the active preset.
- [ ] (v1.6.0) Click **Use** on the previewed preset → it becomes active (green dot moves; Mixer header updates). The Use button is disabled and labelled "In use" once that preset is active.
- [ ] (v1.6.0) Preview a non-active preset and move one of its sliders while the engine runs → your live voice does **not** change (you're editing the preview, not the active chain). Then click Use → the edit takes effect live.
- [ ] (v1.6.0) Preview a non-active preset on the Presets screen, then switch to the Mixer → the Mixer edits the **active** voice (not the preview); returning to Presets, the preview has resynced to the active preset.
- [ ] Switching the active preset (via Use) is glitch-free (no clicks, no momentary silence — DSP graph swap on next buffer).
- [ ] Right editor shows the **previewed** preset's header (glyph chip, name, Bundled/User badge, description) and chain cards. The "In use" badge appears only when the previewed preset is the active one **and** the engine is running.
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

## Live device switching (Phase 11)

- [ ] With the engine running, open Settings → Audio devices. Change the input device dropdown to a different mic. The engine should restart on the new device within ~200 ms; the IN meter responds to the new mic. No manual Stop / Start needed.
- [ ] Repeat with the output device — switch from `Headphones` to `CABLE Input (VB-Audio Virtual Cable)`. The engine restarts; subsequent speech routes through the cable, and a Discord client listening on `CABLE Output` hears it.
- [ ] Stop the engine. Change devices in Settings. No restart should happen (the engine stays stopped); the next manual Start uses the new selections.
- [ ] Re-select the *same* device from the dropdown — no restart (no audible click; engine stays running on the same stream).

## Cast trail + SPELL CAST reveal (Phase 11)

- [ ] On the Mixer, click Cast (or press G). Start drawing a triangle. As you drag, sparks should trail behind the cursor, fading out over ~700 ms.
- [ ] On release, if the classifier matches the bound preset, a centred panel pops in over the Mixer: the preset's glyph (large, in its brand colour) above the preset name (also in its colour) above the `Bundled` / `User` tag. The "◆ SPELL CAST ◆" eyebrow shows above. The panel breathes for ~1 s then fades.
- [ ] After the reveal dismisses, you're back on the Mixer with the new preset's chain loaded.
- [ ] Draw a glyph that's unrecognised. The flash toast at the bottom reads "Glyph not recognised — try again"; no reveal animation.
- [ ] Unbind a glyph in Settings → Glyph casting and cast it. Flash reads "No preset bound to *glyph*"; no reveal.

## Soundboard → microphone (Phase 11)

- [ ] Pick a folder of soundboard clips. A small info-toned chip below the header reads "Clips play through your selected output device — including your modulated mic, so Discord / Zoom / OBS callers hear them."
- [ ] In Settings → Audio devices, set output to `CABLE Input (VB-Audio Virtual Cable)`.
- [ ] Join a Discord call with another person. Set your Discord input to `CABLE Output`.
- [ ] Play a soundboard clip. The other party hears it, mixed with your modulated voice. Speak over the clip — both reach the other end.
- [ ] Stop the clip via Stop-all. The other party stops hearing the clip immediately.

## RNNoise denoiser (Phase 10)

- [ ] Pick a 48 kHz input device (most USB mics). Make sure the engine reports `Running at 48000 Hz` in Settings.
- [ ] On the Mixer, edit any preset to add `denoiser` to the chain with `mix = 80%`. Enable it.
- [ ] Make a deliberately noisy environment: room fan on, mechanical keyboard typing, dishwasher in the background. Speak normally.
- [ ] Listen on the monitor — the voice should sound noticeably cleaner with the denoiser on vs. off; the background noise floor drops by ~6–12 dB without the voice sounding muffled.
- [ ] Toggle the denoiser off in the chain — noise floor returns immediately.
- [ ] Switch to a 44.1 kHz input device. The denoiser should bypass automatically (no effect). Settings still shows the chain entry with denoiser enabled, but the voice is unaffected. (Phase 11 will add a resampler around it.)
- [ ] Sweep `mix` from 0 → 100 %. At 0, the voice is fully dry (no denoising). At 100, fully wet. Intermediate values produce smooth fade between the two.
- [ ] Disable the denoiser and re-enable. The first ~10 ms after re-enable should be silent (warm-up); then steady-state denoised audio.
- [ ] Stack denoiser with the existing noise gate. Both should compose without artifacts: gate cuts the truly-silent bits, denoiser cleans what's left.

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

- [ ] Settings → About shows the DMark + "DivoraVoice v<current build version>" — the real version stamped into the build (e.g. `v1.0.0`), **not** a hardcoded string — + "MIT License · Tauri + SolidJS". In a `tauri dev` build it reads `v0.0.0`; that's expected.
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

## AI voice conversion (Phase 12)

- [ ] Settings → Voice library. With **no** `onnxruntime.dll` present and no model installed, the runtime status reads "not detected" and the voice list is empty — the app still runs (Voice Convert is a passthrough).
- [ ] Install build that bundles `onnxruntime.dll` + the LLVC narrator model → runtime status flips to "detected"; the bundled voice appears in the list.
- [ ] Drop a `<name>.onnx` into the voices folder, click Refresh → it appears; a user file with the same id shadows a bundled one.
- [ ] Add `voice_convert` to a chain and select a model → after a brief background load, your voice converts to the target voice; there is **no** UI hang while the model loads.
- [ ] Select a model file that doesn't exist / is invalid → the effect degrades to passthrough and never hangs (no 60 s freeze).
- [ ] Disable Voice Convert → voice returns to the pre-conversion signal immediately.
- [ ] **Streaming (v1.3.0):** with the bundled narrator, the converted voice has *conversational* latency — talking over a call feels responsive, not the old ~quarter-second lag. The Mixer latency readout shows ~13 ms (vs ~256 ms for a non-streaming model). No clicks/seams between chunks during continuous speech.
- [ ] Confirm the converted audio reaches a Discord call when output = CABLE Input.

## Monitor output routing (Phase 13)

- [ ] Settings → Audio devices shows a **Monitor output** picker with a "None — use main output" option.
- [ ] Set output = `CABLE Input (VB-Audio Virtual Cable)`, monitor = your headphones. Speak: you hear your modulated voice **on the headphones** while a Discord client on `CABLE Output` also hears it.
- [ ] Toggle Monitor **off** → the headphones go silent, but Discord still receives the voice (the main send is unaffected).
- [ ] Toggle Monitor **on** → headphones resume.
- [ ] Play a soundboard clip → it's audible **both** on the headphones (monitor) and to the Discord side.
- [ ] Set monitor = "None" → behaviour reverts to the legacy single-output monitor toggle (toggle mutes the main output).
- [ ] Pick a monitor device with a different sample rate than the input → no pitch shift, no crackle (the monitor path resamples).

## Latency readout (Phase 14)

- [ ] With the engine running and an empty/sample-by-sample chain, the Mixer header shows no latency suffix (or `+0 ms`).
- [ ] Enable **Voice Convert** with the bundled **streaming** narrator (v1.3.0) → header shows roughly `+13 ms` (not the old ~256 ms), and the converted voice is usable in live conversation. A non-streaming BYO model still shows ~256 ms.
- [ ] Enable the **denoiser** at 48 kHz → adds ~10 ms; enable **pitch** or **formant** → ~21 ms each. The number updates the instant you toggle an effect.
- [ ] Disable all latency-adding effects → the readout returns to ~0.
- [ ] Hover the readout → tooltip explains the contributors.

## Soundboard volume + system tray (Phase 15)

- [ ] Soundboard toolbar has a **master volume** slider. Lower it → all clips get quieter; raise it → louder (no clipping/distortion at the top).
- [ ] Right-click a tile → its context menu has a **per-tile volume** slider. Set one tile loud, another quiet → each plays at its own level × master.
- [ ] Restart the app → per-tile gains and the master gain survive (persisted).
- [ ] Restart the app → the last picked soundboard folder is restored automatically with its tiles + per-tile hotkeys (no re-pick needed).
- [ ] Minimize or close the window → the app **hides to a system-tray icon** instead of quitting; audio (and the VB-Cable route) keeps running.
- [ ] Left-click the tray icon (or "Show DivoraVoice") → the window restores + focuses.
- [ ] Tray menu **Quit** → the app actually exits (streams torn down, gone from the Volume Mixer).
- [ ] While hidden to tray, a bound global soundboard hotkey still fires the clip.

## Recording the modulated output (Phase 16)

- [ ] Mixer right rail shows a **Record** card. While the engine is **stopped**, the Record button is disabled (tooltip: "Start the engine to record").
- [ ] Start the engine → Record becomes enabled. Click **Record** → the dot turns red and pulses; the button switches to **Stop**.
- [ ] Speak for ~5 s with an effect chain active, then click **Stop**.
- [ ] Settings → Recordings shows the folder path + the last take's name. Click **Open folder** → the recordings folder opens.
- [ ] Open the `divora-<date>_<time>.wav` in a player → it plays back the **modulated** voice (effects applied), mono, no glitches, correct duration.
- [ ] Play a soundboard clip while recording → the clip is present in the recorded file (recording matches the call audio).
- [ ] Stop the engine while recording → the in-progress file is finalized and is playable (not truncated/corrupt); the Record indicator clears.
- [ ] Start a second recording → it writes a new timestamped file without disturbing the first.
- [ ] Shout into the mic (clip the input) → the recorded file saturates cleanly (no wrap-around static).

## The Coven — voice cast (v1.1.0)

- [ ] The sidebar shows a **Coven** nav item (second, after Mixer). Click it → a gallery of character cards appears.
- [ ] Fifteen cards are present: **Velvet Demon · Hollow King · Choir of Ash · Static Wraith · The Oracle · Seraph · Dirge · The Swarm · The Possessed · Leviathan · The Imp · Dispatch · Corrupted · Whisper Wraith · Deep Narrator**, each with its sigil (in its color), name, a DSP / AI Voice badge, and a lore blurb.
- [ ] **Range + utility (v1.5.0):** **Leviathan** is the deepest voice (below Hollow King); **The Imp** is a high, mischievous pitch-up; **Dispatch** is a clean bandlimited radio/comms voice; **Corrupted** is a bit-crushed, ring-modulated glitch; **Whisper Wraith** is airy/intimate/quiet.
- [ ] **Choir family (v1.4.0):** Summon each and confirm the chord character — **Seraph** is bright/major, **Dirge** is dark/minor with a low toll, **The Swarm** is a dissonant beating cluster, **The Possessed** is your voice doubled an octave (+ fifth) below. All are built on the adjustable Harmonizer.
- [ ] Start the engine on the Mixer, return to the Coven, click **Summon** on *Velvet Demon* → your voice instantly takes on the demon character; the card shows an "Active" badge + colored glow, and the button reads "Summoned".
- [ ] Summon a different member → the active highlight moves; the new character applies live.
- [ ] The Mixer header reflects the summoned preset (name + active rune count) — Coven and Mixer stay in sync.
- [ ] **Deep Narrator** card: with the AI model installed, Summon converts via the model; **without** the model, the card shows "AI model not installed — Summon uses the DSP voice," and Summon still applies the deep DSP character (no hang).
- [ ] Summon a DSP character after the narrator → the AI model is cleared (Settings → Voice library shows no active voice); summon the narrator again → the model reloads.
- [ ] Summon works while the engine is **stopped** too — it sets the active voice; starting the engine then applies it.
- [ ] **The Oracle** sounds calm/resonant at natural pitch (distinct from the pitched characters).
- [ ] **Choir of Ash** (v1.2.1) sounds like a **chord** — the dry root plus a diminished stack (+3 / +6 / +9 st) — not a single voice, thanks to the harmonizer.
- [ ] The **Harmonizer** effect is selectable in the Presets chain editor (Mix + three interval voices); the default 3/6/9 makes a diminished chord; setting the voices to e.g. 4/7 makes a major triad. Mix = 0 is a clean passthrough.
- [ ] The **Chorus** effect is also selectable (Mix + Depth); enabling it thickens/widens the voice; Mix = 0 is a clean passthrough.
- [ ] **Presets — "In use" badge**: with the engine **stopped**, no preset shows the green "In use" badge in the editor (previously every voice showed it). With the engine **running**, the active preset shows "In use."

## Stream overlay (v1.16.0)

- [ ] Settings → Stream overlay → toggle **Show overlay window** on → a frameless, always-on-top window appears showing the spell circle.
- [ ] On the Mixer, hold Space (modulate) / change the preset / move Mystical / switch the Color mood → the overlay's circle updates to match (status colour, node count, mystical detail, mood) within a frame.
- [ ] Set background = **Transparent** → in OBS, add a **Window Capture** of "DivoraVoice Overlay" with a method that supports transparency → the circle composites over your scene with no box.
- [ ] Set background = **Green** (or Magenta) → the overlay fills solid green → add a **Chroma Key** filter in OBS → the green is removed, leaving the circle.
- [ ] Close the overlay window's toggle (or the window's own close) → the window disappears and the toggle reflects it.
- [ ] Restart the app → the background-mode choice is restored.

## Speak — text-to-speech (v1.17.0)

Assets ship in the installer (bundled build). Running **from source** needs
`pwsh scripts/fetch-voice-assets.ps1` first (puts the model + espeak-ng into the
resource dir), same as the AI voice-conversion models.

**Built app (assets present):**

- [ ] Sidebar → **Speak** → text box + a 6-voice picker (US/UK) + **Speak** / **Stop**. No "Soon" badges, no "not installed" banner.
- [ ] Type a sentence → pick each preset voice → **Speak** → you hear that voice on the output; the voices are audibly distinct (US vs UK, M vs F).
- [ ] Set output = **CABLE Input** and join a Discord call → a listener hears the spoken text, mixed with your live mic.
- [ ] A long multi-sentence passage synthesizes as one continuous clip (sentence chunking, no dropouts); the playback **progress ring** tracks it; **Stop** halts it immediately.
- [ ] **Volume slider** (v1.18.0): lower it → Speak is quieter; raise it → louder. Persists across restart (`divora.ttsVolume`).
- [ ] **Preview only** (v1.18.0): with output = CABLE Input + a separate monitor device (headphones) + monitoring on, enable **Preview only** → Speak → you hear it in your headphones but a Discord listener does **not**. Disable it → the listener hears it again. Persists across restart (`divora.ttsPreviewOnly`).
- [ ] Pick a different voice → restart the app → the selection is restored (`divora.ttsVoice`).

**Degradation (no assets — e.g. dev run before fetch):**

- [ ] Voices show a **"Soon"** badge + a "Preset voices aren't installed yet" banner; pressing **Speak** shows a graceful "text-to-speech voices are not installed" notice — never a hang or crash (same as the AI voice-convert missing-model path).

**Your voices — cloning (v1.20.0 / download v1.21.0):**

- [ ] **First run (models not downloaded):** Speak → **Your voices** shows a **Download voice-cloning models (~157 MB)** button. Click it → a progress % appears → on completion the **Add your voice** controls replace it. (Models land in `%APPDATA%/DivoraVoice/tts/`; not in the installer.)
- [ ] Restart → the models are still present (no re-download); **Add your voice** is shown directly.
- [ ] Type a name → **Add your voice** → pick a clean 20–30 s WAV of a voice → after a moment it appears in the list and is auto-selected.
- [ ] **Auto base-match (v1.22.0):** the new voice shows a small **"based on …"** caption naming a preset (e.g. *based on Puck*). Clone a clearly female and a clearly UK-accented reference → the captioned base shifts toward a female / UK preset accordingly (not always Puck).
- [ ] Type text → **Speak** with the cloned voice → the output takes on that voice's timbre (accent/cadence come from the Kokoro base). It mixes into a call like any Speak output.
- [ ] **Delete** a cloned voice (trash icon) → it disappears; if it was selected, the selection falls back to a preset.
- [ ] Restart the app → cloned voices persist (stored under `%APPDATA%/DivoraVoice/voices/cloned/`); a selected cloned voice is restored.
- [ ] Pick a junk/too-short file → a graceful error appears (no crash).

## Stress

- [ ] Switch presets rapidly (1 per second) for 30 seconds — no crashes, no leaks.
- [ ] Play a soundboard clip while changing input device — recovers gracefully.
- [ ] Pull headset cable mid-call — app shows device-disconnected state without crashing.

## Shutdown

- [ ] Closing the window stops all audio streams within 1 second.
- [ ] No orphaned device handles (check Sound Settings → app should disappear from Volume Mixer).
- [ ] Reopening picks up the same device selection.
