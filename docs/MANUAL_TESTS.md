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

## Soundboard

- [ ] Pick a folder with ~20 audio files (mixed mp3, wav, ogg).
- [ ] Every file appears as a tile with correct label.
- [ ] Clicking a tile plays the sound mixed with modulated mic.
- [ ] Hotkey bound to a tile triggers it while window is unfocused.
- [ ] `Esc` panic button stops all currently playing clips.
- [ ] Trigger 8 clips simultaneously — no audio dropout.

## Virtual mic

- [ ] First run with VB-Cable missing → setup card visible, link opens browser.
- [ ] First run with VB-Cable present → CABLE Input auto-suggested as modulated output target.
- [ ] In Discord call, change input device to "CABLE Output (VB-Audio Virtual Cable)" → other party hears modulated voice.
- [ ] In Zoom, same test, with screen share active.
- [ ] In OBS, add "Audio Input Capture" of CABLE Output → meters move while speaking.

## Push-to-modulate

- [ ] Default Right Alt key applies active preset only while held.
- [ ] Release returns to clean voice within one buffer (~5 ms).
- [ ] Reassign hotkey via Settings → new key works.
- [ ] Invert mode ("hold to bypass") toggles direction correctly.

## Stress

- [ ] Switch presets rapidly (1 per second) for 30 seconds — no crashes, no leaks.
- [ ] Play a soundboard clip while changing input device — recovers gracefully.
- [ ] Pull headset cable mid-call — app shows device-disconnected state without crashing.

## Shutdown

- [ ] Closing the window stops all audio streams within 1 second.
- [ ] No orphaned device handles (check Sound Settings → app should disappear from Volume Mixer).
- [ ] Reopening picks up the same device selection.
