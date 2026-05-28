# Manual Pre-Release Test Checklist

Run this checklist before tagging any release that touches audio capture, output routing, or virtual mic detection. CI cannot exercise these flows because they require real hardware and live calls.

## Setup

- [ ] Fresh install on a clean machine (or wipe `%APPDATA%\Divora\`).
- [ ] VB-Cable installed.
- [ ] At least one wired headset connected.
- [ ] Discord, Zoom, and OBS Studio installed.

## Smoke

- [ ] App launches without console errors.
- [ ] All UI tabs reachable (Mixer / Soundboard / Presets / Settings).
- [ ] No background CPU usage above ~3% while idle.

## Audio passthrough

- [ ] Select physical mic as input → speakers as output → toggle self-monitor.
- [ ] Speaking into mic produces sound from headset with no glitches for 60 seconds.
- [ ] Round-trip latency feels under 30 ms (no perceptible echo of own voice).

## Presets

- [ ] Default preset list loads on first run.
- [ ] Switching between presets is glitch-free (no clicks, no momentary silence).
- [ ] Edit a preset, save it, restart app — changes persist.
- [ ] User-edited preset is not overwritten by an app update (simulate by manually replacing bundled file).

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
