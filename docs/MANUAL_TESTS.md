# Manual Pre-Release Test Checklist

Run this checklist before tagging any release that touches audio capture, output routing, or virtual mic detection. CI cannot exercise these flows because they require real hardware and live calls.

> **v1.0 release gate:** before cutting `v1.0.0`, the *entire* checklist must pass on a clean Windows machine — it's the manual test pass the roadmap calls for. The per-feature sections below cover Phases 2–16; the [STABLE-SURFACE.md](STABLE-SURFACE.md) contract is what we're committing to keep working across the 1.x line.

## Setup

- [ ] Fresh install on a clean machine (or quit the app and wipe both `%APPDATA%\com.divora.voicemod\` — presets, voices, models, recordings, logs, and the stream overlay's pictures in `overlay\assets` (the installed app, and any `pnpm tauri build`, `--debug` included) and `overlay\assets-dev` (`pnpm tauri dev`) — and `%LOCALAPPDATA%\com.divora.voicemod\`, the WebView2 profile that holds every `divora.*` setting: wizard, devices, overlay placement and charms).
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
- [ ] Sample-rate mismatch (rewritten for v1.51.2 — this step used to expect a start failure, which has been impossible since v0.9 resampled instead): pick devices with different default sample rates (e.g. 44.1 kHz mic + 48 kHz output) → the engine starts, the Mixer header reads "resampling 44100 → 48000 Hz", Settings reads "Running, resampling: 44100 → 48000 Hz to the output", and **your voice sounds clean** — no buzz, no chop, no gaps. Before v1.51.2 this configuration mangled the audio badly enough that the input tone sat 41 dB under the noise.
- [ ] Monitor-only mismatch: set the separate monitor device to a rate the mic and main output share → the Mixer names the **monitor** ("resampling 48000 → 44100 Hz monitor"), not two identical numbers, and the headphone mix is clean too.

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
- [ ] Voice core says "Modulated" by default with a preset that has an effect on besides the noise gate (Push to modulate is off until the Mixer's card turns it on; since v1.58.0); the core breathes and threads animate dash-flow; particles drift up. With **Clean Passthrough** (the noise gate only) it says "Clean". Turn **Push to modulate** on (**Hold to apply**) → "Clean" until you hold Space → "Modulated".
- [ ] In the Inspector, drag the Gate threshold slider → audible change within ~5 ms; OUT meter responds.
- [ ] In the Inspector, drag the EQ Low slider → audible bass boost / cut.
- [ ] In the Inspector, drag the Distortion drive → audible saturation that doesn't clip past unity.
- [ ] **Bitcrusher (v1.48.0):** pick **Arcade Cabinet** → your voice sounds like a coin-op speech chip. In the Inspector drop **Bit depth** to 3 → obviously grittier and quieter syllables start dropping out; raise **Drive** → they come back. Drop **Rate** toward 2000 Hz → a metallic ringing that moves DOWN as your pitch goes up (that inharmonic motion is the point — Distortion cannot do it). Set Rate to 48000 with Bit depth 16 → effectively transparent. Check **Corrupted** still sounds right after its retrofit.
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
- [ ] Delete a user preset → file disappears from `%APPDATA%\com.divora.voicemod\presets\`; left list updates; active preset falls back to the first remaining one.
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

- [ ] Settings → Hotkeys shows nine rows: Push-to-modulate, Panic — stop all clips, Toggle monitor, Mute your voice (v1.56.0), Record screen, Record voice (WAV), Save replay, Turn Replay on or off, Arrange or lock the overlay. On a fresh profile every row is unbound (push-to-modulate's in-app key is still Space, and Push to modulate itself is off until the Mixer's card turns it on).
- [ ] Click the Push-to-modulate field → "Press a key… (Esc to cancel)" hint shows.
- [ ] Press a chord (e.g., Ctrl+Shift+P) → chip set updates to show `Ctrl` `Shift` `P` and the binding is registered with the backend.
- [ ] Turn **Push to modulate** on in the Mixer (**Hold to apply**). Focus a different app (e.g., Notepad), hold the bound chord → DivoraVoice's Mixer status reads Modulated while held, and your effects are on.
- [ ] Release the chord → the Mixer status returns to Clean within about 50 ms, occasionally about 100 ms (Divora checks every 50 ms whether a global key is still down), plus up to one level update (33 ms). *(P3 timed the level update that carries it, not the status: 12 to 76 ms after the key-up over 8 holds; see Push-to-modulate (v1.58.0).)*
- [ ] Bind Panic to a key (e.g., F8). Trigger 4 soundboard clips. Focus another app. Press F8 → all clips stop immediately.
- [ ] Bind Toggle monitor to a key (e.g., F9). Focus another app. Press F9 → engine monitor toggles in DivoraVoice; Settings reflects the new state on next focus.
- [ ] Click Clear next to any binding → chip set clears, accelerator becomes empty, the OS hotkey stops firing.
- [ ] Close and reopen the app → bindings survive (re-registered via `syncHotkeyBindings`).
- [ ] Bind PTM to an invalid accelerator (e.g., a single modifier) → the in-app field accepts it but the backend log records the registration failure; nothing crashes.
- [ ] (v1.56.0) Bind **Mute your voice** to a key another app already holds (e.g. a Discord or OBS keybind) → note what happens, in Divora and in that app. This build shows nothing on screen when Windows refuses a key: the refusal is only logged (`[hotkey] register failed` in the devtools console).

## Push-to-modulate (v1.58.0)

Since v1.58.0 push-to-modulate switches your effects in the engine: the call, your headphones, WAV and screen recordings and Replay follow it. Before, its key changed only the status. It is off until the Mixer's **Push to modulate** card turns it on. The real-app run below played no audio, so every check of the sound itself is an owner check (N1–N11).

- [ ] **Off by default.** On a fresh profile, Mixer → **Push to modulate**: the switch is off, the line under it reads "Off · your effects are always on.", **Hold to apply** / **Hold to bypass** and **Hold to switch · or press Space** are disabled, and the button's tooltip reads "Turn on Push to modulate to try it." With a preset that has an effect on besides the noise gate (e.g. **Hollow King**), the titlebar, the Mixer's status and the spell circle say **Modulated** and animate; with **Clean Passthrough** (the noise gate only) they say **Clean**. Holding Space changes nothing. *(e2e and unit tests; not run in the app.)*
- [ ] **Turn it on** → **Hold to apply** is picked, and the status says **Clean**. With no global key and no learned `ptm` MIDI mapping, the line reads "Your effects are on only while you hold Space in Divora's window. To use it in a call, set a key in Settings → Hotkeys." Hold Space → **Modulated**; let go → **Clean**. Pick **Hold to bypass** → the line reads "Your effects are off while you hold Space in Divora's window. …" and the status goes the other way round. Bind a global key in Settings → Hotkeys → the line ends at the key ("…while you hold F13."). With no global key but a `ptm` MIDI mapping that has learned its control, and a MIDI input picked in Settings → Control surfaces, it ends "…in Divora's window, or with your MIDI control." *(e2e and unit tests; not run in the app.)*
- [ ] **Hold to switch** (was **Hold to test**), with the card on: hold it down with the mouse → it reads **Effects on** (Hold to apply) or **Effects off** (Hold to bypass) and the status follows; let go, or drag off the button → it lets go. After a mouse click on the switch, a hold or this button, Space holds push-to-modulate (a click hands the keyboard back to the page). Tab to the button and hold Enter or Space → it holds until you let go or the focus moves. Its tooltip reads "Switches your effects for the call, your headphones, recordings and Replay while held." *(e2e and unit tests; not run in the app.)*
- [ ] **Remembered:** with the card on and **Hold to bypass** picked, quit from the tray and relaunch → the card is on, with **Hold to bypass**. Turn it off and relaunch → off; turn it on → **Hold to bypass** again. *(P5 reloaded the page with the card on: see below. A relaunch wasn't run.)*
- [ ] **The in-app key needs its modifiers:** bind push-to-modulate to **Ctrl+Shift+M** → in Divora a bare M does nothing. (Ctrl+Shift+M itself holds through the global key: Windows gives a registered key's key-down to the hotkey, not to Divora's page, so only the next case, which can't be a global key, checks the in-app hold.) Bind it to a key whose character changes with Shift (**Ctrl+Shift+1**, which Settings shows as **!**), hold it in Divora and let go of Shift first → it still lets go. A key held down doesn't press again: clear the binding (the in-app key is Space again), hold Space, re-pick the card's hold with the mouse → the hold is let go, and Space must be pressed again. *(Unit tests only.)*
- [ ] **It lets go when it should:** hold Space and Alt-Tab to another app → the status goes back (the window lost the focus). Hold **Hold to switch** with Enter, then Tab away → it lets go. Hold Space and close the window to the tray with the mouse → it lets go. Re-pick the card's hold, or turn the card off and on → every hold is let go, a global key's included: press it again to hold. *(Unit and e2e tests. In the app, P4 ran the focus loss with another window brought to the front rather than Alt-Tab: see below. A hide with a key held wasn't run.)*
- [ ] **N1 — A real call** (owner check — not run; the release's done-when): Settings → Hotkeys → **Push-to-modulate**: bind a global key; Mixer → **Push to modulate** on, **Hold to apply**, preset **Hollow King**. With Output on CABLE Input, in a live Discord call with someone listening: hold the key and talk → the far end hears Hollow King; let go and talk → it hears your own voice. Press and let go in the middle of words, a few times each → no click, and no syllable doubled or dropped. Just after a press, Hollow King's reverb still rings with what you said while your effects were off (they keep running while off): that is expected, not a doubled syllable. What you said before letting go keeps its effects: turning them off waits out your preset's delay first (the Mixer's latency readout; about 43 ms for Hollow King). The rest of a word you are still saying as you let go is in your own voice. One exception: up to about 20 ms of what you said just before letting go can still reach the call in your own voice. That is the buffering between your microphone and Divora's effects (a buffer or two), which the wait doesn't cover. With the effects off, your own voice still carries the preset's delay, so that a switch lines up with what you say.
- [ ] **N2 — Headphone monitor** (owner check — not run): Settings → Audio devices → **Monitor output (hear yourself)** set to your headphones, Monitor on. Hold and let go of the key while talking → the headphones follow it as the call does; a soundboard clip plays the same either way.
- [ ] **N3 — MIDI** (owner check — not run): Settings → Control surfaces: a mapping set to **Hold push-to-modulate**, learned. A pad holds while its note is down. Two pads on two mappings: it holds until both are up. A momentary CC button holds while down. A fader holds past halfway (64 or more) and lets go at 59 or less; a knob left near halfway (60 to 63) doesn't flicker. Picking the MIDI input, or pressing **Learn** on any mapping, while a pad is down → it lets go. With a fader resting high, turn the card off and on → it doesn't hold until the fader comes down and goes back up.
- [ ] **N4 — A Stream Deck** (owner check — not run): give a button a **Hotkey** action that sends the push-to-modulate key, and record what happens when you press it. Expected, not checked: a blip, since the action sends a quick press and release, so your effects switch for the tap plus up to 50 ms.
- [ ] **N5 — Shift held** (owner check — not run): in a game or app, hold the global key while Shift is down, and record whether it holds.
- [ ] **N6 — An elevated window** (owner check — not run): hold the global key over Task Manager (it runs elevated), and record whether it holds. Divora checks every 50 ms whether the key is still down, and over an elevated window that check may read it as up, so it may not hold at all.
- [ ] **N7 — Voice reading** (owner check — not run): the block "Push-to-modulate with your effects off" under Voice reading, with Loudness off and on.
- [ ] **N8 — Voice Convert** (owner check — not run): **Deep Narrator** with the bundled voice installed, on an output device with 480-frame buffers (note the buffer size, if its control panel shows it): press and let go in the middle of words → nothing doubled or missing, bar about 5 ms (Voice Convert's model delays about 5 ms past what it reports). At other buffer sizes the Denoiser delays more than it reports (928 samples at 512-frame buffers against the 480 it reports), so expect up to about 14 ms skipped or repeated at each switch there.
- [ ] **N9 — Discord's own push-to-talk** (owner check — not run): set Discord's push-to-talk to the same key as push-to-modulate, and record whether Discord still sees it.
- [ ] **N10 — The lock screen** (owner check — not run): card on, **Hold to apply**; hold Space in Divora, press Win+L, unlock, and record from the status whether the hold was let go (Clean means it was).
- [ ] **N11 — A preset switch with your effects off** (owner check — not run): **Hold to apply**, key up, talking: switch from **Clean Passthrough** to **Hollow King** in the middle of a word → a pause of about 43 ms (the delay grows by that much) between two short fades; switch back → about 43 ms skipped; nothing heard twice either way.
- [ ] Measured on the development machine, on debug builds (`pnpm tauri build --debug --no-bundle`) of `93ab993` for P6 and P5 and of `6c180bf` (the claims audit's fixes) for P4 and P3, driven over WebView2's DevTools protocol: the push-to-modulate commands sent from the page through the real IPC, except P3 (a real `WM_HOTKEY` from `SendInput`) and P4 (a key event dispatched in the page, then another window brought to the front). The engine on a silent route input "Analogue 1 + 2 (2- Focusrite USB Audio)", output "Speakers (Realtek USB Audio)", Monitor off, no monitor device. **No audio was played:** the owner declined a test tone this time, and the SMSL DAC's loopback has recorded only silence since 2026-10-24. So nothing below measures the sound itself.
  - **P6, the commands with real arguments:** `set_ptm_mode {mode:"apply"}` twice, `set_ptm_held {source:"key", held:true}` twice, `{source:"hotkey", …}`, `set_ptm_mode {mode:"apply", restore:true}`, then `{mode:"bypass"}` → as expected: `seq` +1 on each change and unchanged on each repeat, the `hotkey` source refused, the restore letting go of the key; the ticks agreed within 1 to 24 ms.
  - **P5, a reload** with the card on, **Hold to apply**: one `set_ptm_mode {apply, restore: true}`, sent at 24 ms and answered at 27 ms, before `list_audio_input_devices` (29 ms); `seq` unchanged; the titlebar, sampled every 20 ms from the reload, read Modulated from 38 to 49 ms, then Clean; main sent one `overlay:state`, clean, at 51 ms, and the overlay received it at 152 ms.
  - **P4, the window losing the focus** (Space held by a key event in the page, then a harness window brought to the front): `set_ptm_held {source:"key", held:false}` went out 8 ms later (the page's `blur` fired at 7 ms, Tauri's focus event at 8 ms), and the next tick read not held (`seq` 3). Harness check: with only the page's `blur` listener removed, Tauri's focus event still let go, at 8 ms; with that listener removed and Tauri's event blocked too, nothing let go, so the check can see a missing release. The page's `blur` on its own wasn't isolated.
  - **P3, the real hotkey** (push-to-modulate bound to Ctrl+Alt+Shift+F24, held 1 s by `SendInput` 8 times, 4 with Divora showing and 4 hidden to the tray): held within 5 to 33 ms of each key-down, on the first tick after it (ticks come about every 33 ms); let go within 12 to 76 ms of each key-up (showing 19 to 76 ms, hidden 12 to 60 ms), an upper bound on the 50 ms check plus the wait for a tick, with no audio timing in it; `seq` +2 a hold, 1 → 9 in each run; the page sent no push-to-modulate command during the holds. Divora was hidden with the titlebar's Close button (`IsWindowVisible` false). The hidden four ran in a second run: in the first, the harness's own check that Divora's page had no focus refused them, since the page read `document.hasFocus()` true just after the CDP click that hid the window; the rerun checked the foreground window instead. Hidden, the page read `visibilityState` `visible`, as in v1.54.0.
  - **B1, the release bench** (`a_long_mute_costs_at_most_twice_speech_or_a_pause`, whose voice stage now runs the push-to-modulate dry line too, settled on the effects): passes on all 24 bundled presets: the last 10 s muted cost 0.70 to 1.00 × max(speech, pause) (the bar is 2 ×), for example Zombie 277.3 µs on the tone, 293.2 µs on the pause and 241.7 µs muted per 480-frame buffer, Clean 4.3, 12.1 and 11.9 µs (`cargo test --release`, this machine).
  - **The sound, from the tests only:** the switch in a WAV, screen takes and Replay, where it lands in what you said, the dry voice lined up with the effects, and clicks rest on unit tests through the real voice stage (`the_dry_lines_up_with_the_wet_at_every_latency`, `what_you_say_while_held_stays_disguised`, `bypassed_send_is_the_dry_exactly`, `no_click_at_the_switch`, `a_growing_delay_pauses_and_repeats_nothing`, `the_chain_keeps_running_while_bypassed`) and on the stage fingerprints: one hash per bundled preset, loudness off and on (48), of what the voice stage sends with push-to-modulate off, identical on this machine to `main` before the change.
  - **Not measured:** any sound in the app — the switch on the call, in the headphones, in a WAV, a take or Replay, and clicks; the far end of a call (N1); the headphone monitor (N2); MIDI with a device (N3); a Stream Deck (N4); a key held with Shift in a game (N5); an elevated window (N6); Voice reading in the app (N7); Voice Convert (N8); Discord's own push-to-talk (N9); the lock screen (N10); sleep and UAC's secure desktop (no check yet); hiding the window to the tray with a key held (WebView2 keeps reporting it visible, so the focus loss is what lets go); a preset switch with the effects off (N11); the Mixer card, Hold to switch and the in-app key's modifiers in the app (e2e and unit tests); a relaunch with the card on; any rate but 48 kHz; the device signal after the resampler when the rates differ; Windows 10; B1 on other CPUs.

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
- [ ] **Streaming (v1.3.0):** with the bundled narrator, the converted voice has *conversational* latency — talking over a call feels responsive, not the old ~quarter-second lag. The Mixer latency readout shows ~18 ms (vs ~261 ms for a non-streaming model). No clicks/seams between chunks during continuous speech.
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
- [ ] Enable **Voice Convert** with the bundled **streaming** narrator (v1.3.0) → header shows roughly `+18 ms` (not ~261 ms), and the converted voice is usable in live conversation. A non-streaming BYO model shows ~261 ms.
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
- [ ] (v1.56.0) Talk and play clips, and **Mute** mid-recording → the file has the clips and silence where your voice was.
- [ ] Stop the engine while recording → the in-progress file is finalized and is playable (not truncated/corrupt); the Record indicator clears.
- [ ] Start a second recording → it writes a new timestamped file without disturbing the first.
- [ ] Shout into the mic (clip the input) → the recorded file saturates cleanly (no wrap-around static).

## Screen recording (v1.52.0)

- [ ] Mixer right rail shows a **Record screen** card under Record, sub-line "Your modulated voice and soundboard". With more than one screen, a picker lists them (`Screen 1 · 3440×1440 · main`, …). With the engine **stopped**, Record is disabled (tooltip "Start the engine to record").
- [ ] Start the engine, pick a screen, click **Record** → "Starting…", then "Recording · 0:01" counting up; Windows draws a yellow border around that screen. Talk in the **Villager** preset and play a soundboard sting, then **Stop** → "Finishing the file…", then "Saved divora-screen-<date>_<time>.mp4".
- [ ] The MP4 plays in Windows Media Player, VLC and a video editor (e.g. Clipchamp/DaVinci): the picture is the chosen screen (scaled to fit 2560×1440 when bigger, keeping its shape), the sound is your modulated voice plus the sting, and voice and picture line up (clap on camera or speak while typing — the sound should not trail the keystrokes).
- [ ] Settings → Recordings: switch **60 fps** and **Sharpest**, turn the **mouse pointer** off, record again → the file is 60 fps, larger, and has no pointer.
- [ ] Settings → Hotkeys → **Record screen**: bind a combination, focus a game or another app, press it → recording starts; press again → it stops and saves. **Record voice (WAV)** does the same for the WAV recorder.
- [ ] Stop the engine mid-take → the card says "Audio lost at m:ss — start the engine to bring it back"; start it again → the line goes away, and the saved file has silence for exactly that stretch and sound after it.
- [ ] Unplug the output device mid-take (Divora rebuilds the session) → the take carries on and the sound continues after the rebuild. *(Same re-attach path as Stop/Start; not yet run with a real unplug.)*
- [ ] Mid-take, change the recorded screen's resolution (Settings → Display, or a game switching to exclusive fullscreen at another size) → the take continues; at a different shape the picture is letterboxed on black rather than stretched. *(Unit-tested geometry; not yet run on hardware.)*
- [ ] Recording the screen Divora is on shows Divora's own window in the file.
- [ ] Mid-take, right-click the tray icon → **Quit** → the app closes within a second or two, and the take is a finished, playable `.mp4` (no `.part` left behind).
- [ ] **No Media Foundation:** quit, then run with `DIVORA_PRETEND_NO_MEDIA_FOUNDATION=1` set → the app starts normally, and the card says "Screen recording needs Windows' media components. Install the Media Feature Pack…" with Record disabled — never an error dialog.
- [ ] **CPU fallback:** run with `DIVORA_CAPTURE_CPU_CONVERT=1` → recording still works (frames converted on the CPU); colours match a normal take.
- [ ] Measured on the development machine (NVIDIA GPU, "NVIDIA H.264 Encoder MFT"; 3440×1440 main screen recorded at 2560×1070), soundboard click against an on-screen flash triggered in the same animation frame, engine on VB-Cable in / Realtek USB out with the output gated silent:
  - **A/V offset, 60 fps, 5 events:** the sound lands **6–19 ms before** the picture (mean −12 ms). The flash reaches the screen a compositor frame after the click is queued, so a small negative offset is expected.
  - **Engine stopped and restarted mid-take:** offsets −13 to −26 ms after the restart; `audioLostAt` reported 2.0 s.
  - **10 minutes at 30 fps, an event every minute:** 11 events from 32 ms early to 4.5 ms late (mean −14 ms; most of the spread is the 33 ms frame), a fitted trend of −7 ms (within that frame-rate noise), 18,120 frames written with one tick skipped, and a measured audio clock of −0.4 ppm — so this take barely exercised drift correction.
  - The voice path (microphone → chain → recording) was not measured end to end: the machine had no physical microphone, and a test signal on the virtual cable could have reached a live call. `timeline.rs`'s tests check the placement given the delays the engine reports, not that those match a real microphone — the talk test above is the check.

## Desktop audio in screen recordings (v1.53.0)

- [ ] Mixer → **Record screen**: a **desktop audio** picker under the screen picker. On a PC that can record everything except Divora it offers **No desktop audio** and **Everything except Divora**; the sub-line under the title follows the choice ("…plus sound from other apps").
- [ ] With **Everything except Divora**, play a game or a video and talk in the **Villager** preset; record, stop → the MP4 has the game or video *and* your voice, your voice **once**. Turn **Monitor** on (so you hear yourself on your headphones) and record again → still once.
- [ ] **A Discord call** (owner check; not run on the development machine): with a live call, record with Everything except Divora → the other participants' voices are in the file, yours once. If Discord's sound is missing, note it here — some call apps' audio may not be recordable this way.
- [ ] **Settings → Recordings → Desktop audio volume** at −30 dB → the game is quiet under your voice; at 0 dB it is as loud as it plays. **Also save separate audio files** on → beside the MP4, `<name>.voice.wav` (your voice and soundboard) and `<name>.desktop.wav` (the desktop audio before its volume); dropped into an editor at the same start as the video, both line up with it.
- [ ] Record with nothing playing for 10 s → the card says "Nothing heard from other apps for 10 s…"; play something → the line goes away.
- [ ] **Upgrade:** on a machine that used v1.52.0's recorder (`divora.screenRecording` saved without `desktopAudio`), the picker starts on **No desktop audio**; a fresh profile starts on **Everything except Divora**.
- [ ] **No process loopback** (forced): run with `DIVORA_PRETEND_NO_PROCESS_LOOPBACK=1` → the picker lists playback devices (the default marked), with "Divora can play here too" / "Divora sends your voice here — can't be recorded" where it does; the device you hear yourself on shows the doubled-voice warning before a take. Record on a device Divora doesn't play on → its sound is in the file. With "everything" saved from an earlier run → before the take the picker shows **Everything except Divora** with "Not offered now — takes leave desktop audio out", and the take records without desktop audio (the app doesn't ask for it, so no "Recording without desktop audio" line appears during the take).
- [ ] **A device that comes to carry your voice mid-take** (forced as above): with Settings → **Monitor output (hear yourself)** set to a device other than the SMSL, record the SMSL as a device while Divora's output is elsewhere; mid-take, set Divora's output to the SMSL → within a second the card says "Recording without desktop audio: Divora sends your voice to …", and the desktop stem from that point is silence; set it back → the line goes away and the device's sound returns. (With no separate monitor device, the SMSL becomes Divora's only output instead, which warns about doubling rather than leaving it out.) *(Run on the development machine with soundboard stings in place of a voice: see below.)*
- [ ] Mid-take, unplug the recorded playback device → "Desktop audio lost at m:ss — trying again"; plug it back → the line goes away and its sound returns. *(The reopen path is tested against scripted devices; not yet run with a real unplug.)*
- [ ] A **surround (5.1/7.1) headset** as the recorded device (fallback) → it records, downmixed to stereo by Windows. *(Not available on the development machine.)*
- [ ] Measured on the development machine (Windows 11 26200), with a separate stimulus process that flashes a 300 px square on the main screen and, in the same loop turn, plays a 15 ms 2 kHz burst at −80 dBFS on the SMSL USB DAC; the engine on VB-Cable in / Realtek USB out, gated silent; 60 fps; onsets found in the f32 desktop stem by matched filter:
  - **Everything except Divora, 10 events, twice:** the sound lands **0–19 ms after** the picture (means +14 and +15 ms; the clusters are one 60 fps frame apart). The stimulus itself queues the burst to leave the device 0–10 ms after it flips the square (behind up to 10 ms of silence; 6 and 10 ms on average in these two runs).
  - **The SMSL as a playback device, 10 events:** −4 to +29 ms (mean +16 ms), within 2 ms of the two Everything-except-Divora means; but the stimulus's own flash-to-burst delay averaged 3 ms in this run against 6 and 10 ms in those, so these runs can't show that the two paths agree that closely. Neither path gets a correction.
  - **Stems against the MP4:** six soundboard stings at the same sample in `voice.wav` and in the MP4's decoded audio, on both paths; the files are the same length to the sample. The MP4 has no edit list.
  - **A device that comes to carry your voice** (forced): recording the SMSL as a device, Divora's output was moved onto it mid-take (monitor on Realtek, Monitor off) and back. The SMSL's sound — the stimulus, a click a second — was left out between the switches (the clicks at 7.1 to 10.1 s missing, those up to 6.1 s and from 11.1 s kept; the switch times are known only to the second, since the status the script read refreshes once a second); three soundboard stings Divora played on the SMSL in between were absent (exactly 0); the card said "Divora sends your voice to …" during, and nothing after.
  - **Doubling,** Divora's output on the SMSL with Monitor on and a soundboard sting at gain 0.0001 (peak 8.0e-5, about −82 dBFS, in `voice.wav`): **Everything except Divora** recorded exactly 0 of it (while `voice.wav` had it); **the SMSL as a device** caught it (peak 5.7e-6, after the device's volume) and reported `desktopDoubled`; with Monitor off, exactly 0 and not reported.
  - **The quiet warning** on the Realtek USB speakers, which neither loopback can hear on this machine (a vendor virtual mixer is the likely cause): the card warned after 12 s that nothing had been heard from it (then worded "Nothing has played on Speakers (Realtek USB Audio) for 12 s."; now "Nothing heard from …", with the hint about the device's sound software), and the device was labelled as Divora's only output.
  - **Another app starting or stopping:** a −80 dBFS 1 kHz tone on the SMSL for 9 s while the stimulus opened and closed a stream on it twice: **Everything except Divora** recorded 40–43 ms of exact zeros at each open and each close (a packet trace showed Windows skipping 11–27 ms and then sending three packets of zeros, with no discontinuity flag); the SMSL's own loopback recorded the tone whole. The take counts each as a gap.
  - **30 minutes, Everything except Divora, 30 fps,** twice: before the review's fixes (thread at the highest normal priority), 30 of 31 events at +9.1 ms and one a frame off; after them (multimedia scheduler), 29 of 31 at +18.8 ms and 2 a frame earlier, +2 ms trend end to end, the stimulus's own delay a steady 10 ms. Stings against the MP4: 30 of 31 at 0 (one at −1 sample), then 31 of 31 at 0; voice, desktop and MP4 audio the same length. Both had exactly two gaps of about 20 ms, matching the stimulus starting and stopping (above): every Everything-except-Divora take in which the stimulus started and stopped once had exactly two, the one without the stimulus had none, and takes through the device's own loopback had none, stimulus or not — so not the reader falling behind.
  - **The voice, with a microphone** (Focusrite "Analogue 1 + 2", plugged in after the rest; engine on it, output gated silent on Realtek): the stimulus played audible ticks (−30 dBFS) on the SMSL's speakers, which the mic heard. **No effects:** in `voice.wav` each tick landed 89.9–91.8 ms after the same tick in the desktop stem (20 of 20, by matched filter). **One pitch effect (−5 semitones):** 92.9–101.2 ms (8 of 12 found at the shifted pitch), so the effect's own delay is taken out to within about 7 ms. The stimulus read the DAC's output time from `IAudioClock`: 13.8–19.7 ms after the desktop copy, which leaves about 74 ms plus the air (about 3 ms a metre). A separate tool reading the Focusrite outside Divora found Windows' own timestamps putting the tick 104–106 ms after the DAC. So either the Focusrite's driver stamps its audio late or the SMSL plays later than it reports; a key-press reference (Space bar sound against its key-down time) would settle it, and wasn't run. Either way, with this pair the voice lands up to about 70 ms after the picture — the A/V offset corrects it. 180,653 and 180,638 packets, none untimed, no ring drops.
  - **Not measured:** doubling with a real voice (checked with a soundboard sting); which of the microphone and the speakers holds the voice's extra 70 ms above; a live Discord call; Windows 10 (process loopback is documented for newer builds; offered only where it has been seen to work); a surround device; a real unplug mid-take; DRM-protected playback.

## Replay (v1.54.0)

- [ ] Mixer → **Record screen** card: a **Replay** row. Before it has been set up, a **Set up Replay…** button (not a switch); activating it opens the explainer (what is kept — calls included whenever the card records desktop audio — the memory, "Divora writes no picture or sound to disk until you press Save", the yellow border, the tray icon, the hardware encoder) with focus on its heading; **Not now** closes it and returns focus to the button.
- [ ] Still before **Turn Replay on** has been pressed in the explainer (opening it or pressing **Not now** doesn't count): **Settings → Hotkeys → Turn Replay on or off** reads "Works once you've turned Replay on from the Record screen card."; bind it and press it → the error toast "Turn Replay on from the Record screen card first — it explains what Replay keeps." (held until the window is shown, if Divora is hidden), and Replay stays off. It is refused the same way if Replay was set up with desktop audio off and the card records desktop audio now.
- [ ] **Turn Replay on** → within a second the titlebar shows `REPLAY · 30 s` (dimmed while it arms), Windows draws the yellow border around the screen, the tray icon gains a red dot and its tooltip says "DivoraVoice · Replay on (30 s, with desktop audio)" — or "… Replay on (30 s)" when the card's desktop audio is off — and the card's screen and desktop-audio pickers are replaced by a line naming what Replay keeps. Settings → Recordings: **Frame rate**, **Quality**, **Show the mouse pointer**, **Desktop audio volume** and (under **Advanced**) **A/V offset** are disabled with "Replay is on — turn it off to change these."; **Also save separate audio files** stays free.
- [ ] Leave the Mixer's **Push to modulate** off (the default since v1.58.0; with it on, the voice counts as disguised only while it leaves your effects on), pick **Villager** with an effect on, and stay unmuted. Wait 40 s without changing any of that, play a soundboard sting, talk, and press **Save** (or the **Save replay** hotkey, or the tray's **Save replay**) → within 3 s a toast "Saved a 30 s replay · Villager" (30 to 32: the clip's length, rounded) and `recordings/divora-replay-<date>_<time>.mp4` of 30 to about 32 s, starting on a clean frame, with the sting; it plays in Windows Media Player, VLC and an editor.
- [ ] If, at any point in the clip's length plus 3 s before Save, the voice was muted, had no effect on besides the noise gate, or had Push to modulate taking your effects off (its key up in **Hold to apply**, or held in **Hold to bypass**), or the preset changed → the toast says "your voice as your call heard it" instead of a preset. (Since v1.56.0 a muted stretch of the clip has no voice at all, and since v1.58.0 a stretch with your effects off has your own voice; the toast's words are unchanged.)
- [ ] Hide Divora to the tray (close the window), save from the hotkey → the tray tooltip says "… · saved 30 s at hh:mm" while hidden; reopen the window → the toast shows then, for its full time.
- [ ] **Record** while Replay is on → the take records on Replay's settings (the locked rows above) and **Also save separate audio files** as set; its file is whole and Replay keeps running.
- [ ] **Stop the engine** while Replay is on → the chip says `REPLAY · HELD`; Save still saves what it kept, ending at the stop (with the soundboard that was playing then). Pick another preset before that Save → the toast names no preset: a held clip is judged up to the hold. Start the engine → it keeps again from empty.
- [ ] Switch the output device in Settings while Replay is on → Replay is **not** held (the engine restarts without a Stop).
- [ ] Lengths over this PC's memory ceiling are disabled with "Needs up to … MB; this PC allows … MB."
- [ ] Turn Replay off from the card, the tray or the hotkey → the chip, the border, the red dot and the tray items go; the hotkey turns it on again (with the engine running). Restart Divora → Replay is off.
- [ ] **Quit** from the tray right after **Save replay** → the clip is saved before Divora exits.
- [ ] Press **Record** and then **Stop** at once, with Replay off → the take stops (a short file, or "Stopped before the screen sent a picture."), and **Record** works again straight after. Turn Replay on, then off as soon as the switch shows on → it ends off: no chip, no border, no red dot. (A second press before the switch flips is another "on", and Replay stays on.)
- [ ] Change the length while Replay is **held** (engine stopped) → the new length shows as selected (on the card and in Settings) and is saved, but the ring keeps its old length: the card still says "Keeps the last 30 s · uses … of memory" and the chip stays `REPLAY · HELD`. Start the engine → the ring keeps the new length (the chip says `REPLAY · 60 s`).
- [ ] **Near-full disk:** run with `DIVORA_PRETEND_FREE_BYTES=1000000` → Save says "There isn't enough free space on the drive to save the replay (it needs about … MB)." and nothing is written, not even a `.part`.
- [ ] **No direct encoder:** run with `DIVORA_PRETEND_NO_DIRECT=1` → a take still records (through the sink writer, desktop audio and all); the Replay row then says "Replay can't use this PC's video encoder." and the switch is disabled.
- [ ] **Sleep** with Replay on and a take running (owner check — not run: v1.54.0 ships with its sleep handling checked by tests only, never with a real sleep): the take should end where the PC went to sleep, its file whole and with no silence for the time asleep, the card saying "The PC went to sleep, so the recording stopped at m:ss. What was recorded is saved."; a Save pressed just before sleeping should still save its clip. After waking and signing in, Replay should keep again from empty: until the screen sends a picture, however long the sign-in takes, the chip shows `REPLAY · 30 s` dimmed (it never says Arming), the card's button reads "Save (0:00 so far)", and Save answers "Replay hasn't kept anything yet — try again in a moment." If the screen capture and encoder can't be set up again within about 30 s of waking, Replay should turn off: the chip and the tray's red dot go, with no message on screen (the reason goes only to Divora's log).
- [ ] **Unplug the recorded monitor** with Replay on (owner check — not run): the chip says `REPLAY · HELD`, Save still saves, and a running take ends with "That screen was disconnected, so the recording stopped at m:ss. What was recorded is saved."
- [ ] Measured on the development machine, on the final build — a debug build of the shipping code before its last change (a fallback that only a failed encoder build reaches) — unless a bullet names another (RTX 4090, "NVIDIA H.264 Encoder MFT"; the 3440×1440 main screen recorded at 2560×1070; the engine on VB-Cable in / Realtek USB out, gated silent; the debug app driven over WebView2's DevTools protocol, by commands from the page unless a bullet says a click; a separate stimulus process flashing a 300 px square on the main screen and, in the same loop turn, queuing a 15 ms 2 kHz tick at −40 dBFS on the SMSL DAC behind the 0 or 10 ms of silence already in its buffer, so every offset below includes that delay; Replay at 60 fps with Everything except Divora unless a bullet says otherwise):
  - **Arming:** 248 ms from the arm to its answer; by the app's log, the ring held its first keyframe about 0.26 s after the arm.
  - **A clip after 35 s** (ticks found in the MP4's own AAC track by matched filter): the tick lands **+18.0 ms** after the flash on 9 of 10 events and +1.3 ms on the other, a 60 fps frame earlier (mean +16.3 ms). Saved in 143 ms: 30.05 s, 1803 frames. `bufferedMs` 31.2 s before the save; 34.7 MB held against a 115 MB cap.
  - **A take recorded while Replay was on** (desktop stem): **+14.7 ms** on 7 of 8 events and +31.4 ms on the other, a frame later (mean +16.8 ms), with a trend of −1.4 ms over 10 s. A take asked for at 30 fps by command while Replay ran at 60 was refused with "Replay is on with a different frame rate. …"; the app's own **Record** can't ask for that, since it sends Replay's settings.
  - **The direct path against v1.53.0's sink writer,** on the day-4 gate build (before takes moved onto the shared pipeline), one take each in the same session, with a −30 dBFS click against a white flash: +9.6 to +26.3 ms (mean +19.6 ms, 1107 frames, none dropped) against −19.6 to +30.4 ms (mean +12.1 ms, 6 of 1100 dropped). The means differ by 7.5 ms, under half a frame: no regression shown, none ruled out below a frame. A take with Replay off was not A/V-measured on the final build.
  - **The card** (its own settings: 30 fps, no desktop audio): turned on from its explainer, and its **Save** clicked in the page (not with the OS mouse) → one `replay-saved`, the file written, the toast "Saved a 3 s replay · your voice as your call heard it".
  - **Whole files:** the seven files the run kept — the clip, a save during a take, a held ring's save, the take, a take after Replay was turned off (30 fps, on a fresh pipeline), the card's save and the fallback take — each have a keyframe table matching their IDR frames, start on one, and have no edit list, no video composition offsets and a BT.709 limited-range tag. Media Foundation decoded every video and audio sample (no gaps over 1 ms, no errors), and ffmpeg decoded each without an error.
  - **Every save attempt answered once** (60 fps, no desktop audio): Replay off, arming, two saves at once (one saved, one "Still saving the last replay.") and a save pressed just before Replay was turned off (saved) — five attempts, five `replay-saved` events; `saving` false afterwards.
  - **A take started right after another, and a ring armed right after one was turned off** (30 fps, no desktop audio), heard every soundboard sting played into them (3 of 3 each). On the build before the tap was fixed, both heard none — see the Architecture notes.
  - **Starting and stopping at once:** a length clicked on the card while held (60 s; 30 fps, no desktop audio) left the held ring at 30 s and applied as the engine came back. Replay turned off 0, 30, 200 and 600 ms after its arm command stayed off, and a take on other settings (30 fps, no desktop audio) then recorded on its own. From the card or the hotkey an Off can only follow once the switch shows on, no sooner than the arm's answer (248 ms above). Stop sent 0, 40, 250 and 900 ms into a take's start (30 fps, no desktop audio) ended it (the start and the Stop both answered within 0.35 to 1.01 s of the start), and the next take recorded. On an earlier build, a 0 ms Stop once reached the recorder before the take had begun: it was refused with "No screen recording is running." and the take ran on until the next Stop (that run's two failed checks). Two commands sent back to back can arrive in either order; no one presses that fast.
  - **20 × (turn on, 5 s, turn off with frames in flight):** no crash, no display-driver event; private memory, read 2.5 s after each turn-off, 102.0 MiB after the first and 104.2 MiB after the twentieth (49 to 130 MiB in between).
  - **Fallback** (`DIVORA_PRETEND_NO_DIRECT=1`): the take recorded through the sink writer with its desktop audio and ended cleanly; Replay's status then said, and an arm was refused with, "Replay can't use this PC's video encoder." Its A/V was not measured: other audio playing on the PC masked the −40 dBFS ticks.
  - **Near-full disk** (`DIVORA_PRETEND_FREE_BYTES=1000000`): refused with "There isn't enough free space on the drive to save the replay (it needs about 7 MB).", the toast said it, nothing was written (no `.part`), Replay still on.
  - **Hidden to the tray** (saves by command; the tray menu and the global hotkeys weren't pressed): the tray button's accessible name read "Replay on (30 s, with desktop audio)", and after a save while hidden "… · saved 12 s". Windows forms that name as "DivoraVoice " plus only the tooltip's first 64 characters, so "at hh:mm" was cut off there; the app sets the whole 71–72-character tooltip, and the tooltip shown on hover was not read. The toast waited and showed when the window came back. WebView2 still reports `visibilityState: "visible"` for a window hidden to the tray, so the toast also asks the window whether it is visible.
  - **Armed for 26 minutes,** on the build before the review's fixes (2026-10-16): the planned 2 hours ended there, when Divora closed with nothing in its log and no Windows crash record — not established whether it was closed or crashed — and the owner chose to ship on the 26 minutes. Replay stayed on throughout, holding 30.1 to 32.0 s and at most 34.7 MB against its 115 MB cap. The process's private memory (sampled once a minute) was 178 MiB at minute 1 and 187 MiB at minute 5, then between 184 and 195 MiB (195 at minute 13), and 185 MiB at minute 26: no net growth after minute 5. One log line in the 26 minutes (the save at 5 minutes, a 31.25 s clip). The 5-minute clip's A/V was not analysed, and nothing past 26 minutes was measured.
  - **Not measured:** two hours armed (26 minutes were, on the build before the review's fixes); the A/V after the first minute armed (the clip saved at 5 minutes wasn't analysed); the fallback's A/V on the final build; the voice's and soundboard's A/V (the stings were counted, not timed); a real sleep and resume; a monitor unplugged; an encoder failure (`dxcap -forcetdr`); switching the output device; the tray icon's red dot and its menu; saves from a global hotkey; Quit during a save; GPU load and a game's frame times with Replay on; Narrator, and what it reads in the explainer. Sleep, a monitor unplug, the device switch, the tray, the hotkeys and Quit have owner checks above; an encoder failure, GPU load, Narrator and the long run have none.

## Charms (v1.55.0)

- [ ] Settings → **Stream overlay**, under **Background**: a **Move and resize** row with an **Arrange** button (disabled while the overlay is off; "Starting…" while it opens), **Fit to monitor**, **Hide from recordings** (only where Windows offers it, Windows 10 version 2004 and later), and a **Charms** list that starts as "Spell circle" with a **Remove** button, then **Add picture…**. Settings → Hotkeys has **Arrange or lock the overlay** ("Switches the overlay between Arrange and Locked while it's open."), unbound.
- [ ] **Upgrade:** on a profile from v1.54.0 or earlier (no `divora.overlayScene`), turn the overlay on → the circle shows where it did before (centred, at most 440 px); in a window smaller than that, it shrinks to fit instead of being cut off.
- [ ] **Add picture…** → a picker titled "Pick a picture or GIF for the overlay" (PNG, JPEG as `.jpg`/`.jpeg`/`.jfif`/`.jpe`, GIF, WebP, APNG; the file's content decides its type, not its name). Pick an animated GIF → the toast "Picture added. Use Arrange to place it."; the list gains "<name> · <W>×<H> · animated" above "Spell circle" (front to back); the overlay shows it towards the bottom right, its long side 30 % of the window's short side or its own long side, whichever is less, but its short side at least 24 px where the window allows (so a tiny or very thin picture is drawn larger than its pixels). The file is copied into `overlay\assets`, named by its content. Add the same file again → a second row, "<name> (2)", and still one file.
- [ ] Refused, each with an error toast and nothing added: over 50 MB → "That picture is over 50 MB. Pick a smaller one."; a text file renamed `.png` → "Divora can add PNG, JPEG, GIF and WebP pictures. That file isn't one."; over 8192 px a side or 16 megapixels → "That picture is too big (over 16 megapixels or 8192 pixels on a side). Make it smaller and try again."; animated and over 4 megapixels → "That animated picture is too big (over 4 megapixels). Make it smaller and try again."; past what the whole overlay may hold (200 MB of files, 64 megapixels, 16 of them animated) → "That's more than the overlay can hold. Remove a picture or pick a smaller one." A file over 20 MB, or animated and over 2 megapixels, is added with "Picture added. It's a large picture, so the overlay may use more memory and CPU." At 16 pictures **Add picture…** is disabled and the list says "The overlay holds up to 16 pictures."
- [ ] **Arrange** (Settings, or the hotkey) → a dashed frame, **Lock** at the top right, **Move window** at the top left and the hint "Esc locks the overlay" ("Click Lock to lock the overlay" when the overlay doesn't have the keyboard); under 320 px wide or 120 px tall the hint goes, under 320 px wide **Move window** shows only its icon, and under 160 px wide **Lock** does too. Settings' button reads **Lock**, and the tray menu gains **Lock the overlay**. Drag a picture or the circle → it moves; drag a corner handle → it resizes and keeps its shape. With one selected: the arrows nudge it 1 px (Shift: 10 px); `=` and `-` (and the numpad's + and −) resize it by 2 px on its long side, `+` and `_` by 10 % (on a US keyboard, Shift with `=` and `-`); `[` and `]` send it back or bring it forward one, `{` and `}` to the back or front; **Delete** removes it (Backspace doesn't); **Esc** during a drag cancels the drag. Restart → everything is where you left it.
- [ ] Settings → Stream overlay → Charms: **Remove** on "Spell circle" → the circle goes and **Add the spell circle** appears; press it → the circle is back, centred. **Remove** a picture → it leaves the overlay, and its file leaves `overlay\assets` once no charm uses it (with two copies of one file, removing one keeps it).
- [ ] The hotkey with the overlay off → the toast "The overlay is off. Turn it on in Settings first."; pressed as it opens → "The overlay is still starting. Try again in a moment."
- [ ] **Click-through** (owner check over a game and OBS — not run there): Locked, over a borderless game and over OBS, clicks and the scroll wheel reach the app under the circle, under a picture and at the window's edges (the resize border too). In **Arrange**, clicks are caught, from the first beat a moment after the chrome shows. *(Only hit-testing was checked, with a window probe on the development machine and no picture: `WindowFromPoint` at the window's centre, four edge and corner points and one inner point, Locked and in Arrange: see below. Clicks, the scroll wheel and a picture weren't checked.)*
- [ ] **Keyboard hand-back** (owner check — not run: needs a game and real keys): bind **Arrange or lock the overlay**, play a borderless game, press it → Arrange; press **Esc** → the next key reaches the game, no click needed. The same with the chrome's **Lock**, and with the hotkey again. From Settings: **Arrange**, then **Esc** in the overlay → Settings has the keyboard again. Arrange, click back into the game, then lock with the hotkey → the game keeps the keyboard. The tray's **Lock the overlay** leaves the keyboard where it is. Note any time Windows refused to hand it back (the next key went nowhere).
- [ ] **Focus without injected input** (owner check — not run): in a borderless game, press the hotkey about 20 times while moving the mouse. Divora's log (`%APPDATA%\com.divora.voicemod\logs\`) has an `overlay focus ok=true` or `ok=false` line for each Arrange: note how many succeeded, for that run only. A key logger (or the game's keybind display) shows no Alt after the hotkey.
- [ ] **Exits:** each of these locks the overlay at once (the chrome goes, clicks pass through): **Esc**, the chrome's **Lock**, Settings' **Lock**, the tray's **Lock the overlay** (in the menu only while arranging; with Replay on, **Save replay** and **Turn Replay off** stay), and the hotkey (from inside a game too). Run them with the overlay unfitted **and** fitted to the monitor Divora's window is on. Fitted there, Settings (and maybe the tray) sit under the overlay, so Esc, the chrome's Lock and the hotkey are the ways out. *(Esc, the chrome's Lock and Settings' Lock were run unfitted over DevTools, and each locked it. After Esc the click-through style came back and the chrome went; for the two Locks only the mode was read. No click was sent through after any of them: see below. The tray, the hotkey and the fitted case are owner checks.)*
- [ ] **The safety lock** (debug build): launch it with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` set, open `edge://inspect` in Edge and inspect `overlay.html`. Fit the overlay to the monitor Divora's window is on, **Arrange**, then run `for(;;){}` in the overlay's console → within about 3 s clicks pass through again, the toast "The overlay stopped responding, so Divora locked it. Turn it off and on again if it looks wrong." shows, and Settings works. Repeat with the DevTools protocol's `Page.crash`, and note what WebView2's error page looks like while Locked. *(The hang was run over DevTools: see below. `Page.crash` and the fitted case not yet run.)*
- [ ] **Keys** (owner check — not run: the run below dispatched keys inside the page, never real ones): in Arrange, and Locked too if the overlay kept the keyboard after **Esc** (normally Esc hands the keyboard back to the window that had it before Arrange, and these keys would reach that window), F5, Ctrl+R, Ctrl+P, Ctrl+F, F12, Ctrl+Plus, Alt+Space and Alt+Left do nothing: no reload (in Arrange one would lock it again), no print dialog, find bar, zoom, DevTools or system menu. **Alt+F4** on an arranging overlay closes it, the Settings toggle turns off, and a window probe finds no overlay window.
- [ ] **Watchdog** (debug build only): quit, then run with `DIVORA_PRETEND_OVERLAY_NO_READY=1` set and turn the overlay on → nothing shows; after 10 s the window is destroyed, the toast "The overlay window didn't start, so Divora closed it. Try turning it on again." shows once, and the toggle turns off. With Hide from recordings on during a take, nothing shows in those 10 s either.
- [ ] **GIFs:** an animated GIF animates Locked and in Arrange, also with Settings → Appearance → **Motion** on **Functional**. An APNG and an animated WebP animate too. A 12-bit or arithmetic-coded JPEG is refused with "Divora can't show that kind of JPEG. Save it as a regular JPEG or a PNG and try again." *(Run over DevTools, except Motion on Functional and the JPEG, which is unit-tested only: see below.)*
- [ ] **Memory:** fill the overlay up to its pixel budgets (animated pictures up to 16 megapixels in all, then still ones until all the pictures together reach 64 megapixels), open it over a game, and note the private bytes of `divora-app` and of every `msedgewebview2` process (browser, renderer, GPU) at open, at the peak and after 5 minutes, and the renderer's and GPU process's CPU. Then close it 20 times while it loads and note `divora-app`'s private bytes. Set the limits for a pass beforehand; if one fails, the budget constants are lowered. *(Run without a game, for 2 minutes drawn small and 30 s drawn large, and the CPU not read: see below.)*
- [ ] **Alpha in OBS** (owner check — not run): a Window Capture of "DivoraVoice Overlay" with a method that supports transparency shows a GIF's soft edges over the scene with no box. **Green** and **Magenta** key out with a Chroma Key filter, and the Arrange chrome shows while arranging.
- [ ] **Restart:** with pictures arranged and **Fit to monitor** and **Hide from recordings** on, quit from the tray with the overlay open, relaunch → the overlay is off; turn it on → it opens Locked, fitted, with the arrangement and both settings kept. With no pictures: **Arrange**, resize the window by its edges, **Lock**, restart → the circle is the same size. Remove a picture, restart → it's gone, and so is its file unless another charm uses it. *(The first part was run over DevTools with one picture, stopping the process instead of quitting from the tray: see below. The circle's size after an edge resize, and a removal across a restart, weren't run.)*
- [ ] **Fit to monitor**, Locked and in Arrange: turning it on moves nothing on screen; turning it on or off while the overlay is off, then opening, moves nothing either. Fitted, a picture drags to a far corner of the monitor, and the **Move window** grip is gone; turning Fit off pulls the picture into the window. With Fit on, **Green** and **Magenta** are disabled, with the tooltip "Turn off Fit to monitor to use a colour background."; with a colour background, Fit is disabled with "Fit to monitor needs the Transparent background."
- [ ] **Fitted** (owner check — not run: no display settings were changed on the development machine): the taskbar still auto-hides and notifications still show. Change the display scale 100 % → 150 % → 100 % → within about half a second the window equals the monitor again, the Arrange frame sits on its edges, and nothing catches clicks on the next monitor. Change the resolution → note what happens (a check every 5 s should fit it again).
- [ ] **Hide from recordings, checked by decoding.** Count clearly green pixels inside the overlay's rectangle in every decoded frame, frame 0 included (the background's `#00b140` decodes muted, so count G−R > 50 and G−B > 50; `ffmpeg -i take.mp4 -vf crop=… -pix_fmt rgb24 -f rawvideo out.rgb`):
  1. Background **Green**, Hide on, Locked: a 10 s take → no frame has green. Hide off → every frame does.
  2. Turn Replay on and save within 5 s, so the clip holds the first frames after arming → no green, from frame 0.
  3. **Arrange** mid-take → green from the first beat on, with the hint "Recordings can see the overlay while you arrange."; none once locked again.
  4. (Owner check — not run.) With Hide on, Locked and a take running, note what OBS shows of the overlay (black, nothing, or a frozen last frame) for Window Capture with each method (BitBlt, and Windows 10 1903 and up) and for Display Capture. Again with Replay on and no take. After the take ends or Replay is turned off, note whether each source comes back by itself or needs a refresh. The help text and the CHANGELOG's OBS sentence follow these results; if any method freezes, they say so ("OBS may show a still picture of it").
  5. (Owner check — not run.) After a take ends with Replay off, OBS Window Capture shows the overlay again; with Replay on, it stays hidden until Replay is turned off.
  6. (Owner check — not run.) Win+D, then Win+M, then restore, all during a protected take; decode the take. Green after the restore means protection was lost there (the fix would re-apply it on the window's `Resized` event).
  7. With Replay and Hide on, turn the overlay on and off 10 times: no blank rectangle or frame while it starts, the circle and pictures draw normally once shown, and a save covering the opens has no green, the first frames after each open included. Again with Hide off, watching for an empty rectangle while it starts.
  8. Also: open the overlay while Replay is on; lock it mid-take; switch Fit during a protected capture.
  9. Read the overlay's display affinity with a window probe (`GetWindowDisplayAffinity`): 0x11 while it is protected, 0x0 otherwise. Divora logs it only at debug level, so its log file doesn't have it.
- [ ] **Windows 10** (owner check on the other machine — not available on the development machine, which runs 26200): install a CI-built installer and decode as above in two orders, each noted as hidden / visible / black: (a) Green, Hide on, Locked; open the overlay, wait at least 30 s, then turn Replay on and save; separately, a 10 s take. (b) The other order: start the take or turn Replay on first, then open the overlay. The log line `overlay shell ready hide_available=…` says whether the toggle is offered. These decide whether the toggle stays on Windows 10 2004 and later (contract 10) or needs Windows 11 22H2 (contract 15); until they are run, the result is "Not measured".
- [ ] **No capture exclusion** (forced): run with `DIVORA_PRETEND_NO_CAPTURE_EXCLUSION=1` → the **Hide from recordings** row is gone, and the log says `overlay shell ready hide_available=false`.
- [ ] **GPU and PresentMon**, on the 3440×1440 main screen (4K only if a 4K display is attached): idle GPU (the 3D engine's use by `dwm.exe` and Divora's processes), and PresentMon of a borderless game (present mode, frame-time p50/p99) for the overlay off, 480 with the circle, 480 with a GIF, fitted with the circle and fitted with a GIF, 60 s each. **Gate:** if fitting adds more than 2 percentage points of idle GPU over 480, or changes the game's present mode (e.g. "Hardware: Independent Flip" → "Composed: Flip") where 480 didn't, ship the bounding-box fallback, then measure Locked again at the box's size. *(Idle GPU measured, 15 s each rather than 60: see below. PresentMon is an owner check — not run, no game.)*
- [ ] **Mixed DPI** (owner check — not run: both monitors on the development machine are at 100 %): move the overlay between a 100 % and a 150 % display → the arrangement keeps its sizes.
- [ ] **Focus:** turning the overlay on never takes focus (as in v1.51.3).
- [ ] **Permissions** (debug build): the overlay's DevTools console shows no "not allowed" error (a refused call shows up as `overlay: overlay_handshake(…) failed`, `overlay: set_overlay_mode(locked) failed` or `overlay: overlay_alive failed`).
- [ ] Measured on the development machine (Windows 11 Pro 26200; a 3440×1440 main screen and a 1920×1080 second screen to its left, both at 100 %; NVIDIA GPU, "NVIDIA H.264 Encoder MFT"), on two debug builds of `charms` (`pnpm tauri build --debug --no-bundle`), driven over WebView2's DevTools protocol — Settings clicks and the overlay's keys and mouse were dispatched in the page, never real input — with a read-only window probe (styles, window properties, display affinity, `WindowFromPoint`, the foreground window); the engine silent (input CABLE Output, output Realtek, monitor off). The first build, from `34f1b0b` (before the toggle fix below and the review's fixes), ran Opening, Arrange from Settings, Fit, Permissions, the GIF, the safety lock, the watchdog, Hide from recordings decoded, the first opened-during-a-capture run, the 12 quick cycles and Focus's console-window case; the second, with those fixes (`1c1e792` and `5d5786e`), ran the rest:
  - **Opening:** built hidden, click-through set before it was first seen visible, visible about 138 ms after the click, Locked; the foreground window didn't change. Locked, `WindowFromPoint` at the centre, four edge and corner points and one inner point never found the overlay (6 of 6 went to the window under it); its extended style was 0xC0138 (transparent, layered, topmost) with `NonRudeHWND` set.
  - **Arrange from Settings:** click-through off about 60 ms later, after the first beat; `WindowFromPoint` then found the overlay at all 6 points. **Esc** locked it in 64 ms and the chrome's **Lock** in 81 ms; Settings' button (named "Lock the overlay" while arranging) locked it too. After Esc, click-through came back and the chrome went; for the two Locks only the mode was checked.
  - **Fit:** on → the window was its monitor exactly (0,0 3440×1440), still click-through with `NonRudeHWND`, `WindowFromPoint` going through it, its background transparent; off → back to 480×480 at the same place.
  - **Permissions:** no "not allowed" message in either window's console; the overlay's handshake, Lock and beat calls all worked. Closing from the toggle removed the window.
  - **A GIF, through the real Add picture…** (the native dialog answered inside the page): a 320×180 animated GIF → "Picture added. Use Arrange to place it."; stored as a 16-hex-digit `.gif` id, 320×180, animated; drawn from `http://charm.localhost/<id>` at 144×81 (aspect kept), not draggable; served 200 with `content-type: image/gif`, `cache-control: public, max-age=31536000, immutable` and `x-content-type-options: nosniff`. `..%2Fmain`, an unknown id, `x.svg` and an uppercase id were refused (404); the same URL from the main window was refused (403). It animated Locked and in Arrange (two screenshots 350 ms apart differ). With a 200×100 PNG added, Settings listed "charm-test.png · 200×100", "charm-test.gif · 320×180 · animated" and "Spell circle", each **Remove** named for its row. A 10-step drag in Arrange moved the top charm by exactly the pointer's travel, and main saved it. **Remove** deleted each file from `overlay\assets` once nothing used it.
  - **Formats:** an APNG and an animated WebP (240×120) animated at their stored shape. A 300×200 JPEG with EXIF orientation 6 was stored as 200×300 and drawn upright (WebView2 read it as 200×300; drawn 96×144).
  - **The safety lock:** in Arrange, the overlay's renderer hung with `for(;;){}` → locked again 2998 ms after the hang began (three missed 1 s checks), click-through back, `WindowFromPoint` going through the hung window, the toast "The overlay stopped responding, so Divora locked it. Turn it off and on again if it looks wrong.", Settings still working. The hung overlay closed in 471 ms.
  - **Watchdog** (`DIVORA_PRETEND_OVERLAY_NO_READY=1`): never visible; closed at 10.2 s; exactly one toast, "The overlay window didn't start, so Divora closed it. Try turning it on again."; the toggle off; the window gone.
  - **Hide from recordings, decoded** (Green, Hide on, Locked, the window 480×360 at 200,200 on the main screen; 30 fps takes of the 3440×1440 screen encoded at 2560×1070; green counted in the window's rectangle inset 4 px), frames with green: a 10 s take with Hide on, 0 of 302; Hide off (a 5 s control), 152 of 152, green over 0.886–0.892 of the rectangle (the circle covers the rest); **Arrange** mid-take (3 s locked, about 3 s arranging, locked again), 92 of 276, at 0.77 only while arranging, none before or after; Replay armed and saved 4.3 s later, 0 of 130, frame 0 included. The display affinity read 0x11 (`WDA_EXCLUDEFROMCAPTURE`) during the take, while Replay was armed and after locking again mid-take, and 0x0 while nothing captured, while arranging, with Hide off, after the take ended (Replay off) and after Replay was turned off.
  - **Opened during a capture** (Replay armed, Hide on, Green, 6 opens and closes): every open read 0x11 and landed on the same rectangle. On the build before the toggle fix, the Replay clip held 641 frames, none with green, frame 0 included. After it, all 6 opens were protected, but Replay went held 2.5 s after arming (see "Seen once" below), so its clip held 14 frames, none with green.
  - **Quick on and off:** 20 cycles from the Settings toggle (on, wait until Arrange is possible, 1.2 s, off, 0.4 s): 20 of 20 opened, and the toggle was off after every close; on to Arrange-possible took 141–205 ms. Before the toggle fix (`1c1e792`), 6 of 12 cycles left the toggle on after a close: the close's answer landed about 10 ms after the "closed" status and was applied last. Main now applies a command's answer only if no status arrived while the call was out.
  - **Restart:** a picture arranged, Hide and Fit on, the process stopped with the overlay open, then relaunched: the overlay was off; `divora.overlayScene` and `divora.overlay` unchanged; Fit and Hide read back on; turning it on gave Locked and fitted, with the picture's rectangle on screen identical (0 px). A main-window reload adopted the open overlay (the toggle on, Arrange working). Removing the picture deleted its file.
  - **Focus:** **Arrange** from Settings, over DevTools, while the owner's Chrome had the foreground: Windows refused the overlay the foreground 3 of 3 times (Divora wasn't the foreground process), and nothing else happened — Chrome kept the keyboard through Esc, the chrome's Lock and Settings' Lock. With the console window a debug build opens for Divora in the foreground (Windows Terminal, titled with `divora-app.exe`'s path; a release build has no console), Arrange did give the overlay the foreground.
  - **Idle GPU** (the 3D engine's use over 15 s, `dwm.exe` plus Divora's processes, silent engine; percent): off 0.17; 480×480 with the circle 1.70, with the circle and a GIF 1.79; fitted to 3440×1440 with the circle and a GIF 1.89, with the circle 1.68. Fitting adds about 0.1 percentage point over 480, so the 2-point gate passes on idle GPU.
  - **Memory**, against limits set before measuring (`divora-app` at most +150 MB at the peak, WebView2 at most +1536 MB at the peak, `divora-app` within 50 MB after 20 closes during a load), with a scene filling both pixel budgets (4 animated 2048×2048 GIFs of 20 frames, the whole animated budget, and 3 PNGs of 4096×4096, filling the total; GIFs with more frames, and extra copies of a picture, which the budget counts once, weren't tried): drawn small (a 480 window on the second screen, each picture about 144 px), `divora-app` 13 → 14 MB, WebView2's private bytes 336 → at most 321 MB (flat for 2 minutes), dedicated GPU memory 101 → 28–62 MB. Drawn large (fitted to the second screen, each picture 1080×1080, 30 s): `divora-app` 13 → 21 MB, WebView2 334 → 880 MB (GPU process +459, renderer +93), GPU memory 101 → 400 MB, flat over the 30 s. 20 on/off cycles closing 300 ms after opening: `divora-app` 13 → 14 MB, WebView2 290 → 266 MB. All three limits pass; the budgets are unchanged.
  - **Seen once, not reproduced:** Replay held with "the video encoder stopped returning frames" (v1.54.0's 1 s liveness check) about 1 s after an overlay open, 2.5 s after arming, in the second opened-during-a-capture run. Not seen in 26 further rounds (156 opens and closes under armed Replay: Hide on with Transparent ×4, with Green ×8, with Green and a placement ×10, and 4 fresh launches with the first arm followed by an overlay open): 1 in 27 rounds, cause unknown. Replay behaved as designed: held, and the save kept the frames from before the hold.
  - **Not measured:** the upgrade from a v1.54.0 profile; the Arrange button's "Starting…" and the Hotkeys row's **Arrange or lock the overlay**; the picker's title and file types (the dialog was answered inside the page); the refusals, the large-picture warning's toast and the 16-picture cap; a second copy of one file; the Arrange frame's hints ("Esc locks the overlay" or "Click Lock to lock the overlay", and "Recordings can see the overlay while you arrange." during a take with Hide on) and its buttons shown as icons only in a small window; a corner-handle resize, Esc during a drag, and the keys that nudge, resize, restack and delete a charm; removing the spell circle and adding it back; real keys in a focused overlay (F5, Ctrl+R, Ctrl+P, Ctrl+F, F12, Ctrl+Plus, Alt+Space and Alt+Left doing nothing; Alt+F4 closing it); the hotkey, anywhere (none was pressed): its two toasts, and about 20 presses in a borderless game (how often the overlay got the foreground, no Alt injected); the keyboard going back to the window that had it (a game, or Settings) after Esc, the chrome's Lock or the hotkey; the tray's **Lock the overlay**; the exits and the safety lock while fitted to the main window's monitor; clicks and the scroll wheel through a locked overlay, and hit-testing over a picture (only `WindowFromPoint` was read, with no picture on the overlay); moving the window with **Move window** or an empty spot, and resizing it by its edges; quitting from the tray (the restart run stopped the process), the circle's size after an edge resize across a restart, and a removal across a restart; the watchdog with Hide from recordings on during a take; a picture dragged to a far corner while fitted, the **Move window** grip gone, and Fit off pulling the picture back in; Green and Magenta disabled while fitted, with their tooltip, and Fit disabled with a colour background (unit-tested only); `Page.crash`; Win+D and Win+M during a protected take; Fit switched during a protected capture; Hide step 7 as written (a save covering 6 opens was decoded, not 10, and only green and the affinity were checked, not a blank rectangle while it starts or how the circle and pictures drew) and its pass with Hide off; the forced no-capture-exclusion run; OBS (Window Capture's alpha edges, Green and Magenta keyed out, what each capture method shows of a protected overlay and whether it comes back by itself); Discord Go Live, Teams and Snipping Tool while protected (the copy names Discord, but nothing here checked it); the taskbar's auto-hide and notifications while fitted (`NonRudeHWND` is set, but its effect wasn't observed); a display scale change 100 → 150 → 100 % and a resolution change while fitted; mixed DPI; pictures holding still as Fit turns on or off (only the window's rectangle was checked); a GIF with Motion on Functional; PresentMon's present mode and frame times of a borderless game; memory after 5 minutes (drawn small ran 2 minutes, drawn large 30 s), and over a game; the renderer's and GPU process's CPU; Windows 10 and Windows 11 21H2 (the owner's two-order check on the other machine; until it is run, whether the toggle needs contract 10 or 15 is pending, and the CHANGELOG says so); a 12-bit or arithmetic-coded JPEG refused in the app, and a GIF whose first frame is larger than its logical screen, or whose logical screen is 0×0, served to WebView2 (the JPEG's refusal and the GIF's grown canvas are unit-tested only); a WebP whose frames are larger than its canvas (not tested); a 4K display; a long Replay with the toggle on. Discord, Teams and Snipping Tool, Windows 11 21H2, those GIFs and that WebP, and the long Replay have no row; moving the window has one in the Stream overlay section below, and the rest have one above.

## Mute (v1.56.0)

- [ ] Mixer → the voice status card (Clean / Modulated / Muted) has a **Mute** button. Click it → the button reads **Unmute**, the card says "Your voice is off · clips and Speak still play", and the titlebar shows **Muted** in red (its tooltip: "Your voice is muted for the call, your headphones and recordings. Clips and Speak still play."). Press Space (the in-app push-to-modulate key) → still muted: a mouse click hands the focus back to the page. Tab to the button and press Space or Enter → it mutes or unmutes and keeps the focus. *(Clicks were run over DevTools: see below. Space after a click and the keyboard weren't.)*
- [ ] Muted, stop the engine → the titlebar still says **Muted**; start it → still muted. Mute or unmute while it's stopped → kept for the next start. Quit from the tray while muted and relaunch → unmuted: a mute is never saved. Settings → Diagnostics → **Test my setup** while muted, with no clip playing → **Signal reaches the output** says "Your voice is muted in Divora. Unmute it, then run the test again." *(Restarts while muted were run by command: see below. The rest wasn't run in the app.)*
- [ ] **M1 — A real call** (owner check — not run; the release's done-when): Settings → Hotkeys → **Mute your voice** (unbound until you set it): bind a key, and in the Stream Deck app give a button a **Hotkey** action that sends it. With Output on CABLE Input, in a live Discord call: press the button → the far end stops hearing your voice; play a soundboard tile and a **Speak** line → the far end hears both while you're muted; press it again → your voice is back, and the far end hears no click as it goes or comes back. Again with Divora hidden to the tray (close the window) → the button still mutes and unmutes; nothing on screen shows the mute unless the stream overlay is open (its circle says Muted).
- [ ] **M2 — Headphone monitor** (owner check — not run): Settings → Audio devices → **Monitor output (hear yourself)** set to your headphones (not the Output device), Monitor on; the note under it ends "The Monitor toggle below silences only this device, never the main send." Mute → your voice goes silent in the headphones, and a soundboard clip still plays there. Set it to **None — use main output** → the note ends "With no separate monitor device, the Monitor toggle below turns Divora's whole output on or off, the call included."
- [ ] **M3 — MIDI** (owner check — not run): Settings → Control surfaces: pick the **MIDI input**, **Add mapping**, set its action to **Mute your voice**, **Learn**, and move the control. A pad toggles: press → muted, press again → unmuted. A latching CC button: on mutes, off unmutes; after the mute was changed elsewhere (the hotkey), one press does nothing and the next acts — that dead press is expected. A momentary CC button mutes while held. A fader mutes when pushed past halfway (64 or more) and unmutes when pulled back below (59 or less). **With a fader resting low, mute from the hotkey, then touch the fader without passing halfway → still muted.** A knob left near halfway (60 to 63) doesn't flap. Relaunch (or pick the MIDI input again) with the fader high and nudge it → it mutes; with it low → nothing.
- [ ] **M4 — A glyph** (owner check — not run): Settings → Glyph casting: set a glyph's action to **Mute / unmute**. On the Mixer, cast it → the omen says **Mute**, and you're muted; cast it again → the omen says **Unmute**, and you're unmuted. Each cast does what its omen said.
- [ ] **M5 — A real unplug** (owner check — not run): mute, then unplug the headset → after Divora recovers it's still muted, and no burst of your voice reaches the call or a recording.
- [ ] **M6 — Voice Convert** (owner check — not run): a preset with Voice Convert and its model installed (e.g. **Deep Narrator**). Mute, say a word, unmute at once → the word is never heard.
- [ ] **M6b — The end-of-word cut** (owner check — not run): the same preset; say a word that ends right as you press **Mute**, with a WAV recording running (or a listener on the call), and note how much of the word is missing. Expected: about its last 75 ms with Deep Narrator, worked out from the code: Voice Convert holds the voice 18 ms with the bundled model plus about 5 ms inside the model, and the preset's denoiser, pitch and formant about 52 ms more. (Through v1.56.0 it was about 0.31 s: the downsampler held about 256 ms whatever the model.) Record the length under Measured below.
- [ ] **M7 — Voice reading** (owner check — not run): the reading on, talking: mute → the card reads **Muted in Divora** with the last reading held, and a screen reader says "Reading paused while your voice is muted."; unmute and keep talking → the panel recovers within a few seconds, with no reading that shows the after-effects half as silent. **At unmute the panel never shows "Input silent", and the screen reader never says "Input is silent."**
- [ ] **M8 — Screen readers** (owner check — not run), with Narrator on; record each case under Measured or Not measured below: **(a)** Divora focused — mute and unmute with the Mixer's button, then with the hotkey: each time "Your voice is muted.", then "Your voice is unmuted."; **(b)** another app focused (Discord) — the hotkey; **(c)** Divora hidden to the tray — the hotkey. Nothing is said at launch: the announcer stays quiet until the first mute. The CHANGELOG's screen-reader sentence covers only the cases that pass.
- [ ] **M9 — Light theme** (owner check — not run): Settings → Appearance → Theme → **Light**, then mute → the titlebar's red **Muted** pill reads clearly.
- [ ] Measured on the development machine (Windows 11 Pro 26200, i7-14700K; 3440×1440 and 1920×1080 screens at 100 %), on one debug build of `mute` at `72ee4e3` (`pnpm tauri build --debug --no-bundle`, with the review's fixes: the FTZ/DAZ flush, the button's focus, the MIDI sides and the Settings copy), driven over WebView2's DevTools protocol — the mute commands sent from the page through the real IPC, except the hotkey run (a real `WM_HOTKEY` from `SendInput`) and the button run (clicks on the Mixer's **Mute** dispatched over DevTools); the engine on the silent route (input the Focusrite "Analogue 1 + 2", output "Speakers (Realtek USB Audio)" gated silent with Monitor off, no monitor device; nothing played into the VB-Audio cable). The voice was a 440 Hz tone (660 Hz for the echo run) at −30 dBFS on the SMSL DAC, which the Focusrite heard at only about −73 to −80 dBFS — far weaker than speech, so the analyser's automatic edge finding failed on the first four runs below and those recordings were measured directly. Every recording was analysed, then deleted:
  - **The call's audio** (the WAV recorder's file, which is the send at the engine rate, before any resampler), a 1 kHz −20 dBFS soundboard clip playing, 10 mutes and unmutes 2.5 s apart: the tone read −79.8 to −81.0 dBFS in the unmuted stretches and −168.9 to −269.3 dBFS in the muted ones — at least 88 dB down, against a 70 dB bar. The clip read −20.0 dBFS in every stretch, muted or not. Each toggle answered in turn ({true, 1} … {false, 10}), within 17 ms of its slot. (A Blackman-Harris Goertzel per stretch over the logged times ±50 ms.)
  - **Voice only** (no clip, 10 toggles): every muted stretch, 5 of 5, is all exact-zero samples; the open stretches carry the mic at −48.6 to −55.8 dBFS RMS. Silence began 0–11 ms after each command was sent (the next buffer plus the fade).
  - **No click** (supporting evidence only): with the mic's room noise as the signal, the 256 samples before silence had 0.38–0.94× (mean about 0.63) the RMS of the steady stretch before them — near the 0.58 a 256-sample linear fade gives, and well under a hard cut's 1.0; at unmute the first 256 samples had 0.38–0.81×. The strict bound, no step larger than the fade's slope, is the unit tests' (`no_step_larger_than_the_slope`, `the_send_is_a_pure_linear_ramp`); the planned 440 Hz measure couldn't run on a tone this weak.
  - **What you say while muted, through an echo** (Echo at 500 ms, feedback 90 %): proves nothing here. The trial passed (no 660 Hz above the reference after unmute), but its positive control failed: the burst reached the mic too weakly to make a measurable echo. It rests on `what_is_said_while_muted_never_reaches_the_send`, which fails with the chain's starving switched off.
  - **Restarts while muted** (a screen take with stems; mute; two engine restarts by command, the second sent 1 s after the first returned; unmute): the voice stem (`<take>.voice.wav`) is exactly 0.0 from 2.05 s (the mute logged at 2.006 s) to 6.80 s (the unmute at 6.79 s), straight through both restarts (3.85 s and 5.78 s), and the take recorded through both. It reported audio lost at 5.03 s (`audioLostAt`) during the second back-to-back restart, cleared by the end (idle, no error): the recorder's existing restart behaviour, not the mute. (From a 50 ms map of the stem; the analyser's own muted window was misaligned by about 0.94 s.)
  - **Replay** (on, the tone, a 5 s mute, save): the tone was 68.8 dB down in the muted stretch of the saved MP4's AAC audio, against a 50 dB bar. The voice reached the clip at only −82 dBFS.
  - **The real hotkey:** **Mute your voice** bound to Ctrl+Alt+Shift+F24 (it registered), the overlay open, the chord sent four times by `SendInput` about 480 ms apart → the engine's change count went 72 → 76, one per press, never two (the page didn't toggle it again). Each press showed in its own tick 12–25 ms later, muted after odd presses and unmuted after even; the titlebar pill read Muted / Clean / Muted / Clean, and so did the overlay's spell circle. The step that should have hidden Divora to the tray left it visible, so this run doesn't show the hotkey while hidden.
  - **A reload while muted** (overlay open): the engine stayed muted with the same change count (69; 71 in a second probe), so the boot sent no unmute or toggle, and after the reload the overlay received only "muted". Sampled every 20 ms, the pill read Clean at 51 ms and **Muted** at 174 ms after the reload: until the boot's mute read lands, the page shows the push-to-modulate status, and the overlay waits for it. That the read goes out before the device list rests on the e2e and unit tests: the run's IPC log didn't survive the reload.
  - **Clicks racing ticks:** 40 clicks on the Mixer's **Mute** button over DevTools, 0–32 ms apart, while ticks ran: 15 of 15 checks passed — one `set_input_muted` per click, each sending what its label said; every animation frame showed the newest state received before the previous frame; the pill's loud style matched its label in every frame.
  - **The commands with real arguments** (from the page, 12 of 12 checks): `set_input_muted {muted:true}` twice → {true, 67} both times (the second changed nothing, the count included), `{muted:false}` → {false, 68}, `input_mute` → {false, 68}; the ticks agreed within 6–23 ms.
  - **CPU during a long mute** (the release bench `a_long_mute_costs_at_most_twice_speech_or_a_pause`, the voice stage on a test thread that flushes as the callback does; every bundled preset, loudness on: 10 s of a 220 Hz tone standing in for speech, then 180 s muted, against 180 s of −70 dBFS room tone): all 24 presets pass (the bar is muted at most 2× max(speech, pause); they came in at 0.68–0.99× max(speech, pause)), e.g. Zombie 277.5 µs speech / 294.8 pause / 243.3 muted per 480-frame buffer, The Oracle 39.0 / 50.2 / 34.1, Clean 3.8 / 11.8 / 11.6. Without the FTZ/DAZ flush, 21 of 24 fail (up to 15.9× max(speech, pause); e.g. The Oracle 429 µs muted, Zombie 1.1 ms): subnormal floats in the starved chain.
  - **Not measured:** real speech, or a voice as loud as speech at the mic (the tone reached it at about −80 dBFS), so "no click" and "nothing said while muted comes back through an echo" rest on unit tests; the hotkey while Divora is hidden to the tray; a Stream Deck (`SendInput` stood in for its **Hotkey** action); the far end of a live call (Discord listens on the cable, and no test audio may go there), a game, and OBS's own mic source; the device signal after the resampler when the input and output rates differ (the WAV is taken before it); any rate but 48 kHz (a Bluetooth hands-free mic at 8 or 16 kHz, a 96 or 192 kHz device: unit tests only); the headphone monitor's own output (by construction); Voice Convert (no model was loaded), and with it the end-of-word cut; screen readers; the overlay's Muted on the stream with the main window hidden; a real device loss (the restarts were by command); a mute held for minutes in the app (the bench covers CPU only); the denormal cost on any other CPU; a screen take's MP4 audio (its voice stem was measured, and Replay's MP4); Space after a mouse click on **Mute**, and the button from the keyboard; stopping and starting the engine while muted (only restarts by command ran), a mute or unmute while it's stopped, and a relaunch while muted; MIDI, the glyph, Voice reading and Test my setup (automated tests only); the light theme; Windows 10. The call, the Stream Deck, the hidden hotkey and the overlay while hidden (M1), the headphones (M2), MIDI (M3), the glyph (M4), a device loss (M5), Voice Convert (M6, M6b), the reading (M7), screen readers (M8, the hidden case included), the light theme (M9), the button, the stopped engine, the relaunch and Test my setup have checks above; a game, OBS's mic source, the resampled device signal, other rates, a long mute in the app, other CPUs, a take's MP4 and Windows 10 have none.

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

## Hotkeys survive a restart (v1.51.4)

- [ ] Settings → Hotkeys: bind push-to-modulate to **Ctrl+Alt+Win+F9** (the chip reads "Win"; it saves as `Ctrl+Alt+Super+F9`) → with **Push to modulate** on in the Mixer (**Hold to apply**), hold it while another app is focused → the voice modulates. Windows or another app owns many Win-key combinations (Ctrl+Win+P was taken on the verification machine); a taken one is refused and only logged (`[hotkey] register failed` in the devtools console), so pick another combination rather than counting it as a regression.
- [ ] Bind panic and monitor too, quit from the tray, relaunch → all three are still shown **and still work** without re-binding.
- [ ] Clear a binding, relaunch → it stays cleared, and the key is free in other apps.

## Stream overlay (v1.16.0)

- [ ] Settings → Stream overlay → toggle **Show overlay window** on → a frameless, always-on-top window appears showing the spell circle.
- [ ] On the Mixer, with **Push to modulate** on, hold Space (the status switches) / change the preset / move Mystical / switch the Color mood → the overlay's circle updates to match (status colour, node count, mystical detail, mood) within a frame.
- [ ] (v1.58.0) With **Push to modulate** off (the default) and a preset with an effect on besides the noise gate, the circle says **Modulated** and animates, where it said **Clean** before v1.58.0. Turn Push to modulate on (**Hold to apply**) → **Clean** until you hold the key. *(e2e; P5 read the overlay's events after a reload: see Push-to-modulate (v1.58.0).)*
- [ ] Set background = **Transparent** → in OBS, add a **Window Capture** of "DivoraVoice Overlay" with a method that supports transparency → the circle composites over your scene with no box.
- [ ] Set background = **Green** (or Magenta) → the overlay fills solid green → add a **Chroma Key** filter in OBS → the green is removed, leaving the circle.
- [ ] Close the overlay window's toggle (or the window's own close) → the window disappears and the toggle reflects it.
- [ ] Restart the app → the background-mode choice is restored.
- [ ] (v1.51.3) Toggle the overlay **off and on three times** → it reappears every time, and the toggle always matches what is on screen.
- [ ] (v1.51.3) Opening it **does not take focus**: click into another app, switch the overlay on, and keep typing → the keystrokes still go to that app.
- [ ] (v1.51.3) **No border** around the transparent window, and **no taskbar button** for it.
- [ ] (v1.55.0) With **Fit to monitor** off, **Arrange**, then drag **Move window** (or an empty spot) → the window moves. Locked, pressing the overlay does nothing (clicks go through). Double-click → nothing (it doesn't fill the screen). *(Not run in the real app.)*
- [ ] (v1.51.3) The circle's status word and node names use the app's fonts (Bricolage / Space Mono), not a system font.
- [ ] (v1.51.3) Move it, then **restart the app** and switch it on → it opens where you left it, at the same size.
- [ ] (v1.51.3) **Mixed scaling:** with a 100 % and a 150 % display, leave it on the 150 % one and restart → same place, same apparent size. Leave it on the second display, unplug that display, switch it on → it opens fully on the main display.
- [ ] (v1.51.3) It shows the **current** preset's chain the instant it opens (no default circle first).

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

- [ ] Preset voices show a **"Soon"** badge + a "Preset voices aren't installed in this build" banner that points at Critter Chatter; pressing **Speak** with a preset selected shows a graceful "text-to-speech voices are not installed" notice — never a hang or crash (same as the AI voice-convert missing-model path). **Your voices** is hidden; **Saved clips** is still shown.

**Your voices — cloning (v1.20.0 / download v1.21.0):**

- [ ] **First run (models not downloaded):** Speak → **Your voices** shows a **Download voice-cloning models (~157 MB)** button. Click it → a progress % appears → on completion the **Add your voice** controls replace it. (Models land in `%APPDATA%/com.divora.voicemod/tts/`; not in the installer.)
- [ ] Restart → the models are still present (no re-download); **Add your voice** is shown directly.
- [ ] Type a name → **Pick a clip** → choose a clean 20–30 s WAV of a voice → after a moment it appears in the list and is auto-selected.
- [ ] **Record in-app (v1.23.0):** with the engine running, type a name → **Record** (button shows **Stop & save**, a "Recording…" hint appears) → speak 20–30 s → **Stop & save** → the voice is cloned, appears, and is auto-selected (same quality as an imported clip). The transient clip is not left behind in `%TEMP%\DivoraVoice`.
- [ ] **Record with the engine stopped (v1.23.0):** stop the engine, then **Record** → a graceful "start the engine first" message appears (no crash); the recorder stays idle.
- [ ] **Auto base-match (v1.22.0):** the new voice shows a small **"based on …"** caption naming a preset (e.g. *based on Puck*). Clone a clearly female and a clearly UK-accented reference → the captioned base shifts toward a female / UK preset accordingly (not always Puck).
- [ ] **Preview (v1.47.0):** with the engine running, press ▶ on a preset voice → you hear a short sample in your MONITOR only (a call/OBS listener hears nothing). Press ▶ on a second voice mid-playback → the first stops; they never overlap. Press ▶ on the same voice again → it stops. Press it a third time → instant (cached). Try a cloned voice → first press takes a few seconds, then is instant after. Check **Saved clips** did NOT grow. With the engine stopped the ▶ buttons are disabled and explain why.
- [ ] Type text → **Speak** with the cloned voice → the output takes on that voice's timbre (accent/cadence come from the Kokoro base). It mixes into a call like any Speak output.
- [ ] **Rename (v1.43.0):** click the **pencil** on a cloned voice → the card becomes an inline editor, already focused with the name selected → type a new name → **Enter** (or ✓) → the card shows the new name, it stays selected if it was, and the **"based on …"** caption + engine badge are unchanged. **Escape** (or ✕) discards the edit and returns focus to the pencil. Restart → the new name persists. Rename it to blank → nothing is saved and the editor stays open.
- [ ] **Reactive (v1.46.0):** on the Mixer, pick a distortion-y voice (Static Wraith / Creeper), turn **Reactive** on → speak normally, then raise your voice: the grit builds as you get louder and eases back within about a second. The **Envelope** meter tracks your level. Turn Intensity to 0 → no movement; to 100 → strongest. Switch to a preset with no Distortion → the card says so. Toggle off mid-sentence → the voice returns to the preset's authored sound with no click. Restart → the on/off state and Intensity persist.
- [ ] **Delete** a cloned voice (trash icon) → it disappears; if it was selected, the selection falls back to a preset.
- [ ] Restart the app → cloned voices persist (stored under `%APPDATA%/com.divora.voicemod/voices/cloned/`); a selected cloned voice is restored.
- [ ] Pick a junk/too-short file → a graceful error appears (no crash).

**Accent-preserving cloning — VoxCPM (v1.24.0):**

- [ ] **Download (first run):** Speak → **Your voices** shows **Accent-preserving clones (~1.6 GB)**. Click → progress % → on completion it disappears (models in `%APPDATA%/com.divora.voicemod/tts/`; not bundled).
- [ ] **Record an accent clone:** with the engine running + accent models present, type a name → **Record** → the hint shows a **fixed sentence to read aloud** (not "speak naturally") → read it → **Stop & save** → the voice appears (no "based on …" caption — it's not preset-based).
- [ ] **Speak it:** type text → **Speak** the recorded voice → the output keeps **your accent and prosody** (not a US/UK preset accent), mixed into a call. Compare to an OpenVoice clone of the same voice (timbre-only) — the accent difference should be audible. (First utterance loads the engine — a few seconds; later ones are quicker.)
- [ ] **Import stays timbre-only:** **Pick a clip** still produces an OpenVoice clone (timbre-only, with a "based on …" caption) even when the accent models are present.
- [ ] Restart → accent clones persist + speak correctly (stored as `reference.wav` under `voices/cloned/<id>/`).

**Experimental GPU (DirectML) cloning — v1.30.0:**

- [ ] With accent clones working on CPU, selecting a VoxCPM clone shows a **GPU** control in the Speak panel (Windows + DX12). First run: **Enable GPU (download ~1.2 GB)** → click → a progress % appears → on completion a **Use GPU (experimental)** checkbox replaces it (persists `divora.cloneGpu`).
- [ ] Enable the toggle → **Speak** the cloned voice → the output keeps your voice/accent (identical in character to CPU) and synthesis is modestly faster (~1.3× on the decode step). No crash, no garbled/robotic audio.
- [ ] Force a GPU-unavailable case (machine without a usable DX12 GPU, or rename `DirectML.dll`): with the toggle on, **Speak** still produces correct audio — it **falls back to CPU** silently (never a hang or crash).
- [ ] Restart with GPU on → the toggle persists and Speak still works.

## Speak — Critter Chatter babble voices (v1.50.0)

Procedural: no model and no download, so every check here should pass in the
built app **and** in a source build with no voice assets at all.

- [ ] Sidebar → **Speak** → a **Critter Chatter** group (caption "Playful babble · built in, no download") lists **Bright**, **Mellow**, **Gruff** below the preset voices. None shows a "Soon" badge.
- [ ] Select each → type a sentence → **Speak** → you hear babble, one short syllable per letter, starting in well under a second. The three are audibly different (Bright highest and fastest, Gruff lowest and slowest).
- [ ] Set output = **CABLE Input** and join a Discord call → a listener hears the babble mixed with your live mic. With **Preview only** on, you hear it in your monitor and the listener does **not**.
- [ ] **Rapid repeat:** type a long line and press **Speak** 5–6 times quickly → only the newest take is audible; the takes never talk over each other. **Stop** silences it immediately. (Repeat once with a Kokoro preset in the built app — same result.)
- [ ] **Audition:** with the engine running, press ▶ on a Critter Chatter card → a short sample plays in your monitor only; ▶ on another card mid-sample stops the first. With the engine stopped the ▶ is disabled with a "Start the engine" hint.
- [ ] **Saved clips:** each Speak adds a clip → **Play** replays the same babble; **Load into editor** restores the text **and** re-selects the Critter Chatter voice.
- [ ] **Persistence:** select **Gruff** → restart the app → Gruff is still selected (`divora.ttsVoice` holds `babble:gruff`).
- [ ] **Source build without Kokoro assets** (skip `fetch-voice-assets.ps1`): the "aren't installed in this build" banner shows and **Your voices** is hidden, but every Critter Chatter check above still works, including **Saved clips**.
- [ ] **Odd text:** Speak each of these → no crash, no hang, and a sensible result: `Grüße aus Köln — ça va?` (accented letters babble, the dash pauses, the `?` rises), `日本語 и кириллица` (non-Latin letters still babble), `Call 555-0123 at 9:30!` (digits babble), `🙂🎉` alone (nothing to voice → a graceful "no speakable text" notice), and a very long pasted paragraph (babbles to a capped length, the app stays responsive).

## What's new (v1.49.0)

These need a real installed build — a dev build reports version `0.0.0` and
deliberately does nothing here.

- [ ] Install over an older version. On first launch a banner appears near the
      bottom reading "Updated to v1.49.0 — What's new".
- [ ] The banner does **not** block anything: the Mixer is fully usable behind
      it, and Start/Stop still work with it on screen.
- [ ] "See what's new" opens the panel; it lists **every** release since the one
      previously installed, newest first, not just the newest.
- [ ] Bold lead-ins and `code` spans render as formatting — no stray `**`
      or backticks anywhere in the panel.
- [ ] Close the panel and relaunch the app: **nothing** is announced again.
- [ ] Dismiss the banner with × (without opening the panel), relaunch:
      still nothing announced.
- [ ] Quit with the banner still showing, relaunch: still nothing announced
      (the version is recorded when shown, not when dismissed).
- [ ] Settings → About → "What's new" opens the panel at any time, including
      when nothing is new.
- [ ] Escape closes the panel; so does clicking the dimmed backdrop.
- [ ] "Full changelog" opens the GitHub CHANGELOG in the default browser.
- [ ] Disconnect from the network entirely, then open the panel from About —
      it renders identically (the notes ship with the app).
- [ ] On a **fresh** install (clear `%APPDATA%` and browser storage first):
      the first-run wizard appears and **no** what's-new banner is shown.

## Voice reading

The panel reports **measured acoustic properties of a signal**. Five are shown
as numbers per half — pitch, pitch range, level, pace, tone — and level
movement reaches the screen only as a word in the phrase (`flat` / `dynamic`).
Voiced ratio is measured and used to decide voicing, but never displayed. It is not emotion recognition and must never read as though it were,
so most of this section is about what the panel says when it has *nothing* to
report. Those are
the cases CI cannot reach and the ones where this feature usually goes wrong.

**Off by default**

- [ ] On a fresh install the Mixer's **Voice reading** card is present with its
      switch **off**, and the card shows no numbers and no phrase.
- [ ] With it off, run the app for a few minutes with the engine live: CPU is
      unchanged from a run without the card (nothing is analyzed).
- [ ] Turn it on, quit, relaunch → it comes back on. Turn it off, relaunch →
      it comes back off.

**Speaking vs silence — the freeze**

- [ ] Engine running, panel on. Speak normally for ~10 seconds: the state reads
      **Live**, the phrase appears (at most three words), and Pitch / Range /
      Level / Pace all move.
- [ ] Stop talking and stay still for a full minute. The state changes to
      **Paused** with a "measured N s ago" counter that keeps climbing, and the
      numbers **hold at the last speaking window** — they must NOT drift down
      toward a quieter, flatter, narrower reading. Watch the phrase too: it
      must not change at all while paused.
- [ ] Keep not talking for over 30 seconds: the held numbers disappear and both
      halves read "Nothing measured yet." The state must be **Too quiet to
      measure** — NOT "Still listening", which would claim the panel has not
      heard enough speech to compare against, when it has. (A reading from
      minutes ago is not a
      stale reading.)
- [ ] Speak again: the panel goes back to **Live** within about a second.

**Mute vs engine stopped vs quiet-but-present**

- [ ] Mute the mic **at the device / in Windows** (a real mute, producing
      digital silence) while the panel is on → the state reads **Input
      silent**, not "too quiet". Unmute → it recovers.
- [ ] (v1.56.0) Mute in Divora (the Mixer's **Mute** button) while the panel
      is on → the state reads **Muted in Divora**, not **Input silent**, with
      the last reading held (cleared after 30 s, as in a pause). Unmute → it
      never shows **Input silent**, not even for a moment, and goes back to
      reading as you speak.
- [ ] Stop the engine from the Mixer → the state reads **Engine stopped**
      within a second, and any held numbers are cleared. Start it again → the
      panel starts over with **Still listening**.
- [ ] Sit in a quiet room with the mic live and say nothing, with no device
      mute → the state reads **Too quiet to measure** (or **Paused**, if a
      window was measured earlier), *not* **Input silent**. These three must
      never be confused: they mean different things and each has its own words.
- [ ] Unplug the mic mid-session (device-loss recovery rebuilds the session) →
      the panel does not keep showing the old reading, and a new baseline is
      built after the rebuild.

**Push-to-modulate with your effects off** (owner check N7 — not run; rewritten for v1.58.0, when push-to-modulate began to switch the effects)

- [ ] Turn **Push to modulate** on in the Mixer (**Hold to apply**) and leave
      the key **up**, engine running, panel on, a preset with a pitch shift
      (e.g. Hollow King). Speak.
- [ ] The **Your voice** half keeps reading normally: the mic is still open,
      only your effects are off. It must not read as muted or stopped.
- [ ] The **After effects** half shows "No measured difference from the
      microphone half — your effects are off, or they are passing the signal
      through." It must not look like a failure, and there must be no wording
      suggesting the preset stopped working. The line under its heading reads
      "What the call hears: your voice after your effects, or as it is while
      Push to modulate has them off."
- [ ] Hold the key down and keep speaking: the note goes, and the after-effects
      half diverges from the microphone half (pitch and/or brightness move)
      while the microphone half is unchanged.
- [ ] Let go and keep speaking: the note comes back with fresh after-effects
      numbers, about half a second to three-quarters of a second after you let
      go (the after-effects half first refills its window), never ahead of
      them.
- [ ] Hold the key, speak a phrase, and stop speaking just as you let go: the
      last numbers stay, dimmed, without the note — they were measured with
      your effects on.
- [ ] **Repeat the whole block with Loudness normalization ON.** This is the
      case that matters: the passing-through note is measured by correlating
      the two taps, and the loudness stage sits between the chain and the wet
      tap, so a bypassed chain still arrives at a different level. With
      loudness on, the note must still appear. (A version that compared the
      two halves' levels instead passed this block with loudness off and
      failed silently with it on.)

**A preset switch moves only the after-effects half**

- [ ] Speak steadily while switching from a neutral preset to one with a large
      pitch shift (e.g. Hollow King, −5 semitones).
- [ ] The **After effects** pitch jumps by roughly the preset's shift within a
      window or two. The **Your voice** pitch does **not** move.
- [ ] The card names the active preset next to "After effects", and carries the
      line saying a preset change moves those numbers on its own.
- [ ] Switch back and forth a few times: only the second half ever reacts.

**A headset in a noisy room (the octave-error case)**

- [ ] Use a gaming headset in a room with fan / keyboard / background noise,
      with the gate and denoiser enabled, and talk for two or three minutes.
- [ ] Watch **Your voice → Pitch** and **Range**. A single window that halves
      or doubles the pitch must **not** appear on screen — the guard holds a
      suspected octave jump until a second window agrees with it.
- [ ] Deliberately change register (speak in a much higher or lower voice and
      hold it): the new pitch does land, after at most one extra window.
- [ ] The Range readout must not flicker between two very different values
      while the voice is steady.

**Speak / soundboard are on neither half**

- [ ] With the panel on and the engine live, play a soundboard clip or a Speak
      utterance while **not** talking. Neither half reacts — that audio is
      mixed in after the chain, downstream of both taps. (This is correct, not
      a bug: the panel reads the microphone, not the mix.)

**Honesty + accessibility**

- [ ] Read every word on the card in each state. Nothing names a feeling
      (happy / sad / angry / stressed / calm / confident / nervous / mood /
      emotion), and nothing addresses the user in the second person about their
      state ("you sound…", "you seem…").
- [ ] The card carries its one-line "measured properties of the sound — not a
      reading of the person speaking" note.
- [ ] Start a screen reader (Narrator or NVDA), turn the panel on, and talk for
      a minute. **The reader must stay quiet** while the numbers move. It may
      announce the state sentence when it changes (stopped / silent / still
      listening / reading), and it must read the values only when the user
      navigates to them. Continuous narration while speaking is a failure.
- [ ] Navigate to a value with the screen reader: it reads a label that names
      which half it belongs to ("Pitch, your voice" vs "Pitch, after effects")
      and a spoken value ("118 hertz").

**Not on the overlay**

- [ ] Open the stream overlay with the panel on and speak. Nothing from the
      reading appears on the overlay — no phrase, no numbers, no state. This is
      deliberate: an overlay audience cannot check a reading, and a guest on
      the mic would be read too.

## Stress

- [ ] Switch presets rapidly (1 per second) for 30 seconds — no crashes, no leaks.
- [ ] Play a soundboard clip while changing input device — recovers gracefully.
- [ ] Pull headset cable mid-call — app shows device-disconnected state without crashing.

## Shutdown

- [ ] Closing the window stops all audio streams within 1 second.
- [ ] No orphaned device handles (check Sound Settings → app should disappear from Volume Mixer).
- [ ] Reopening picks up the same device selection.
