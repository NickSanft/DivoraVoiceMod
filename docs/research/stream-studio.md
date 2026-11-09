# Research spike — stream studio: screen recording, replay, overlay charms, timers, OBS

**Date:** 2026-10-12 · **Baseline:** v1.51.2 · **Status:** plan, no shippable code ·
**Outcome:** **GO** — ten releases: two patches first, then eight minors. The owner answered the first round of questions on 2026-10-12 ([Owner decisions](#owner-decisions)).

## The question

The owner asked whether DivoraVoice can:

1. **Record audio and video of a given screen.** "Audio" means game and Discord audio as well as the modulated voice.
2. **Place draggable elements on a screen** — for example, a picture or GIF that can be moved and resized.
3. **Have timers and other stream-related functionality.** This now explicitly includes:
   - a **replay buffer** that saves the last moments on a hotkey;
   - **OBS WebSocket control**.

### Hard constraints

1. **Local-first, MIT, no account, no telemetry.** No GPL is linked into the app (espeak-ng's arm's-length subprocess is the precedent). Large optional components are only opt-in, explained downloads. Nothing in this plan touches the network without explicit consent.
2. **The audio callback contract** (CHANGELOG v1.51.1/v1.51.2): no allocation, no deallocation, no lock, no syscall. New taps use the existing ring + writer-thread shape.
3. **Additive-only surface** (docs/STABLE-SURFACE.md). New commands, events, optional fields, `divora.*` keys and hotkey-id prefixes are fine. Nothing is renamed, removed, retyped or repurposed.
4. **The per-release ship loop** (docs/PLAN.md:167-191). The What's-new panel compiles in each release's lead prose and its Added/Changed/Fixed/Removed bullets (src/data/changelog-parse.ts), so every one of those sentences must be literally true.
5. **Headless CI.** It runs on windows-2025 with no audio devices, no GPU session and no interactive desktop. Anything that needs them degrades at runtime and is tested through pure seams.
6. **It stays a voice app.** OBS keeps compositing, styling, streaming and recording. Divora builds what a *voice* app is uniquely placed to do, and says plainly what it leaves to OBS.

## Verdict

**Yes to all of it**, in a shape that keeps this a voice app.

- **Recording.** A new workspace crate, `divora-capture`, drives Windows Graphics Capture (WGC) and Media Foundation through the `windows` 0.61 crate Tauri already compiles. That means no new crate versions, no sidecar, no GPL, and H.264/AAC from Windows' own codecs. The voice comes from new rings beside the WAV tap. Game and Discord audio follow in the very next release.
- **Replay.** It reuses the same capture and the same encoder. The encoded video and the mixed audio sit in a ring in memory, and nothing is written to disk until you press Save.
- **Pictures and GIFs.** These extend the existing overlay window with an **Arrange / Locked** mode pair, designed around Tauri's whole-window-only click-through.
- **Timers.** They get a Rust-owned clock: the main window hides to the tray, where WebView2 throttles JS timers. Once a *real* voice mute exists (v1.56.0), timers also get end-of-timer voice actions, which no OBS plugin, Stream Deck timer or Streamer.bot action offers. Examples: "Villager voice for five minutes", "BRB mutes your voice".
- **OBS control.** It is opt-in and off by default, through a small client Divora owns. Timer text goes straight into an OBS text source, a break switches OBS to a BRB scene and back, and an uptime clock follows OBS going live. Divora never starts or stops your stream.

**The honest cost:**
- roughly **50–75 working days** across ten releases;
- several hundred lines of COM the maintainer must own, all inside one crate;
- a hand-written OBS WebSocket client of about 600–800 lines;
- WGC capture, the GPU path, the hardware encoder and OBS itself can only be verified on this machine, not on CI;
- five overlay defects and two hotkey defects that exist today, fixed first;
- a day-5 kill switch on the encoder spike, with a fallback crate.

**Not built:** chat, alerts, goals and anything needing an account.

### How this was checked

**Round 1 — the first eight releases.**
1. **Discovery.** Five agents mapped the repo subsystems this touches: overlay, recorder, action bindings, build constraints and the design system. Six more researched capture, encoding, system audio, scene architecture, stream features and WebView2/Tauri limits against primary sources.
2. **Planning.** Three independent plans (MVP-first, risk-first, user-first) were scored by three judges. The judges were a maintainer, a Windows-multimedia engineer and a streamer. The risk-first plan won narrowly, with grafts from the other two.
3. **Verification.** Every load-bearing claim (40) went to three skeptics: API existence, repo fit, and licence/ethos/platform. **26 stood, 8 were contested and 6 were refuted.** A completeness critic then read the whole plan against the owner's asks.

**Round 2 — after the owner added Replay and OBS to scope.**
1. **Design.** Two designers each drafted one release.
2. **Verification.** Each design's claims went to two skeptics: one checked primary-source API facts, the other repo fit and ethos. Of 47 claims, **40 stood, 3 were contested and 4 were refuted**. The skeptics also listed 50 section problems.

Every correction is folded in below. The refuted claims and what replaced them appear in [Decisions](#decisions), marked **"claim fixed"**, so nothing quietly changed.

## Owner decisions

**Answered by the owner on 2026-10-12.**

| Question | Answer | Effect on the plan |
|---|---|---|
| Does "record audio" include the screen's own audio? | Yes — game and Discord audio | Desktop audio is v1.53.0, directly after the recorder |
| Timers before or after charms? | After | Charms v1.55.0; mute, Hourglass and voice timers follow |
| "Charms" and "Hourglass" as names? | Approved | Used throughout |
| Replay buffer and OBS WebSocket in this cycle? | Yes, add both to scope | Replay is v1.54.0; OBS is v1.61.0 (planned as v1.59.0; see [Releases](#releases)), the last core release |

**Calls made in this revision, which the owner may veto:**
- **Replay goes directly after desktop audio, before charms.** It refactors v1.52.0's encoder: the H.264 MFT is driven directly and fans out to the take and the ring. That refactor is cheapest while v1.52.0's spike results and A/V measurements are fresh. The cost is that Charms land ~9–13 days later.
  - *Alternative:* put Replay after the voice timers. That keeps the first three asks first, but it re-measures takes after unrelated work, and the voice-timer release could not offer "save replay" as an action.
- **OBS control stays small.** It covers timer text, a BRB scene switch and back, and uptime following. It drops two things from the earlier sketch:
  - **"Stop recording at zero."** A misfired timer ending a live stream is the worst failure available.
  - **Show/hide of individual sources.** That is generic automation, which Advanced Scene Switcher and Streamer.bot already own.
- **Replay and OBS actions reach timers and hotkeys first.** MIDI and glyph bindings for them move to the bindings release in the tail. The ask was "save on a hotkey", and keeping every persisted union addition in one place keeps the contract tidy.

**Still open (the plan proceeds on the stated default unless the owner says otherwise):**
1. **Process-loopback dependency** (v1.53.0). *Default:* hand-roll it on `windows` 0.61 in `divora-capture`. *Alternative:* the `wasapi` 0.26 crate, accepting a fourth `windows` copy.
2. **Recorder defaults.** *Default:* match the monitor up to 1440p at 30 fps, cursor on, and leave the yellow capture border as Windows draws it.
3. **A hidden A/V offset control** under Settings → Recordings → Advanced. *Default:* yes.
4. **Verification setup.** *Default:* A/V, click-through, Replay and OBS checks run on this machine only before each tag. The other machine has no toolchain.

## What exists today

**The overlay window (v1.16.0).** How it works today:
- **Creation:** `openOverlay` (src/stores/app.tsx:967-990) creates `WebviewWindow("overlay")` from JS: 480×480, `transparent`, `decorations:false`, `alwaysOnTop` and `resizable`, with no position.
- **Content:** it loads `overlay.html` → src/overlay.tsx, which renders only `<SpellCircle>`.
- **State:** it comes from a broadcast `overlay:state` that an App.tsx:97-116 `createEffect` emits on every tracked change. Nothing flows back.

**Its defects, all in source:**
- **It cannot be moved:** there is no drag region.
- **It draws a 1 px border:** it lacks `shadow:false`, so the transparent undecorated window shows one (tauri #8308).
- **It never reopens:** `closeOverlay` calls `close()`, but src-tauri/src/lib.rs:2235-2240 turns close into `hide()` for *every* window. So the overlay is never destroyed, and the next `openOverlay` finds the hidden window and never shows it.
- **The first update can be lost:** with no ready handshake, the first state push can land before the listener exists.
- **Wrong fonts:** overlay.tsx does not load the @fontsource fonts.

docs/ARCHITECTURE.md has no overlay section.

**The WAV recorder (Phase 16).** How the tap works:
- **Where and what:** the output callback pushes `&mono[..span]` into an 8192-frame SPSC ring while `state.recording` is set (divora-core/src/audio/engine.rs:2038-2040 in v1.56.0). That signal is post-chain, post-loudness mono at the **input** device's rate. Soundboard voices are folded in just before (engine.rs:2029-2031 in v1.56.0).
- **Silent drops:** `push_slice`'s return value is discarded with `let _ =`, so a full ring drops audio silently.
- **Writer:** a per-session thread writes 16-bit mono WAV.
- **Session end:** rings and writers die with the engine session, and nothing restores a take after a device-loss rebuild.
- **No timestamps:** both callbacks discard cpal's timestamps.

Nothing in the app captures video, encodes, or opens a loopback stream.

**Actions and mute.**
- **Hotkey ids** are flat strings: `ptm | panic | monitor` (the `HotkeyAction` type, app.tsx:293-302 in v1.56.0, which has since gained more), plus the dynamic `sb:<clipId>` prefix.
- **MIDI kinds** are `preset | tile | ptm | monitor | param` (src/types.ts:185 in v1.56.0, which adds `mute`).
- **The glyph `mute` action** toggles a **cosmetic** flag (`setUi("muted", !ui.muted)`) that only changes the status pill. **No code path silences the microphone.** (v1.56.0 makes it a real mute of Divora's voice: app.tsx:1103-1107.)
- **Lost bindings:** PTM/panic/monitor bindings are not persisted, although App.tsx and ARCHITECTURE.md say they are.
- **Windows key:** HotkeyCapture emits `"Win"`, which global-hotkey 0.8.0 rejects; it accepts `Super`.
- **No hotkey-id section:** STABLE-SURFACE.md has none, not even for `sb:`.
- **Stream Deck:** it is a help card. A Stream Deck "Hotkey" action reaches whatever a global hotkey reaches.

**Platform facts the plan leans on.**
- **Data directory:** `app_data_dir()` is `%APPDATA%\com.divora.voicemod` (verified on disk). Comments in lib.rs and the docs say `%APPDATA%\DivoraVoice`, which is wrong.
- **Minimum Rust version:** the workspace declares `rust-version = "1.80"`, which is false. The real floor is **1.89**:
  - `symphonia-adapter-libopus` 0.3.0, an unconditional divora-core dependency, declares 1.89, the highest of any locked crate;
  - `ort` rc.12 declares 1.88;
  - the code already uses `is_none_or` and `repeat_n`, which need 1.82.
- **`windows` crate copies:** Cargo.lock compiles three: 0.54.0 (cpal), 0.56.0 (midir) and 0.61.3 (tauri/tao/wry/webview2-com).
- **Nav rail:** a seventh entry would leave 2.25 px spare at the 600 px minimum height, so in practice there is **no free slot**.
- **Tray:** it has only Show and Quit (lib.rs:2186-2219).

## Architecture

End state after v1.61.0. Every item is additive to the v1.51.2 surface.

**Windows.**
- **`main`** is unchanged except that the close handler becomes label-scoped.
- **`overlay`** keeps its label and Vite entry.
  - **Creation:** from v1.55.0 it is created by a Rust `open_overlay` command (`WebviewWindowBuilder`, `visible:false`, click-through set *before* the first show). The backend places it at the rectangle main computes (`placeOverlay`), and shows it once its page says it is listening.
  - **Locked mode** (default): whole-window click-through, no hover.
  - **Arrange mode:** entered only after the overlay reports it has painted; click-through goes off only at the page's first beat, and three 1 s ticks without a beat lock it again (the lease). It has visible chrome and six ways out: Esc, the chrome's Lock button, the Settings button, a tray item (shown only while arranging), a hotkey and the lease.
- **Permissions:** a dedicated `capabilities/overlay.json` grants the overlay event listen, unlisten, emit and emit-to, and start-dragging, and `"overlay"` leaves `default.json`'s window list. Capabilities union per window, so leaving it there would make the restriction a no-op. Capabilities don't cover app commands (there is no app ACL manifest): the overlay relies on `set_overlay_mode`, `overlay_handshake` and `overlay_alive`. Every `set-*` window call happens in Rust, at the rectangle main computes.

**Crates.**
- **`divora-core`** keeps its contract and gains:
  - **two new rings** at the tap site. **Voice** carries `mono` *before* the soundboard fold; **soundboard** carries `sb`. Each gets its own latency correction;
  - a preallocated **side ring** of `(StreamInstant, span, cushion occupancy)` per callback;
  - an `AtomicU64` **drop counter**, and the mute: one `AtomicU64` word, `input_mute` (`seq << 1 | muted`, v1.56.0); and push-to-modulate's, `ptm` (`seq << 16 | mode << 8 | held`, v1.58.0), with a per-session dry line inside the voice stage;
  - a **drain thread** whose sender lives *outside* the engine session, so a device-loss rebuild re-attaches it rather than silencing the rest of the take.
- **`divora-capture`** is a new workspace member with no tauri or cpal dependency.
  - **Unsafe scope:** its FFI modules are the tree's only production FFI `unsafe`, each with a justification and `// SAFETY:` comments. The one other production `#[allow(unsafe_code)]` is `divora-core`'s `audio::denormals` (v1.56.0), an `asm!` block that sets FTZ and DAZ on the audio thread.
  - **Modules:** `probe`, `monitors`, `wgc`, `encoder`, `mux`, `replay`, `loopback` and `dpapi` (v1.61.0: its calls in `native::dpapi`, under the crate's one unsafe scope, `native`; there is no `ffi` module). The pure, CI-tested pieces are `FramePacer`, `ClockFit`, `AudioTimeline` and `ReplayRing`.
  - **`CapturePipeline`:** fans encoded packets out to `PacketSink` consumers. These are the take's muxer and the replay ring.
  - **Runtime-only Media Foundation:** `mfplat.dll`/`mfreadwrite.dll` are loaded with `LoadLibraryExW(…SYSTEM32)` + `GetProcAddress` for every MF free function used. COM methods are vtable calls and import nothing. A static import would stop `divora-app.exe` from launching at all on Windows N editions, where it launches today. v1.61.0's `crypt32.dll` (DPAPI) is loaded the same way, so it isn't among the exe's imports, and the import guard refuses a load-time one (the release exe's whole import set is T10's, in v1.61.0's As built).
- **`src-tauri`** owns:
  - `AppState.screen_recorder`, which outlives engine sessions and holds the pipeline and the replay state;
  - `timers.rs`, `overlay_assets.rs` and `overlay_windows.rs`;
  - `obs/` (v1.61.0): `proto.rs`, `machine.rs` and `scenes.rs`, all pure, plus `link.rs` (a running link's two tasks), `files.rs` (the password and the slots' file), `log.rs` and `mod.rs` (`ObsShared`, managed by `obs::init` in setup).

  Recorder, replay and OBS commands are `async`. Tauri 2 runs sync commands on the main thread, which every window shares. That orders only commands sent one after another: commands in flight together can reach Rust out of the order they were sent (v1.59.0's S2, 40 sent at once), so since v1.59.0 the store sends the mute's, push-to-modulate's and the timers' commands that change something one at a time, through a queue per feature (`src/data/inOrder.ts`); since v1.60.0 the effect chain's (`set_effect_chain`, `set_effect_param`, `set_effect_enabled`, `set_voice_model`) and the soundboard's plays and stops too, not its master volume (`finish_timer` goes through the timers'); the OBS commands get one too (v1.61.0).

**Threads.** The cpal callbacks keep their rules. Per buffer, the new work is two ring pushes, one side-ring push, one `fetch_add`, one atomic load and (v1.56.0) setting FTZ and DAZ in MXCSR, plus (v1.58.0) one more atomic load and push-to-modulate's dry line (a ring write and a delayed read every buffer; the voice itself is touched only while the effects are off or switching), all lock-free and allocation-free.
- **`divora-av-tap` (5 ms):** forwards chunks over a bounded channel.
- **`divora-encoder` (MTA COM):** owns the D3D11 device, frame pool, video processor, H.264 MFT and fan-out. It lives one per take in v1.52.0 and becomes one per pipeline in v1.54.0. WGC's FrameArrived only signals it.
- **`divora-replay-save`:** short-lived, one per save. It muxes a snapshot so capture never waits on disk.
- **`divora-loopback`:** owns the desktop-audio stream.
- **`divora-timers` (100 ms):** owns timer deadlines.
- **`divora-timer-actions` (v1.60.0):** runs the backend's half of a timer's actions, handed over by `try_send` so `divora-timers` never waits: the mute, Stop all clips and lines, a replay save handed on to the blocking pool, a line put in the slot of **`divora-timer-lines`**, which renders one line at a time and plays it. v1.61.0's OBS operations go to the link from here.
- **The OBS link:** two tasks on Tauri's existing tokio runtime, with no new OS thread and no second runtime. They exist only while the user has OBS control switched on.

```
 main (WebView2, hides to tray)                  overlay (WebView2, transparent, shadow:false,
 ┌───────────────────────────────┐              topmost; Locked by default, Arrange on request)
 │ store · Settings rows ·       │ overlay:state ┌───────────────────────────────┐
 │ Mixer cards · charm/timer defs│──────────────▶│ scene: spell circle, charms,  │
 │                               │◀──────────────│ timer face; emits on pointerup│
 └───────────────┬───────────────┘ ready / edit  └───────────────────────────────┘
                 │ invoke (recorder/replay/OBS commands async)  ▲ timer-state, timer-actions (broadcast)
 ════════════════╪════════════════ Tauri IPC ═══════════════════╪═════════════════════
 src-tauri                                                       │
 ├─ commands: *_screen_recording · *_replay_buffer · save_replay · open_overlay ·
 │            set_overlay_mode · import_overlay_asset · *_timer (finish_timer, v1.60.0) ·
 │            set_input_muted · set_ptm_* · obs_*
 ├─ AppState.screen_recorder (outlives engine sessions; owns the CapturePipeline + replay state)
 ├─ thread divora-timers (100 ms; timers/<id>.txt on whole seconds) ──text──────┐
 │    ├─ emits timer-actions (v1.60.0: the page's half, presets and clips)      │
 │    └─ try_send ─▶ thread divora-timer-actions (v1.60.0: mute, Stop all,      │
 │         a replay save, lines) ──ops (v1.61.0)────────────────────────────────┤
 │         └─ thread divora-timer-lines (a timer's line, one at a time)         │
 ├─ obs link (2 tokio tasks, only when enabled) ◀───────────────────────────────┘
 │        └──── ws://<ip>:4455 ────▶ OBS (obs-websocket v5, spoken to, not linked)
 └─ tray: Show · Save replay · Turn Replay off · Lock the overlay · Quit
        │                                           │
        ▼                                           ▼
 divora-capture (NEW; windows 0.61;            divora-core (callback contract unchanged)
 the tree's only FFI unsafe scope)             ├─ output callback: + voice ring, + sb ring,
 ├─ probe   (LoadLibrary mfplat → MFStartup    │    + (StreamInstant, span, cushion) side ring,
 │           → MFTEnumEx; build ≥ 18362)       │    + dropped fetch_add, + input_muted load, + ptm load
 ├─ wgc     (CreateForMonitor, 2-buffer pool)  ├─ thread divora-av-tap (5 ms) ──AvChunk──┐
 ├─ loopback (process loopback, v1.53.0) ──────┼──────────────────────────────────────┐  │
 ├─ thread divora-encoder: BGRA→NV12 (GPU) →   ├─ thread divora-recording (WAV, as today)│  │
 │   H.264 MFT ─┬─▶ TakeMux (IMFSinkWriter     └─ engine thread                          │  │
 │              │    passthrough + AAC)  ◀────────────── AudioTimeline (PCM) ◀───────────┴──┘
 │              └─▶ ReplayRing (RAM) ──snapshot──▶ thread divora-replay-save ──▶ *.mp4
 ├─ native::dpapi (OBS password, v1.61.0; crypt32.dll loaded at run time)
 └─ FrameSink trait (kill switch: windows-capture 2.0.1 behind it, a code branch, never a feature)
```

## Releases

Version numbers are planning labels; a patch landing in between shifts the rest. Two minors landed after v1.56.0: v1.57.0 (Voice Convert keeps up, outside this plan) and v1.58.0 (push-to-modulate, from v1.56.0's follow-ups). So Hourglass, the voice timers and Talk to OBS, planned as v1.57.0, v1.58.0 and v1.59.0, are now v1.59.0, v1.60.0 and v1.61.0. The tail, then labelled v1.62.0 to v1.64.0, was split on 2026-11-07 into six minors, v1.62.0 to v1.67.0 (Tail releases, below): the yellow-border spike, labelled v1.63.0 there, is now v1.65.0, and the timers and bindings, labelled v1.64.0 there and in v1.61.0's As built, are now v1.67.0.

**Every release ships with:**
- **The full ship loop:** tests, the pre-push checklist, a CI conclusion of literally `success`, then the annotated tag.
- **STABLE-SURFACE.md rows** for every command, event, wire type, storage key, hotkey-id prefix and on-disk file it adds.
- **User-facing CHANGELOG prose.** Crate, MSRV and timing notes go under "Architecture notes", which the What's-new panel does not show.

### v1.51.3 — Fix: the stream overlay window (patch, 1–2 days)

**Goal.** Make the existing window correct before anything builds on it. This is also the cheapest probe of transparent-window behaviour on the current WebView2 runtime.

**Deliverables.**
1. **Close handling.** lib.rs hides on close only when `label == "main"`, through a `#[doc(hidden)] pub fn hides_on_close(label)` that the test calls directly. The overlay's close now destroys it, and the reuse path calls `show()`.
2. **Window options:** `shadow:false`, `skipTaskbar:true` and `focus:false`.
3. **Geometry.**
   - **Persisted:** in **physical** pixels, with the monitor name and scale factor. These are optional fields on `divora.overlay`, back-compatible with `{bg}`.
   - **Restored:** after creation, with `setPosition(PhysicalPosition)`/`setSize(PhysicalSize)`. Creation options are *logical* pixels, while `onMoved`/`availableMonitors` report physical ones.
   - **Clamped:** by a pure `clampToMonitors()`, tested at mixed 100 %/150 %.
4. **A root `data-tauri-drag-region`.** SpellCircle's buttons stay clickable because a drag starts only on the attributed element.
5. **Ready handshake.** The overlay emits `overlay:ready` after a double rAF, and main re-pushes the current state.
6. **Fonts:** overlay.tsx loads the fonts.
7. **`rust-version` = 1.89.** This is the floor `symphonia-adapter-libopus` 0.3.0 already sets. It goes in Architecture notes, not What's-new.
8. **Docs:**
   - ARCHITECTURE.md gains the missing overlay section;
   - STABLE-SURFACE and the lib.rs comments get the real data directory;
   - MANUAL_TESTS gains move, off-on-off and restart rows.

**Tests.**
- **Rust:** `hides_on_close`.
- **Vitest:** `{bg}`-only back-compat, `clampToMonitors` and the ready re-push.
- **e2e:** the Settings strings stay unchanged and the boot stays console-clean.

**Done when.** A 30-second clip shows the overlay toggled off and on three times, dragged, and the app restarted. It reappears where it was, shows the current chain immediately, and has no border and no taskbar entry.

### v1.51.4 — Fix: hotkeys that forgot their keys (patch, 1 day)

**Deliverables.**
1. **Persistence:** PTM/panic/monitor bindings are saved under a new `divora.hotkeyBindings` key.
2. **Windows key:** HotkeyCapture emits `Super` for it.
3. **A Hotkey ids section in STABLE-SURFACE.md**, listing `ptm`, `panic`, `monitor` and the `sb:` prefix. Every later prefix (`rec:`, `overlay:`, `tm:`) gets its row there.

**Changelog.** The Fixed bullets say what is literally true: these bindings never survived a restart, although the docs said they did.

**Done when.** Ctrl+Alt+Win+F9 bound to push-to-modulate is still registered after a restart. (Ctrl+Win+P is owned by Windows on the verification machine.)

### v1.52.0 — Record your screen with your voice (10–15 days; spike gate on day 5)

**Goal.** One click records a chosen monitor to an MP4: H.264 High, plus AAC-LC 48 kHz stereo at 192 kbps.
- **What starts it:** the button, or one hotkey, so a Stream Deck key works.
- **What it carries:** your modulated voice and soundboard, with a measured A/V offset of 40 ms or less.
- **What it needs:** nothing new. It uses Windows' own codecs, adds no new crate versions, and needs no OBS.

**Deliverables.**
1. **`probe::capture_status()`**, cached. It checks, in order:
   1. load mfplat/mfreadwrite;
   2. `MFStartup`;
   3. `MFTEnumEx` for H.264 and AAC;
   4. OS build ≥ 18362 *and* a successful `CreateForMonitor`.

   `GraphicsCaptureSession::IsSupported()` is not enough on its own: it returns true on LTSC 2019 (17763), which is below the floor. Any miss disables the card with a reason, never an error:
   - *"Install the Media Feature Pack (Settings › Apps › Optional features), then restart"*;
   - *"Screen recording needs Windows 10 version 1903 or later."*

   CI asserts that `divora-app.exe`'s import table names no `mf*.dll`.
2. **Capture.**
   - **WGC:** `CreateForMonitor`, a free-threaded pool with **2** BGRA buffers, and a cursor toggle behind `IsPropertyPresent`.
   - **Size changes:** a change in `ContentSize` mid-take (a game mode switch, a DPI change) triggers a pool `Recreate` and scaling to the fixed encoder size.
3. **Encoder ladder.** Each rung is reached only if the previous one fails the spike.
   1. **Sink writer with a D3D manager:**
      - attributes `MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS`, `MF_SINK_WRITER_D3D_MANAGER` and `MF_SINK_WRITER_DISABLE_THROTTLING`;
      - CODECAPI rate control via `SetInputMediaType`'s encoding parameters;
      - frames converted **BGRA→NV12 by a D3D11 video processor**. The inbox H.264 encoder takes only NV12/I420/IYUV/YUY2/YV12, and nothing documents the writer inserting an RGB→NV12 converter.
   2. **The hardware H.264 MFT driven directly** (`MFTEnumEx` HARDWARE, `MFT_MESSAGE_SET_D3D_MANAGER`, `ProcessInput`/`ProcessOutput`), with the sink writer as a passthrough muxer. This is the robmikh/displayrecorder pattern (MIT, windows 0.61), and the shape v1.54.0 adopts for every take.
   3. **The `windows-capture = "=2.0.1"` kill switch** (below).

   **On every rung:**
   - **Encoder check:** the encoder actually loaded is checked via `GetServiceForStream` and logged as hardware or software rather than assumed.
   - **CPU fallback:** it converts BGRA→NV12 itself.
   - **Output file:** written as `<name>.part` with `MF_TRANSCODE_CONTAINERTYPE = MPEG4`, because the extension cannot pick the container. It is renamed after the writer is released.
4. **Output resolution:** "Match monitor, up to 1440p" at 30 fps by default. 60 fps and native 4K are options. The H.264 level is chosen from the size.
5. **`FramePacer`.** WGC delivers frames only when the screen changes; the pacer turns them into constant-rate timestamps, repeating the last frame on a static screen.
6. **The audio, precisely.**
   - **Sources:** the voice ring carries `mono[..span]` *before* the soundboard fold; the soundboard ring carries `sb[..span]`. The side ring carries each callback's `StreamInstant`. That value is `Copy`, already computed by cpal from IAudioClock's QPC, and adds no clock read.
   - **Relative timing** is exact, via the public `duration_since` against the first instant. Voice-conversion overruns (one callback in twenty-six, engine.rs:1594) therefore show up as steps rather than being averaged away.
   - **Absolute anchor:** taken once per pipeline. The first instant's QPC value is read off the drain thread and cross-checked with `QueryPerformanceCounter`. The fallback pairs it with `Instant` at drain time. Any constant residue goes to a hidden **A/V offset** control.
   - **Corrections:** voice PTS subtracts input latency, cushion occupancy and `chain.latency_samples`; soundboard PTS subtracts none of them. That matters because a bring-your-own non-streaming voice model adds ≈256 ms to the voice and nothing to a soundboard sting (voice_convert.rs:582-592).
   - **`AudioTimeline`** inserts silence for counted drops, then resamples each source to 48 kHz, skipping the stage when the source is already at 48 kHz (`MonoResampler` has no bypass of its own). It then mixes, converts to stereo, clamps to i16, and emits 1024-frame AAC blocks.
   - **`ClockFit`** handles drift only, nudging `set_resample_ratio_relative` within ±0.5 %.
7. **`ScreenRecorder`** (Idle → Starting → Recording → Stopping) outlives engine sessions.
   - **Device-loss rebuild:** the tap re-attaches, and the resampler is rebuilt if the rate changed.
   - **Engine stop mid-take:** the audio turns to silence, and the status reads "audio lost at mm:ss".
   - **Engine already stopped:** starting a take is refused with the same message as the WAV recorder.
8. **Wire surface.**
   - **Commands:**
     - `screen_capture_status`;
     - `list_capture_monitors`;
     - `start_screen_recording {monitorId, filename, fps?, quality?, cursor?}` → path, async;
     - `stop_screen_recording` → path, async.
   - **Event:** `screen-recording {state, path, droppedFrames, driftPpm, error}`.
   - **Files:** `recordings/divora-screen-<date>_<time>.mp4` (written as `.part`).
   - **Hotkey ids:** `rec:screen`, plus `rec:wav` so the WAV recorder gets one too. Both are unbound by default, and the **`rec:` prefix row** is added to STABLE-SURFACE.
9. **UI.**
   - **`ScreenRecordCard`** beside the WAV `RecordCard` on the Mixer rail. Its sub copy reads *"Your modulated voice and soundboard"*, and it has a monitor picker.
   - **Settings → Recordings:** fps, quality and cursor, plus Advanced → A/V offset.
   - **Card copy** says that recording the monitor showing Divora records Divora's own window, that HDR screens record washed-out (SDR only for now), and that the yellow border is drawn by Windows.
   - **New Sigils:** `record` and `screen`.
   - **README:** gains *"Screen recording: Windows 10 1903 or later"* and *"H.264/AAC encoding is done by Windows' bundled codecs"*.

**Spike gate (day 5).**
- **What must pass:** 30 s of the primary monitor at 1080p30 plays in Windows Media Player, VLC and an editor. Dropped frames stay under 2 % idle and under 5 % with a game running. The encoder name appears in the log.
- **If it fails after rung 2,** take the **kill switch**: swap the internals for `windows-capture` behind `FrameSink`. This is a code branch, never a Cargo feature, because CI builds `--all-features`, which would defeat the duplicate-`windows` guard. On that branch:
  - accept the fourth `windows` copy (0.62.2), updating the CI guard's allowlist in the same commit;
  - prepend silence to the audio, because its encoder clocks audio by sample count;
  - re-send the last frame on a ~1 s watchdog, because its sample callbacks block;
  - check its import table for static MF imports too.

  This branch also removes Replay (v1.54.0).

**Tests.**
- **CI (pure logic):**
  - `FramePacer`;
  - `ClockFit`: converges on +80 ppm with ±5 ms jitter, and flags a 100 ms step;
  - `AudioTimeline`: drops, gaps, per-source offsets, and 44.1/48/96 kHz sources;
  - `ScreenRecorder` transitions with a mock `FrameSink`;
  - the filename sanitiser and serde key freezes.
- **CI (callback seam):** the output callback stays allocation-free with the new rings. This runs through the real callback with voice convert off, since its inference still allocates. A seam check confirms the pushed count is routed, not discarded.
- **CI (encoder smoke, not ignored):**
  - **Write:** ≥ 30 synthetic NV12 frames → software sink writer → MP4.
  - **Check:** the `ftyp`/`moov` boxes are parsed back.
  - **Early return,** with a log line, if mfplat cannot load, `MFStartup` fails, or no encoder is enumerated.
- **Ignored on CI:** real WGC capture, the GPU path and AAC (`#[ignore]` plus a `CI` early return).
- **Vitest and e2e:** card states, and the `screen-recording` event fired through `__E2E_EMIT__`.

**Done when.** A 30-second clip:
1. Pick a monitor, press the hotkey, talk in the Villager preset, play a sting, stop.
2. The MP4 opens in VLC and an editor with an offset of 40 ms or less, and under 20 ms of drift across 10 minutes. The numbers are recorded in MANUAL_TESTS.
3. Forcing the probe to "no Media Foundation" shows the hint, never an error.

**Not in this release.**
- **Audio:** no game or Discord audio (that is next).
- **Capture scope:** no window capture.
- **Border:** no border removal.
- **Overlay:** no hiding the overlay from the recording (v1.55.0).
- **Format:** SDR only.
- **Crash recovery:** a crash mid-take loses that take, and the `.part` name says so.

**As built (2026-10-14).**
- **Spike gate:** passed on **rung 1** (sink writer + DXGI device manager → "NVIDIA H.264 Encoder MFT", read back). A 30 s take of the 3440×1440 screen at 30 fps had 900 frames, 0 dropped; it decoded cleanly and was tagged BT.709 studio range. The game-running drop figure was not measured. Rungs 2 and 3 were not needed, so v1.54.0 builds the directly driven encoder.
- **Differences from the deliverables above:**
  - **The anchor** is one `Instant::now()` in the output callback on the first buffer of a take, paired with that buffer's `StreamInstant`. cpal's instant is never parsed, and the drain needs no QPC cross-check. Per buffer the callback still reads no clock.
  - **`AudioTimeline` reads the mix through a windowed sinc at `ClockFit`'s fitted positions** straight into 48 kHz, rather than driving `MonoResampler` and nudging its ratio. The ±0.5 % clamp bounds the glide. Steps, meaning a device that starved, are detected after cancelling scheduling jitter with each callback's device frame count.
  - **Voice placement:** the jittery part (input latency + ring depth) is smoothed and moved only in a pause, or mid-word when 20 ms out. The chain's part is applied exactly.
  - **Recorder transitions** are tested as a pure table (`screen.rs`) instead of with a mock `FrameSink`. There is no `FrameSink` trait, because the kill switch was not taken.
  - **CPU fallback** is used where the device has no video support. It is forced on with `DIVORA_CAPTURE_CPU_CONVERT=1` and checked against the GPU path. The CPU path stretches a changed aspect; the GPU path letterboxes it.
  - **The probe** also requires an AAC encoder. `DIVORA_PRETEND_NO_MEDIA_FOUNDATION=1` forces the "no Media Foundation" path for manual checks.
- **Measured** (MANUAL_TESTS → Screen recording), on the soundboard path:
  - **A/V offset:** −6 to −19 ms at 60 fps.
  - **Drift over 10 minutes:** a fitted −7 ms, within frame-rate noise (11 events, −32 to +4.5 ms at 30 fps; one tick skipped). The audio clock was within 0.5 ppm, so real drift correction rests on the tests.
  - **Not measured:** the voice path end to end. The machine had no physical microphone, and a test signal on the virtual cable could have reached a live call.

### v1.53.0 — Game and Discord audio in your recordings (6–9 days)

**Goal.** Complete ask 1: the screen's own audio, mixed into the recording's single AAC track **without doubling your voice**.

**Why process loopback first.** In Divora's recommended setup the send goes to CABLE Input and the monitor to headphones (docs/PLAN.md, Phase 13). Recording the headphone endpoint therefore captures Divora's own monitor mix:
- your voice, whenever Monitor is on;
- the soundboard and Speak, whenever their monitor is on.

The clean fix is to capture **every process except Divora**.

**Deliverables.**
1. **Process loopback.**
   - **API:** `ActivateAudioInterfaceAsync` with `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` on Divora's own PID.
   - **Where:** in `divora-capture`'s FFI scope on windows 0.61, with an `#[implement]` completion handler. The alternative is the `wasapi` 0.26 crate (see the still-open owner question).
   - **Format and timing:** dictated by the app (48 kHz f32 stereo), with per-packet QPC from `GetBuffer`.
   - **Availability:** runtime-detected. Microsoft documents it for build 20348+; community reports have it working on updated Windows 10 2004+.
2. **Fallback where activation fails:** cpal 0.15.3 endpoint loopback.
   - **Stream:** `build_input_stream` on an output device, with the format from `default_output_config()`, F32 only.
   - **Doubling guard:** compares **endpoint IDs**, not friendly names, against the engine's output and monitor. If Divora monitors on that endpoint, it asks for monitoring to be off for the take.
   - **Idle gaps:** gaps while the endpoint is idle are zero-filled from the timestamps.
3. **`AudioTimeline` gains the desktop source:**
   - its own drift fit;
   - a soft-clip mix with a desktop gain;
   - optional sidecar stems `<name>.voice.wav` / `<name>.desktop.wav` (the MP4 sink takes one audio stream);
   - a "desktop audio silent for 10 s" warning.
4. **Wire surface:** `list_desktop_audio_sources`, and new optional arguments `start_screen_recording {desktopAudio?, desktopGain?, stems?}`.
5. **UI:** a "Desktop audio" picker on the card. It defaults to "Everything except Divora" where process loopback is available.
6. **cpal stays at 0.15.3.** Version 0.18 changed rerouting and timestamp semantics that touch the v1.33.0 and v1.51.x fixes.

**Tests.**
- **CI (pure logic):** gap fill, the drift controller, the mixer, stems alignment and the guard predicate.
- **CI (callback seam):** loopback ingestion stays allocation-free.
- **Skipped on CI:** device paths.

**Done when.** A 30-minute take of a game plus the modulated voice has no doubled voice, and every track is within 20 ms of the video at the end.

**Not.**
- **No per-application capture picker.**
- **No DRM-protected playback.** Loopback never captures it, and the copy says so.

**As built (2026-10-15).**
- **The capture thread registers with the multimedia scheduler** ("Audio", avrt.dll resolved at run time) while a stream is open. It was added after a first 30-minute take at the highest normal priority had two 20 ms gaps; a second take under the scheduler had two as well, and a trace then showed them to be Windows' — process loopback skips about 15 ms and sends 30 ms of zeros whenever another app opens or closes a stream (the stimulus, at the start and the end). The registration stays: it is how Windows keeps audio threads on time.
- **Process loopback leaves a gap of about 40 ms whenever another app starts or stops playing** on the same device (measured with a tone throughout; endpoint loopback had it whole; other devices untested). Nothing in Divora can recover it; the release notes say so.
- **Owner question 1 taken at its default:** process loopback is hand-written on `windows` 0.61 in `divora-capture` (`native/loopback.rs`). No new crate versions: `windows-core` 0.61.2 (needed by name for `#[implement]`), `ringbuf` and `hound` were already in the lock. A CI step now pins the `windows` copies in the app's normal dependencies on Windows to 0.54.0, 0.56.0 and 0.61.3 (build and dev dependencies aren't checked) — the guard this plan referred to (v1.52.0 kill switch) did not exist until now.
- **Differences from the deliverables above:**
  - **The fallback is hand-written too, not cpal.** cpal 0.15.3 discards `GetBuffer`'s flags (SILENT packets were measured to occur), exposes no endpoint id, panics on COM failures, and cannot live in `divora-capture`. Both paths share one loop (`desktop_thread.rs`, generic over the device so CI drives it with scripted ones) and ask Windows for the same 48 kHz float stereo, converted with `AUTOCONVERTPCM` — no mix-format parsing.
  - **"Everything" is never swapped for a device.** Where it is offered and fails at take time, the take records without desktop audio and says why (`desktopError`): a device can be carrying Divora's own voice. Playback devices are offered only where process loopback is not, since "everything" is not tied to one device — it recorded clicks played on a USB DAC that was not the Windows default — though neither it nor endpoint loopback hears this machine's default Realtek USB device.
  - **The guard warns rather than asks, and matches by name, not endpoint id.** It knows Divora's devices by the *names* its engine opened (the engine's notion of identity; two devices sharing a name both count). A device Divora sends your voice to when the take starts is not recorded, and one it starts sending your voice to during the take goes in as silence from the next time the recorder takes in desktop audio (the feed reads the engine's routing on every pump, about every 20 ms); for the device you monitor on, or Divora's only output, the card warns before the take and `desktopDoubled` reports it during the take, updated once a second. It does not switch monitoring off for the take.
  - **Defaults:** desktop audio is "everything" on a first run, and **off** for anyone upgrading — a take started from a hotkey must not begin recording a call unannounced. Volume −6 dB, range −30…0 dB (`desktopGainDb`, a name `arm_replay_buffer` now uses too). Stems off.
  - **The timeline:** each desktop run has its own fit (a break never bridged by a step); the 10 s "silent" check counts sound (−60 dBFS), not packets; with desktop audio the track is stereo through a soft knee at −3 dBFS, and without it v1.52.0's to the bit.
  - **No per-application picker, no "follow the default device" (a picked device stays picked), no `IMMNotificationClient`.**
- **Measured** (MANUAL_TESTS → Desktop audio), with a separate stimulus process flashing a square and playing a −80 dBFS click on a USB DAC:
  - **A/V:** "everything" +0.2 to +17 ms (mean +15 ms, 10 events at 60 fps, after the code review's fixes; an earlier take before them gave +2 to +19 ms, mean +14); a playback device −4 to +29 ms (mean +16 ms, 10 events, before the fixes). The raw means are within 2 ms, but the stimulus's own delay before the burst averaged 3 ms in the device run and 10 ms in the later "everything" run, so this does not show the two paths agreeing more closely than that; no per-path constant.
  - **Stems** line up with the MP4's decoded audio to within one sample: soundboard stings sat at the same sample in the voice stem and in the MP4 in 79 of 80 cases over five takes (three short, two of 30 minutes); the other, in the 30-minute take before the review's fixes, was one sample apart (the MP4 has no edit list; MF's AAC output starts on time). Past a WAV file's size limit they stop and `stemsError` says where.
  - **Doubling:** with Divora's output on the DAC and Monitor on, "everything" recorded exactly 0 of Divora's sting; the device itself caught it and reported `desktopDoubled`; with Monitor off, 0 and not reported.
  - **Mid-take send:** recording the DAC as a device, with Divora's output moved onto it (its monitor on the Realtek) and back, its clicks (one a second) were left out between the switches — the switch times known only to the second — and Divora's stings there were exactly 0; the card gave the reason, then dropped it.
  - **The voice, with a microphone** (Focusrite, added afterwards): ticks the mic heard from the SMSL's speakers landed in the voice stem 90–92 ms after the same tick in the desktop stem, within about 7 ms of that with a pitch effect; the DAC's own output time (`IAudioClock`) was about 17 ms after the desktop copy, leaving about 74 ms, and the Focusrite's raw Windows timestamps put the tick 105 ms after the DAC. Mic driver or speakers: not settled (a key-press reference would). With this pair the voice can land up to about 70 ms late; the A/V offset corrects it.
  - **30 minutes after the fixes:** 29 of 31 events at +18.8 ms, 2 a frame earlier, +2 ms trend end to end; stings 31 of 31 at the same sample in the stems and the MP4.
  - **Not measured:** doubling with a real voice (checked with a soundboard sting); which of the mic and the speakers holds the extra 70 ms above; a live Discord call; Windows 10; a surround (5.1/7.1) device on the fallback; a real unplug mid-take; DRM-protected playback. This machine's own default device (Realtek USB with a vendor virtual mixer) is invisible to both loopbacks — the 10 s warning is what a user with such a device sees.

### v1.54.0 — Replay: save the last moments on a hotkey (9–13 days; gate on day 4 if v1.52.0 shipped rung 1)

**Goal.** Something good just happened. You press one key and get the last 15, 30, 60 or 120 seconds of one screen as an MP4, without having been recording.
- **What triggers a save:** the hotkey (so a Stream Deck key works), the tray, or the card.
- **What the clip carries:** your modulated voice, your soundboard and, if you picked it, game and Discord audio.
- **Off by default:** Replay is off until you turn it on, and never on at launch.
- **No disk writes:** Divora writes no video or audio to disk until you save.

**What it changes in v1.52.0.**
- **The sink writer never hands back encoded samples.** `IMFSinkWriter` returns none to its caller (windows 0.61.3 `Win32/Media/MediaFoundation/mod.rs:34047-34093`). The only way to observe rung 1's output would be a custom `IMFMediaSink`, which is more owned COM than driving the MFT directly.
- **So every take moves to rung 2's shape.** The H.264 MFT is driven directly and its output fans out to the take's muxer and the replay ring. Microsoft documents "compressed input with identical output" as a remux, and `SetInputMediaType` loads an encoder only "if the formats do not match".
- **Encoder change disclosed:** the Changed bullet says takes now use the new encoder path. The A/V offset is **re-measured** for takes and clips, and the hidden offset control's default moves if the measurement did.
- **Effort by v1.52.0's outcome:**
  - **Rung 2 shipped:** this release reuses it.
  - **Rung 1 shipped:** this release builds the directly driven encoder. Gate on day 4: 30 s through the directly driven hardware MFT must play in VLC and WMP with < 2 % drops. Otherwise the release is dropped and the plan says so.
  - **Kill switch shipped:** there is **no Replay**. This is a scope decision, not an API impossibility, since windows-capture's MPEG-2 stream output could in principle be demuxed. Holding raw frames is no fallback: 1440p NV12 is ≈ 4.98 GB per 30 s.

**Memory budget.**
- **What the ring holds:** at most *N* + 2 s, because one GOP of slack keeps a keyframe at or before "now − *N*".
- **Video size:** the configured bitrate. The table assumes 8 Mbps at 1080p30, 16 Mbps at 1440p30 and 24 Mbps at 1440p60 (YouTube's SDR upload recommendations).
- **Audio size:** 192,000 B/s of 48 kHz stereo 16-bit PCM.
- **Hard byte cap:** 1.25× the estimate. Past it the ring evicts whole GOPs, and the save reports the shorter length.

| Length | 1080p30 | **1440p30 (default)** | 1440p60 | Cap at default |
|---|---|---|---|---|
| 15 s | 20 MB | **37 MB** | 54 MB | 47 MB |
| 30 s | 38 MB | **70 MB** | 102 MB | 88 MB |
| 60 s | 74 MB | **136 MB** | 198 MB | 170 MB |
| 120 s | 145 MB | **267 MB** | 389 MB | 334 MB |

**Memory ceiling.** Arming is refused when the cap exceeds min(1 GiB, physical RAM ÷ 8), measured with `GlobalMemoryStatusEx`. The length picker greys out lengths over the ceiling and says why.

**Deliverables.**
1. **One pipeline, two consumers.**
   - **Ownership:** `ScreenRecorder` owns a `CapturePipeline`. It runs while a take *or* Replay needs it, and fans packets out to `TakeMux` and `ReplayRing`. The `divora-encoder` thread lives as long as the pipeline.
   - **Cost:** a take while armed costs one passthrough mux of bytes already encoded, with no second conversion or encode.
   - **Rung 1 survives only as a per-machine fallback for takes.** On such a machine the Replay row says *"Replay can't use this PC's video encoder."*
2. **Shared settings while armed.** A take uses the ring's settings: monitor, fps, quality, cursor, desktop source and desktop gain. `start_screen_recording` with different ones is refused with a plain message, and the card disables those pickers with the reason. Stems are still allowed.
3. **A take never contains anything from before you pressed Record.** It starts on a keyframe forced with `CODECAPI_AVEncVideoForceKeyFrame`. If the encoder will not force one, the take starts at the *next* natural IDR (≤ 2 s late), never at an earlier one.
4. **Encoder settings, set explicitly.**
   - **GOP:** `CODECAPI_AVEncMPVGOPSize` = 2 × fps. The documented default, 0, lets the encoder choose.
   - **Forced IDRs:** the encoder thread also forces an IDR when the input frame count since the last IDR reaches 2 × fps, keyed on the forced frame's input PTS.
   - **Keyframe checks:** only NAL type 5 counts as a cut point. If forced frames do not come back as IDRs within 3 frames of their input PTS, Replay is unavailable with *"This PC's encoder doesn't make regular keyframes."*
   - **B-frames:** `CODECAPI_AVEncMPVDefaultBPictureCount` = 0 is set before `SetOutputType`. If samples still show DTS ≠ PTS, arming is refused rather than written with negative DTS.
   - **Features:** `ICodecAPI` needs the `windows` 0.61 features `Win32_System_Com`/`Ole`/`Variant`. These are already compiled through tauri, so there is no compile cost, but `divora-capture` declares them.
5. **`ReplayRing`.** It is pure, lives in `divora-capture::replay`, and is owned by the encoder thread, so it needs no lock.
   - **Video entries:** access units `{pts, dts?, dur, key, seq: Arc<SeqHeader>, data: Arc<[u8]>}`, with times in 100-ns ticks. Each encoder sample is copied and released at once, and same-PTS slices are merged.
   - **Keyframes:** found from the bitstream (NAL type 5, 3- or 4-byte start codes).
   - **Sequence header:** taken from `MF_MT_MPEG_SEQUENCE_HEADER`, or parsed from the first IDR.
   - **Audio:** a preallocated ring of AudioTimeline's mixed 48 kHz stereo i16 PCM, (*N* + 3 s) long, with a 100-ns anchor for sample 0.
   - **Eviction:**
     - by time: drop the front GOP while the second IDR is at or before newest − *N*;
     - by bytes: drop the front GOP over the cap while two or more GOPs remain;
     - audio older than the front IDR is dropped.
   - **What clears both rings:** a pipeline restart, a sequence-header change, or **system sleep**. Sleep is detected by an explicit resume signal (`RegisterSuspendResumeNotification`). FramePacer and AudioTimeline then re-anchor instead of filling the gap. A clip never spans a discontinuity.
6. **Saving a clip.**
   - **The cut:** `save_replay` records *T*end. The encoder thread enters a non-blocking pending-cut state, polled each loop, until both rings pass *T*end, for at most 1 s. Voice audio lags video by its latency corrections, ≈256 ms with a non-streaming model. The thread never blocks.
   - **The window:** *T*0 is the latest IDR at or before cut − *N*. Audio runs from the sample matching *T*0 to the cut, and everything is rebased so *T*0 = 0. The MPEG-4 sink writes no edit lists, so this is the only way to align them without re-encoding.
   - **Snapshot:** `Arc` clones of the access units, plus a copy of the PCM window (≤ 23.4 MB at 120 s). A short-lived `divora-replay-save` MTA thread takes it.
   - **`Mp4Writer`:** shared with takes, over `IMFSinkWriter` (`MF_TRANSCODE_CONTAINERTYPE = MPEG4`, throttling disabled).
     - **Video:** passthrough, with one clean H.264 media type (including `MF_MT_MPEG_SEQUENCE_HEADER`) as both target and input. Every rebuilt sample carries `SampleTime`, `SampleDuration`, DTS where present, and **`MFSampleExtension_CleanPoint` = TRUE on IDRs**, so the MP4's sync-sample table is right.
     - **Audio:** PCM in and AAC out, loaded by the writer exactly as for a take.
     - **Ordering:** video and audio `WriteSample` calls are **interleaved by timestamp**.
     - **Loading:** every new mfplat free function (`MFCreateSample`, `MFCreateMemoryBuffer`, `MFCreateMediaType`, …) goes through v1.52.0's runtime loader.
   - **Files:** `recordings/divora-replay-<date>_<time>.mp4.part`, renamed when finalized. The `.part` file is deleted on failure.
   - **Refusals:** a save is refused below 1.1 × the snapshot size of free disk space (`GetDiskFreeSpaceExW`). One save runs at a time; a second press while saving shows *"Still saving the last replay."*
7. **Capture backend and consent.**
   - **WGC, the same capture as takes.** DXGI Desktop Duplication is not built:
     - its device must live on the adapter driving the monitor, which breaks hybrid-GPU laptops;
     - it shares a default limit of four duplicating processes with OBS, Discord and Teams;
     - the app may have to draw the cursor;
     - it loses access on mode switches and the secure desktop.
   - **Border copy:** the card says *"Windows draws a yellow border around this screen while Replay is on. Divora can't turn it off."* That border is also the one indicator visible while Divora is hidden to the tray.
   - **Off at every launch:** the armed flag is never persisted.
   - **Explainer first:** *every* arm path (card, hotkey, tray) checks that the inline explainer has been shown. It covers what is kept, the memory used, that nothing is written until Save, the border, and that a hardware encoder is needed.
   - **Saving while off:** a save hotkey or tray press while Replay is off is a no-op with the toast *"Replay is off."*
   - **Indicators while armed:**
     - a titlebar chip `REPLAY · 30 s` with the new `replay` Sigil, beside "LOCAL · NO ACCOUNT" (src/shell/Titlebar.tsx:67), static under reduced motion;
     - a tray tooltip suffix, plus the menu items **Save replay** and **Turn Replay off**;
     - the OS border.
   - **What "no disk" means:** an Architecture note states that Windows may page the ring to the pagefile, or write it to hiberfil.sys on hibernate. The promise is only that *Divora* writes nothing until Save.
8. **Who can arm it.**
   - **A hardware H.264 encoder** is required. Otherwise: *"Replay needs a hardware video encoder; this PC encodes with the CPU."*
   - **The memory ceiling** applies.
   - **The engine** must be running, with the same refusal message as the recorders.
9. **Lifecycle.**
   - **Device-loss rebuild:** the ring continues, with silence filling the gap.
   - **User stops the engine:** the audio goes silent and the ring is **held** (frozen, saveable). If a take is running, the pipeline keeps capturing for the take, as v1.52.0 defines. The next engine start clears the ring and resumes.
   - **Monitor lost or encoder failure:** held with a reason, still saveable, logged. It is not re-armed automatically.
   - **Quit or crash:** the ring is discarded and nothing is written. Changing the monitor means disarming and arming again.
10. **Wire surface.**
    - **Commands, all `async`:**
      - `replay_buffer_status` → `ReplayStatus`;
      - `arm_replay_buffer {monitorId, seconds: 15|30|60|120, fps?, quality?, cursor?, desktopAudio?, desktopGainDb?}` → `ReplayStatus` (`desktopGainDb`, as v1.53.0 named it);
      - `disarm_replay_buffer`;
      - `save_replay` → `{path, durationMs}`.
    - **Events:** `replay-buffer` (`ReplayStatus`, on every transition and at 1 Hz while armed) and `replay-saved`.
    - **`ReplayStatus` keys:** `supported`, `reason?`, `state` ("off"|"arming"|"armed"|"held"), `saving`, `seconds`, `bufferedMs`, `bytes`, `capBytes`, `monitorId?`, `heldReason?` ("engine-stopped"|"screen-lost"|"encoder-failed").
    - **Key:** `divora.replayBuffer` = `{seconds, monitorName?, explained?}`. It never stores an armed flag.
    - **Hotkey ids:** `rec:replay` (save) and `rec:replay-arm` (on/off), unbound by default.
11. **UI.**
    - **The card:** a Replay row on `ScreenRecordCard`, with:
      - an on/off switch;
      - the length with its memory estimate (*"≈ 70 MB of memory"*);
      - a **Save** button showing the buffered seconds;
      - the held or unsupported reasons.
    - **Settings → Recordings** gains a Replay subsection: the length (with per-option memory), two hotkey rows and the folder.
    - **The save toast** names the preset in use. There is no sidecar file and no preset in the filename.
    - **New Sigil:** `replay`.

**What's-new copy.**
- **Lead prose:** *"Replay keeps the last 15 to 120 seconds of one screen in memory, with your modulated voice, your soundboard and, if you pick it, game and Discord audio, and saves them as an MP4 when you press Save or its hotkey. It is off until you turn it on, Divora writes no video or audio to disk until you save, and Windows draws its yellow capture border around that screen while it runs."*
- **Added bullet:** *"A saved replay starts on a keyframe, so it can run up to about 2 seconds longer than the length you chose."*
- **Changed bullet:** *"Screen recordings and Replay now share one capture and one encoder, so recording while Replay is on does not encode the screen twice."*

**Tests.**
- **CI (pure logic, `divora-capture::replay`):**
  - **eviction:** once full, the duration stays in [*N*, *N* + 2 s + 5 frames]; the front is always an IDR; whole-GOP eviction by time and by bytes; a keyframe-less stream raises the fault;
  - **resizing:** the length shrinks and grows;
  - **segments:** the resume signal and a sequence-header change start new segments that no window spans;
  - **window selection** and the non-blocking cut with its 1 s timeout;
  - **alignment:** the audio start index matches the rounding formula exactly, and both spans agree to ±1 sample;
  - **snapshots:** later pushes leave a snapshot untouched;
  - **Annex B helpers;**
  - **frozen tables:** the memory estimate and ceiling;
  - **the consumer state machine:** a mismatched take is refused while armed; arming is refused with the engine stopped, without a hardware encoder, or over the ceiling; an engine stop holds the ring;
  - **`ClockFit` over a simulated 2 h at +100 ppm with jitter and steps:** the residual stays ≤ 20 ms, and it re-anchors at a GOP boundary if it saturates;
  - **serde freezes.**
- **CI (passthrough mux smoke, not ignored):**
  - **Encode:** the inbox H.264 MFT is driven synchronously over 96 synthetic 320×240 NV12 frames (GOP 30), into a 2 s ring.
  - **Save:** the ring is snapshotted and written through `Mp4Writer`.
  - **Parse back:** `ftyp`/`moov`, `avc1` + `avcC`, `stss` starting at sample 1, the sample count, and the `mdhd` durations.
  - **Early return:** if mfplat cannot load, `MFStartup` fails, or no H.264 MFT is enumerated. With no AAC encoder it checks video only.
- **Vitest and e2e:** card states, memory labels, the titlebar chip, `rec:replay`/`rec:replay-arm` registration and persistence, command-name freezes, and both events through `__E2E_EMIT__`.
- **Manual (MANUAL_TESTS, this machine):**
  - **Playback and sync:** the clip plays in VLC, WMP and an editor. The A/V offset is ≤ 40 ms after 5 min **and after 2 h armed**.
  - **Memory:** `ReplayStatus.bytes` ≤ `capBytes` across 2 h. The process RSS change is recorded separately, against a take-running baseline.
  - **Load:** CPU %, GPU "Video Encode" %, and the game's PresentMon present mode and frame times, with Replay off versus on.
  - **Saves and failures:**
    - saving during a take leaves both files whole;
    - hotkey and tray saves work while hidden to the tray;
    - an engine stop holds the ring;
    - unplugging the captured monitor;
    - sleep and resume;
    - a near-full disk is refused and leaves no `.part` file.

**Done when.** With Replay armed at 30 s, a game played for five minutes in the Villager preset, a sting fired and Divora hidden to the tray, the hotkey produces a clip that:
- appears within 3 s as a `divora-replay-*.mp4` of 30 to about 32 s;
- starts on a clean frame;
- contains the sting;
- shows an A/V offset of 40 ms or less.

The same holds after two hours armed.

**Not in this release.**
- **Bindings:** no MIDI or glyph action for Replay (bindings tail). The voice-timer release adds "save replay" as a timer action.
- **Capture and buffering:** no DXGI Desktop Duplication, no border removal, no arm-on-launch, no disk-backed buffers, no lengths over 120 s, and no saving part of the ring.
- **Settings:** no Replay settings separate from takes; no window capture.
- **Encoders:** no Replay on software encoders or on the kill-switch branch.
- **Trimming and automation:** no exact-length trimming (the MPEG-4 sink writes no edit lists), no auto-clipping, and no upload.

**As built (2026-10-17).**
- **The gate:** v1.52.0 shipped rung 1, so this release built the directly driven encoder (`EncoderSession`, async, D3D11-aware). The day-4 gate passed on "NVIDIA H.264 Encoder MFT", with decoding standing in for the plan's VLC and WMP playback (the files were not watched by eye): no dropped frames at 30 and 60 fps and at 1080p; every video frame decoded without error by ffmpeg (VLC's decoder), and every video frame and audio sample by Media Foundation (Windows Media Player's engine); the sync-sample table equal to the IDR frames; and the same bitrate as the sink writer on the same busy picture (11.50 against 11.55 Mbps).
- **Rung 1 is kept, not removed.** Takes go through v1.53.0's sink writer, with the same audio feed, on a PC with no hardware encoder, or after the direct encoder failed on this run (it couldn't be built when a take or Replay started, or it proved untrustworthy; remembered until the next launch). Replay then says why it can't run (*"Replay needs a hardware video encoder; this PC encodes with the CPU."* or *"Replay can't use this PC's video encoder."*). Any failure of the direct path's own parts (the encoder, its device and pool, the GPU converter) counts, even a passing one such as an encoder whose sessions another app holds — a take must never fail where v1.53.0's would have recorded; the price is Replay unavailable until the next launch. Found by the last claims audit, after the real-app runs. `DIVORA_PRETEND_NO_DIRECT=1` forces the second case for manual checks. `DIVORA_CAPTURE_RUNG1` and `DIVORA_CAPTURE_CPU_CONVERT` only send takes started while Replay is off to the sink writer: Replay still runs on the direct path, a take started while it is on goes on its pipeline, and turning it on during a sink-writer take is refused with *"Stop the screen recording first, then turn Replay on."*
- **No `CapturePipeline` object; the recorder keeps a slot.** `ScreenRecorder` holds a slot that is empty, running (with the pipeline's `PipelineHandle`) or ending. A direct-path take or an arm that finds it empty starts a new pipeline: its own `divora-encoder` thread, capture and encoder. A take started while Replay is on goes on Replay's pipeline with Replay's settings (or is refused in the user's words); an arm during a take joins the take's pipeline on its settings. As the plan says, the pipeline runs only while a take or Replay needs it. A request while the last one is on its way out waits up to 3 s for it.
- **Decisions are made by a pure core** (`PipelineCore`, driven in CI by a pretend encoder, engine and shell). The encoder thread writes no recording or clip: a take's file and each saved replay are written on threads of their own, which it hands messages without waiting (a writer too far behind ends the take). Its log lines are its only file writes. Beyond one loop's wait (20 ms or less) it waits only for the first picture at the start (up to 5 s); to drain the encoder (up to 0.5 s) when its output is wanted, or quiesce it (up to 0.1 s) when it isn't; while it builds a new encoder after a stream change, or a new session after a sleep; and at its end for the desktop-audio thread (up to 2 s).
- **The A/V offset was re-measured and its default (0 ms) did not move.** In one session on the day-4 gate build, before takes moved onto the shared pipeline, a take on the direct path measured +9.6 to +26.3 ms (mean +19.6 ms) and one on v1.53.0's sink writer −19.6 to +30.4 ms (mean +12.1 ms, 6 frames dropped), the click heard as desktop audio. The means are 7.5 ms apart, under half a 16.7 ms frame, so no regression is shown and none below a frame is ruled out. The final build's figures are below.
- **Owner decisions:**
  - a PC sleep ends a running take, its file kept — rung 1 too;
  - Replay uses the card's desktop-audio choice, not a setting of its own;
  - the on/off hotkey can't turn Replay on, and says so in a toast, until Replay has been turned on from the card's explainer (with desktop audio, if the card now records it: below);
  - the tray icon gains a red dot while Replay is on;
  - an encoder that can't be built again (after a sleep, or after it restarted itself) turns Replay **off**, replacing the plan's "held with a reason";
  - the release ships on a 26-minute armed run, not the plan's two hours, and without a real sleep: the sleep handling is covered by tests only (below).
- **Differences from the deliverables above:**
  - **Hotkey ids are `replay:save` and `replay:toggle`.** In the hotkey code `rec:` means "start or stop a recorder"; a save is neither, so Replay's two ids get a prefix of their own.
  - **`divora.replayBuffer` is `{seconds, explained, explainedDesktop}`.** No `monitorName`: Replay keeps the card's screen. Read with desktop audio off, the explainer must be read again before Replay keeps desktop audio, because that is when a clip can carry a call.
  - **`ReplayStatus` also carries `armed`** (the settings the ring runs with, so the card can lock its pickers to them and a take can ask for exactly them), and `replay_memory` reports the ceiling and each length's estimate, cap and whether it is allowed.
  - **`save_replay` waits for the clip (up to 90 s) and resolves with `{path, durationMs, error: null}`, or rejects with the reason.** Every attempt, refusals included, is also answered by exactly one `replay-saved` event, `{path, durationMs, error}`, and the toast comes only from the event. If the 90 s wait runs out, the command rejects as not saved; the event still carries the real outcome when it comes.
  - **The save toast names the preset only when the voice carried it for the whole clip,** and otherwise says "your voice as your call heard it": a clip may be shared, and calling the voice disguised could mislead. A held ring's clip is judged back from when it was held, not from the save.
  - **Memory:** the estimate is `(N + 2) · bitrate/8 + (N + 4) · 192 kB`, the cap 1.25× it. That is the plan's budget, which already counted audio, with two differences: 4 s of PCM slack where the plan had 3, and the bitrate a take would use at the chosen quality instead of the table's assumed one (14.4 Mbps at 1440p30 Balanced, not 16 Mbps). So the default, 30 s at 1440p30 Balanced, is up to 81 MB, not the table's 88 MB. The UI shows the cap rounded up and the ceiling rounded down, in MB of 10⁶ bytes.
  - **The cut waits up to 1.2 s, not 1 s,** for the frame showing at the press and its audio; audio still missing then is silence.
  - **A held ring is not cleared by the next engine start until the ring runs again,** and the soundboard playing just before an engine stop is in the clip (the timeline's last epoch is closed first).
  - **An engine "stop" is the user's, not `is_running()`.** A device-loss rebuild leaves the engine not running for its length, so the ring holds only on a user stop (`AudioEngine::user_stop_count()`), a recovery that gave up, or 5 s of not running. A device switch in Settings restarts the engine without a stop, so Replay carries on through it with silence in the gap; a switch whose new device won't open counts as a stop and holds the ring.
  - **Sleep:** detected by Windows' resume notification and by the performance counter running ahead of the interrupt time that leaves sleep out (`Instant` counts it). Each loop iteration, on both paths, reads its clock once, before the sleep check, and uses that reading for all its work. On a sleep a take ends at the previous check's reading, from before the sleep (`sleep::LoopSleep`), its file kept, and a pending Save is cut there. Nothing later reaches the timeline, so the sleep is not written as silence; a sink-writer take's finishing wait pumps no more than 2 s past its end. The ring is emptied; its session is built again at once, retried every 2 s for 30 s (then Replay turns off), and its clock starts again at the screen's first picture, however long that takes. A take can't start until then. A ring held for a lost screen or a failed encoder has nothing left to save and turns off. **None of this was tried with a real sleep:** it rests on tests of the pure core given the moment, of `LoopSleep` and of the bounded finishing pump, each checked by putting its defect back.
  - **The take's writer gives up** ("the disk couldn't keep up", file kept) when 4096 messages are queued, or once more than 64 MiB (67.1 MB) of encoded video is waiting.
  - **Quit waits for a save in progress** for as long as its size needs (5 to 60 s, plus the cut), then cancels it and waits up to 10 s more. A save still writing stops and removes its `.part`; one already finishing its file can't be stopped, and is kept if it finishes within those 10 s.
  - **The tray never turns Replay on.** Its menu gains **Save replay** and **Turn Replay off** while it is on.
- **Found in the real app and fixed:** a take or an arm that attached the engine's tap before the output callback had run since the last pipeline let go of it (a request waiting on that pipeline starts the moment it ends) had no voice or soundboard: the callback never saw the tap disarmed, so it started no new run and every buffer was dropped. Each attach is now a numbered take, stamped on every buffer.
- **Found by the claims audit and fixed:** the moment a sleep began was read at the end of the loop's last iteration, after its wait, which is where a PC most often goes to sleep. A sink-writer take across a sleep still got the sleep as silence, and the direct path built it in memory before cutting it off. Each iteration now reads its clock before the sleep check (above).
- **Measured** (MANUAL_TESTS → Replay) in a debug build of the shipping code before its last change (a fallback that only a failed encoder build reaches) (the "final build") unless noted: the 3440×1440 main screen recorded at 2560×1070, the engine silent, and a separate stimulus process flashing a square and playing a −40 dBFS tick on the SMSL DAC. The A/V figures are at 60 fps with Everything except Divora, the tick reaching the file as desktop audio; the voice and soundboard were not A/V-measured.
  - **A clip after 35 s:** the tick +1.3 to +18.0 ms after the flash (nine at +18.0 ms, one a frame earlier; mean +16.3 ms, 10 events); saved in 143 ms; 30.05 s long. Before the save the ring held 31.2 s and 34.7 MB against its 115 MB cap.
  - **A take while Replay was on:** +14.7 to +31.4 ms (seven at +14.7 ms, one a frame later; mean +16.8 ms, 8 events).
  - **Whole files** (7: the clip, the take, a save during it, a held ring's save, a take after Replay was turned off, a save from the card's button, the fallback's take): the keyframe table equals the IDR frames and starts on one, no edit list, no composition offsets, BT.709 limited range (`colr` and ffprobe). Media Foundation decoded every video and audio sample with no gap over 1 ms and no error, and ffmpeg decoded each without an error.
  - **Also passed:** five save attempts (Replay off, arming, two at once, one just before Replay was turned off) answered by five `replay-saved` events; a take started right after another, and a ring armed right after Replay was turned off, each with 3 of 3 soundboard stings; a Stop sent 0, 40, 250 and 900 ms after a start ending the take, and Replay turned off 0, 30, 200 and 600 ms into its start ending off; 20 × (on, 5 s, off), with private memory read 2.5 s after each off at 102.0 MiB after the first and 104.2 MiB after the last (49 to 130 MiB in between), and no display-driver event; the fallback (`DIVORA_PRETEND_NO_DIRECT=1`): a take through the sink writer, then Replay refused with its reason (the take's A/V not measured: other audio was playing); a disk pretended nearly full (`DIVORA_PRETEND_FREE_BYTES=1000000`): the save refused, nothing written, Replay still on.
  - **26 minutes armed, not 2 hours,** on the build before the review's fixes (2026-10-16): the run ended there, when Divora closed with nothing in its log and no Windows crash record (whether it was closed or crashed is not established), and the owner chose to ship on it. Replay stayed on throughout, held 30.1 to 32.0 s and at most 34.7 MB against its 115 MB cap, and the process's private memory stayed between 178 and 195 MiB (read once a minute). One log line in the 26 minutes, for a save at 5 minutes, whose A/V was not analysed.
  - **Not measured:** two hours armed, and the A/V after the first minute armed (the clip saved at 5 minutes wasn't analysed); the A/V of a take with Replay off on the final build (above, on the gate build only); the "done when" game session (a game's frame times, CPU and GPU "Video Encode" with Replay off versus on); a real sleep and resume (above); a monitor unplugged; an encoder failure; switching the output device; a save from a real hotkey press (every save was made through the `save_replay` command or the card's button, the one while hidden to the tray included); the tray's red dot and its menu (only its tooltip was read, as the tray button's name, and no menu item was clicked); Quit during a save; a really full drive; playback by eye in VLC, WMP and an editor; the memory change against a take-running baseline (only the process's absolute private memory was recorded); Narrator. The sleep, unplug, device-switch, tray, hotkey and Quit cases are owner checks in MANUAL_TESTS.

### v1.55.0 — Charms: pictures and GIFs on your overlay (6–9 days)

**Goal.** Add a picture or GIF to the overlay, drag and resize it beside the spell circle in **Arrange**, then **Lock** it so clicks pass through to the game. The arrangement survives a restart, and OBS Window Capture shows it with alpha.

**Deliverables.**
1. **Rust creates the window.** `open_overlay` creates it `visible:false`, with `shadow:false`, `skipTaskbar`, and `set_ignore_cursor_events(true)` before the first show. It is shown only after `overlay:ready`. A 10 s watchdog destroys an overlay that never painted, because an unpainted non-click-through topmost window is an invisible click trap.
2. **Arrange / Locked modes.**
   - **Command:** `set_overlay_mode`.
   - **Ways out of Arrange:** Esc, the Settings toggle, a tray item "Lock the overlay", and a new hotkey id `overlay:arrange` (an `overlay:` prefix row in STABLE-SURFACE).
   - **Arrange chrome:** a tinted frame and the hint "Esc locks the overlay".
3. **Scene model.** `divora.overlayScene` = `{version:1, designSize:{w,h}, elements:[{id, type:"spellcircle"|"image", x,y,w,h as 0..1 of designSize, z, src?, fit?}]}`, with unknown fields preserved.
   - **Images** keep their natural aspect and anchor by centre, so a window resize or Fit to monitor re-anchors them without stretching.
   - **Edits** go through a pure reducer in src/overlay/scene.ts: add, remove, move, resize, z-order, nudge and clamp.
4. **Editor.**
   - **Input:** Pointer Events with `setPointerCapture`, never HTML5 drag-and-drop.
   - **Controls:** four corner handles, aspect lock, arrow nudge and Delete.
   - **Sync:** `overlay:edit` is emitted on pointerup only.
   - **Inside the overlay:** no native `<select>` and no `title` tooltips, because WebView2 popups escape capture exclusion.
5. **Assets.**
   - **Import:** `import_overlay_asset {path}` copies a dialog-picked PNG/JPEG/GIF/WebP/APNG into `app_data_dir()/overlay/assets/<sha256[..16]>.<ext>`. It enforces a magic-byte allowlist, a 50 MB cap and path confinement, with a soft warning above 20 MB.
   - **Delete:** `delete_overlay_asset`.
   - **Asset protocol:** tauri's `protocol-asset` feature with `assetProtocol {enable:true, scope:["$APPDATA/overlay/assets/**"]}`. The scope uses the variable because the real directory is `com.divora.voicemod`, and the resolved path is logged at startup.
   - **Hashing:** `sha2 = "0.10"`, pinned. 0.11.1 is current and would add new lockfile versions.
6. **Fit to monitor**, off by default. It sets the `NonRudeHWND` window property before showing, so the shell does not treat the window as a fullscreen app.
7. **Hide the overlay from recordings.**
   - **The toggle:** off by default, covering takes *and* Replay while either runs.
   - **How:** `set_content_protected(true)`, applied **only while Locked**. Locked makes the window layered, which Windows 10 needs for exclusion to work.
   - **Keeping it on:** it is re-applied after every show and mode flip. The window is never hidden and re-shown during a protected capture.
   - **The copy says** it may show as a black box on Windows 10, and that it hides the overlay from OBS too.
8. **Settings → Stream overlay** gains:
   - the charms list, Add picture, Arrange / Lock, and Fit to monitor;
   - the copy *"If you use OBS, add images in OBS. Charms are for the window OBS captures and for recordings without OBS."*
9. **New Sigils:** `image`, `unlock` and `expand`.

**Tests.**
- **Vitest:**
  - the reducer, exhaustively: clamp, aspect, re-anchoring, z-order and migration;
  - the existing reading-free source scan, which now covers scene.ts automatically.
- **Rust:**
  - the asset allowlist, id derivation and traversal rejection;
  - the mode machine: it cannot leave Locked before the page has painted (in Rust, not Vitest: the machine lives in the backend).
- **E2E:** controls render with monitors → null.
- **Manual:**
  - click-through over a game and over OBS;
  - GIF animation;
  - idle GPU %, dwm.exe, and the game's PresentMon present mode, comparing the 480×480 window with the fitted one, static with GIF, on 4K. If the fitted window is costly, shrink it to the elements' bounding box in this release.

**Kill switch.** If click-through over WebView2's child window fails on the current runtime, Locked degrades to "hide while not arranging". OBS users are unaffected.

**Not.** No text or video charms, rotation, snapping, per-monitor windows or timer face yet.

**As built (2026-10-18).**
- **No separate spike: S0 was folded into the real-app run (S2),** on this machine (Windows 11 Pro 26200; 3440×1440 and 1920×1080 screens, both at 100 %; an NVIDIA GPU), in two debug builds of the `charms` branch (a local branch, since squashed into `42a289d`) driven over WebView2's DevTools protocol and read through Win32 — no game, and no real mouse or keyboard input. R1's and R2's checks ran on the first build, before S2's fixes; R4's and R7's on the second, with the late-answer fix (below) and the review's fixes (MANUAL_TESTS → Charms says which run used which). What it settled:
  - **R1, click-through:** rung 0 held — tao as is (`R1_RUNG = 0`). Locked, the window was `WS_EX_TRANSPARENT|WS_EX_LAYERED` before it was first seen, `WindowFromPoint` passed through it at 6 of 6 points (centre, edges, corners, an inner point); the circle drew Locked, and a GIF drew and animated, Locked and arranging. Not tried over a game.
  - **R2, hiding:** it works on this machine, by decoding: a 10 s take with Hide on had the overlay in 0 of 302 frames, a 5 s take with it off in 152 of 152, and a Replay clip saved 4.3 s after arming in 0 of 130 from frame 0. The toggle ships.
  - **R4, the fitted window's cost:** the idle-GPU half of the gate passed — fitted added about 0.1 percentage point of 3D-engine time over 480×480 (the gate was 2) — so the bounding-box fallback was not built. The present-mode half (PresentMon over a borderless game) was not measured.
  - **R3, Windows 10 / Windows 11 21H2:** pending, on the owner's two-order check on the other machine. Until then the toggle is offered from `UniversalApiContract` 10 (Windows 10 2004, `MIN_CONTRACT`), and the CHANGELOG says the outcome there is unknown and the gate decision pending.
  - **R7, memory:** the three thresholds set beforehand passed with a scene filling both pixel budgets (four animated 2048×2048 GIFs of 20 frames and three 4096×4096 PNGs; extra copies of a picture, which the budget counts once, weren't added), so the designed limits stay (the figures are in the CHANGELOG). It was drawn large for 30 s, not 5 minutes, and not over a game, and the renderer's and GPU process's CPU weren't read.
  - **Not settled:** whether the browser keys stay blocked and Alt+F4 closes the overlay under real keys (S0 f, R16); the hotkey's foreground success in a game (S0 e); a GIF whose first frame is larger than its screen, or whose screen is 0×0, served to WebView2 (S0 d's fixtures; their sizes are unit-tested); a WebP whose frames are larger than its canvas (not tested); whether rAF runs in a hidden overlay (S0 c, informational).
- **Differences from the deliverables above** (the design's D1–D22):
  - **D1:** pictures are served by Divora's own `charm` scheme (`http://charm.localhost/<id>`), not `protocol-asset`: to the overlay only, at most two at a time, 503 once it is closing; Settings shows no thumbnails. The log names the assets folder at startup; `pnpm tauri dev` builds use `overlay/assets-dev`, while a `pnpm tauri build --debug` build, like S2's, uses `overlay/assets`.
  - **D2:** the backend builds, places and shows the window, but main still computes the rectangle (`placeOverlay`, through `planFrame`). The window also gets `minimizable(false)` and a 64×48 minimum size.
  - **D3:** the page's ready and painted signals are one app command (`overlay_handshake {instance, stage}`), not Rust event listeners, which tauri parks on lock contention. The window shows at ready, Arrange waits for painted, and the watchdog closes a window that never readied within 10 s, not one that never painted.
  - **D4:** the backend owns the mode in a pure machine and reports it on a new event, `overlay:status` (and a new command, `overlay_status`). Two more ways out of Arrange: the chrome's Lock button and the lease. The test that the machine can't leave Locked early is Rust (`cannot_arrange_before_painted`, `machine_table_is_exhaustive`), not Vitest.
  - **D5:** main owns the scene and pushes it on `overlay:scene` with a session, a revision and an ack; edits come back on `overlay:edit` as `{instance, seq, op}` (the plan named the event, not its payload).
  - **D6:** positions are fractions of a design size that only grows as you arrange (a Fit change sets it to the new window's or monitor's size), centre-anchored; an element keeps its size and only shrinks to fit. Images store `nw`, `nh`, `anim`, `bytes` and `name`, and the scene its `frame`; the plan's per-image `fit?` is dropped, and the reducer gains `reframe`.
  - **D7:** aspect is always locked, with no control to unlock it. From the keyboard, **=** and **-** resize it a little, **+** and **_** by 10 % (on a US keyboard that is Shift with = and -); the numpad's + and − resize it a little.
  - **D8:** Hide is a request, with no status key claiming it. The affinity is set only once the window is being shown (the first time just before it shows, with no message pump in between), then only when it changes, never on a closing one, and last on the way to Locked (an unmeasured hedge); the recorder asks before a capture starts (`before_capture`, whose `CaptureCleared` token `open_feed` takes). The plan's "Locked makes the window layered, which Windows 10 needs" (deliverable 7; the Decisions row says the same) doesn't hold: on 19045/22000 the click-through Locked adds is the suspected trigger of Electron #47834. The copy says "may still show", not a black box.
  - **D9:** Fit counts only with the Transparent background; the fitted window isn't resizable; the scene records its frame and is reframed at the next open if Fit changed while closed; geometry isn't saved while fitted; the fit keeper re-fits after a scale change and checks every 5 s for a resolution change.
  - **D10:** caps of 8192 px a side, 16,777,216 px and 4,194,304 px animated, besides 50 MB (10⁶ bytes); only 8-bit Huffman-coded JPEGs, sequential or progressive (SOF0/1/2); sizes as displayed (a GIF's canvas grown to hold its first frame, EXIF orientation). "Path confinement" is restated: any regular file the user can read is accepted, and the stored file's name is derived from its content, never from the path.
  - **D11:** `overlay.json` grants five permissions (event listen, unlisten, emit and emit-to; start-dragging). It doesn't cover app commands, as the Architecture section now says.
  - **D12:** the window moves only in Arrange, unfitted, from the **Move window** grip or an empty spot, which reverses v1.51.3's "press anywhere".
  - **D13:** the overlay's circle is decorative (no tooltips, focus or pointer on its nodes), laid out square from `SPELL_CIRCLE_PX` (440), with a round hit target; at most one is shown, and it can be removed.
  - **D14:** 16 pictures, 64 stored elements, a 24 px minimum side and hit box, and a budget over distinct pictures (200 MB of files, 16,777,216 animated and 67,108,864 total pixels), all applied when adding, never when reading.
  - **D15:** the tray's **Lock the overlay** (`overlay-lock`) is shown only while arranging, in the plan's order: Show · Save replay · Turn Replay off · Lock the overlay · Quit.
  - **D16:** the hotkey `overlay:arrange` toggles Arrange and Locked, acted on in Rust inside `WM_HOTKEY`; JS only shows a toast while the overlay is off or starting.
  - **D17:** the overlay blocks every key but Tab, Alt+F4, and Enter or Space on a button (`shouldPreventKey`), and its context menu; it has no `<select>` and no `title`.
  - **D18:** the kill switch is a ladder — rung 0 tao as is and rung 1 an opaque layered window (both in code, chosen by `R1_RUNG`), rung 2 hidden while Locked (designed, not built) — and rung 0 shipped. The plan's "OBS users are unaffected" is corrected: under rung 2, OBS Window Capture would have nothing to show.
  - **D19:** the boot reveal finds the main window by its label.
  - **D20:** the main window's overlay code moved to `src/overlay/controller.ts`, tested with its dependencies injected.
  - **D21:** the Arrange lease: click-through goes off only at the page's first beat (`overlay_alive`, every 500 ms), and three silent 1 s ticks lock the overlay again, with a toast.
  - **D22:** focus is taken with plain `SetForegroundWindow`, never tao's `set_focus` (which presses Alt when Windows refuses), and handed back on Lock to the window that had it before Arrange, if the overlay still has the foreground.
- **Smaller changes while building:** the picker also offers `.jfif` and `.jpe` (the import goes by the content); a status that keeps the mode doesn't move the keyboard in the overlay; main's overlay actions never reject: they catch the failures they expect, showing most as toasts, and log anything else.
- **Owner decisions:**
  - the `overlay:arrange` hotkey does nothing but show a toast while the overlay is off;
  - the spell circle can be removed, and added back (**Add the spell circle**);
  - the caps: 16 pictures, 50 MB, 16 megapixels and 8192 px a side;
  - Hide keeps the overlay hidden while Replay is held, not only while it keeps;
  - at the design's defaults: the 4-megapixel cap on animated pictures, the scene budget, the tray item only while arranging, hiding whatever screen Divora records (R15), no refusal of long, thin pictures, and no single-pointer Smaller/Bigger buttons (the keyboard resizes).
- **Found in the real app and fixed:** after a quick close the Settings toggle could stay on with the window gone (6 of 12 quick cycles): `close_overlay`'s answer ("closing", open) arrived about 10 ms after the "closed" event and was applied last. Main now applies a command's answer only if no `overlay:status` arrived while it was in flight (the backend emits one on every change); 20 of 20 cycles after.
- **Seen once, not reproduced:** Replay held with "the video encoder stopped returning frames" about 1 s after an overlay opened, 2.5 s after arming; not seen in 26 further rounds (156 opens and closes with Replay on, 4 of the rounds right after a fresh launch). Cause unknown; Replay behaved as designed.
- **Not measured** (owner checks, or not possible here): Windows 10 and 21H2 (R3); what OBS shows, of the overlay's alpha and of a hidden overlay; Discord, Teams and the Snipping Tool while hidden; real clicks and the scroll wheel through the Locked overlay (only `WindowFromPoint` was checked); real keys; the hotkey (none was pressed, in a game or anywhere) and the keyboard going back to the window that had it; the tray item; the ways out of Arrange and the lease while fitted, and the lease after a renderer crash (`Page.crash`); Win+D and Win+M during a protected take; the taskbar and notifications while fitted; a scale or resolution change while fitted; pictures holding still as Fit turns on or off (only the window's rectangle was checked), and a picture dragged to a far corner while fitted, then pulled back in as Fit turns off; mixed DPI; PresentMon; a 4K screen; a long Replay with Hide on; a GIF with Motion on Functional; the circle's size after an edge resize, and a removal, across a restart; the watchdog with Hide on during a take; memory drawn large for 5 minutes, over a game, and the renderer's and GPU process's CPU; a 12-bit or arithmetic-coded JPEG refused in the app (unit-tested only); moving the window (with **Move window** or an empty spot) or resizing it by its edges (only a charm was dragged); opening the overlay during a capture with Hide off, watching for an empty rectangle while it starts (only Hide on was checked); Fit switched during a protected capture; Green and Magenta disabled while fitted; Settings' "Starting…" label.
- **Follow-ups:**
  - **Monitor-aware hiding (R15).** Hide applies whatever screen Divora records, so with the overlay on a second monitor that OBS captures and Replay on the first, OBS loses the overlay for the session although Divora's recording never contained it. Protect only while the overlay is on the recorded monitor: a pure factor fed by the recorded monitor id (`StartRequest.monitor_id`, `SharedSettings.monitor_id`) and the overlay's cached rectangle, protecting whenever either is unknown.
  - **R3's outcome:** if the common path shows the overlay on Windows 10, raise `MIN_CONTRACT` to 15 (Windows 11 22H2), so the row is hidden below it.
  - **R4's present-mode half,** over a borderless game; the bounding-box fallback if fitting changes the present mode.
  - **R16,** if real keys get through: turning WebView2's browser accelerator keys off, which needs `webview2-com` and an owner decision.
  - **The encoder stall above,** if it is seen again.

### v1.56.0 — Mute that mutes (2–3 days)

**Goal.** Turn the cosmetic mute into a real one, so the BRB timer and every mute binding silence the call while the soundboard keeps playing.

**Deliverables.**
1. **Engine.** An `input_muted: AtomicBool` zeroes the **voice after loudness, before the soundboard fold**, with a fixed 256-sample ramp. Zeroing before the chain would let voice-convert, reverb and echo tails keep talking. `AudioEngine::set_input_muted` also drives the existing Voice-reading mute.
2. **Wire:** `set_input_muted {muted}` and an optional `LevelUpdate.inputMuted`.
3. **Bindings:** a `mute` hotkey id (persisted in `divora.hotkeyBindings`) and a `mute` MIDI action kind. The glyph mute already exists.
4. **Store:** an idempotent `setMuted(bool)` beside the toggle.
5. **Titlebar pill:** goes loud while muted.
6. **STABLE-SURFACE ruling.** The persisted glyph `{kind:"mute"}` keeps its name and now does what it always claimed to do. That fulfils its documented meaning rather than repurposing it. The Changed bullet says plainly that mute was cosmetic before.

**Tests.**
- **Allocation-free:** the callback stays allocation-free with mute toggled mid-run.
- **Silent send:** the send is exactly zero after the ramp while a soundboard voice passes.
- **No click:** no step is larger than the ramp slope.

**Done when.** A Stream Deck key mutes the call while a clip stays audible, and unmuting does not click.

**As built (2026-10-20).**
- **The real-app run (S2),** on this machine (Windows 11 Pro 26200, i7-14700K), in one debug build of the `mute` branch (`72ee4e3`), driven over WebView2's DevTools protocol, the mute commands sent from the page through the real IPC except R7 (real `WM_HOTKEY`, from `SendInput`) and R9 (CDP clicks on the real button). The engine was silent: the Focusrite in at 48 kHz, the Realtek speakers out with Monitor off and no monitor device, nothing played into the cable. The voice was a 440 Hz tone (660 Hz for R4) at −30 dBFS on the SMSL (owner-approved, ≤ 30 s a phase), which the Focusrite heard at only about −73 to −80 dBFS: the analyser's automatic edge finding failed in R1–R4, and the lead measured those recordings directly. What it settled:
  - **R1, the call's audio** (the send's WAV, a 1 kHz −20 dBFS clip playing, 10 toggles 2.5 s apart): mute works and the clip is untouched — the tone at −79.8 to −81.0 dBFS unmuted and −168.9 to −269.3 dBFS muted (≥ 88 dB down; the line was 70 dB), the clip at −20.0 dBFS in every stretch.
  - **R3, the voice alone:** every muted stretch 100 % exact-zero samples (5 of 5); silence began 0–11 ms after each command was sent.
  - **R2, the click:** supporting evidence only. With room noise as the signal, the 256 samples before silence were 0.38–0.94× (mean ≈ 0.63) the steady RMS, near a 256-sample linear fade's 0.58 and well below a hard cut's 1.0; at unmute 0.38–0.81×. The planned second-difference measure couldn't run on so weak a tone; the strict bound is the unit tests' (`no_step_larger_than_the_slope`, `the_send_is_a_pure_linear_ramp`).
  - **R4, speech said while muted, through an echo:** not measurable here. The trial passed, but its positive control failed (the burst reached the mic too weakly to make a measurable echo), so it proves nothing. Covered by `what_is_said_while_muted_never_reaches_the_send`, which fails with the starve switched off.
  - **R5, a take across two restarts:** the voice stem exactly 0.0 from 2.05 s (mute at 2.006 s) to 6.80 s (unmute at 6.79 s), straight through both restarts, from the lead's 50 ms map of the stem (the analyser's window was misaligned). The take kept recording; its `audioLostAt` at 5.03 s, cleared by the end, is the recorder's restart behaviour, not the mute.
  - **R6, Replay:** the tone 68.8 dB down in the saved clip's muted stretch (the line was 50 dB).
  - **R7, the real hotkey** (Ctrl+Alt+Shift+F24 four times by `SendInput`, about 480 ms apart, the overlay open): seq 72 → 76, +1 per press, never +2; each press in its own tick 12–25 ms later; the pill and the overlay's circle Muted, Clean, Muted, Clean. The hide-to-tray step left main visible, so "while hidden to the tray" is not shown.
  - **R8, a reload while muted:** the engine kept its seq, so the boot neither unmuted nor toggled it; the overlay received only "muted"; the pill read Clean at 51 ms and Muted at 174 ms. "`input_mute` before device enumeration" rests on the e2e and unit tests: the harness's IPC log didn't survive the reload.
  - **R9, 40 clicks 0–32 ms apart:** one `set_input_muted` per click, each sending what its label said; every animation frame showed the newest state received before the previous frame.
  - **R10, the real commands:** `{muted:true}` twice → `{true, 67}` both times (seq unchanged the second time), `{muted:false}` → `{false, 68}`, `input_mute` → `{false, 68}`; ticks agreed within 6–23 ms.
  - **B1, a long mute's CPU** (not in the app: the release-mode bench `a_long_mute_costs_at_most_twice_speech_or_a_pause`, the voice stage alone on a test thread that flushes as the callback does; every bundled preset, loudness on, 180 s muted against 10 s of a 220 Hz tone standing in for speech and 180 s of −70 dBFS room tone): all 24 pass with the flush below, the last 10 s muted at 0.68–0.99× max(speech, pause), against a bar of 2×; without it 21 of 24 fail, at up to 15.9× max(speech, pause).
- **Differences from the deliverables above** (the design's D1–D23):
  - **D1:** the plan's "after loudness, before the soundboard fold" is not one point: four consumers sit between them — the reading's wet tap, the screen recorder's tap, the monitor's copy of the voice and, with no monitor device, the previews mixed into it. The mute sits at the earliest, inside `mix_voice_and_soundboard` right after `loudness.process` and before the soundboard renders, so every recorder matches the send and the soundboard and the previews are untouched.
  - **D2:** the chain's input is starved too (`feed`, before the modulator). Zeroing after loudness alone keeps the effects on the live mic, and at unmute they release what they held of words said while muted: up to about 256 ms from Voice Convert, seconds from an echo.
  - **D3, D4:** one position on a two-segment linear path. Muting closes the send over L, then the chain's input over L; unmuting opens them in the other order; a toggle mid-fade turns round. L is the power of two at or above 5 ms, 64 to 2048: 256 at 44.1 and 48 kHz, as planned, but 512 at 88.2 and 96 kHz, 1024 at 176.4 and 192 kHz, 128 at 16 to 24 kHz and 64 at 8 to 12 kHz. A gain of 0 writes `+0.0`, a gain of 1 leaves the sample alone. The cost, stated in the CHANGELOG (owner Q10): what the chain still holds at the press is cut with the send — 10–21 ms for each denoiser, pitch or formant effect, added up, and about a quarter-second more with a voice model loaded in Voice Convert (its downsampler's 4096-sample rounds, streaming model or not).
  - **D6–D9:** the state is one engine-lifetime `AtomicU64`, `seq << 1 | muted`, not an `AtomicBool`; every change bumps `seq`, a set to the value it has doesn't. The setters are direct atomic ops, never the command channel, so a mute isn't queued behind a Start or a device-loss rebuild on the engine thread. The commands and the hotkey's `WM_HOTKEY` run on the main thread, though, so they wait while `start_audio_engine`, a sync command, holds it opening a device. Never stored; every launch starts unmuted; each session's gate starts at the word.
  - **D10:** `toggle_input_muted` and an `input_mute` read beside the plan's `set_input_muted`. Labelled controls (the Mixer button, the glyph by its omen, a MIDI CC) set the state they name; unlabelled triggers (the hotkey, a pad) toggle. The store keeps a read-only mirror, the largest `seq` from answers, ticks and one read at creation; `UiState.muted` is gone.
  - **D11:** the hotkey is acted on in Rust (`mute_on_hotkey`, inside `global_shortcut_handler`), on press only; JS ignores the event. `mute:` is reserved.
  - **D12:** a MIDI CC acts on crossings — 64 or more mutes, 59 or less unmutes, 60–63 keeps the side — so a fader resting low can't undo a hotkey mute and a knob near halfway doesn't flap. The first message after load acts only when up; a mapping's side is forgotten when it changes and, for every mapping, when the MIDI input is picked. A pad toggles.
  - **D17:** `set_input_muted` doesn't drive the reading's mute: the reading worker reads the word once per pass and stamps `mutedInDivora` on the snapshot it publishes, and the panel keys **Muted in Divora** off that, not the store's mirror. `set_reading_muted` stays the device fact, with no caller.
  - **D18:** `LevelUpdate` carries `inputMuteSeq` beside `inputMuted`, on every tick.
  - **D19–D21:** the pill loud and static, with a polite live region for the mute's edges only; a **Mute** / **Unmute** button on the Mixer that sends what its label says (D20); copy that names the call, your headphones and recordings, never "microphone", and Monitor's copy corrected for a single output.
  - **D22:** the allocation test covers the voice stage, not the callback (Voice Convert's inference still allocates).
  - **D23:** the overlay's `overlay:state` pushes are chained one at a time, and held until the store knows the mute (500 ms at most).
  - **The tray left out** (D14, owner): no item or tooltip.
  - **K16, not in the plan: subnormals.** A long mute feeds the chain exact zeros, and its recursive states decay into subnormal floats; B1 found 21 of 24 presets at 2–16× the CPU of speech or a pause. The design's contingency, feeding the starved chain an alternating ±1e-18, was tried and reverted: the noise gate 23 of the 24 presets start with took it down 60 dB, and loudness's `x * x` average and the STFT magnitudes squared that under the normal range. Instead the output callback sets FTZ|DAZ in MXCSR first thing, every buffer (`divora-core/src/audio/denormals.rs`, x86-64 only, a no-op elsewhere): `divora-core`'s first production `unsafe`, an `asm!` block of `stmxcsr`, `or` and `ldmxcsr`. Rust formally treats a changed float environment as undefined behaviour; the expected effect is that a float the compiler folded at build time can keep a subnormal that the thread would flush. `feed` writes exact `+0.0` again.
- **Owner decisions** (2026-10-20):
  - your voice goes silent in the headphone monitor too; clips and Speak still play there (Q1, D16);
  - push-to-modulate, which is cosmetic as well, gets its own small release right after this one, before Hourglass; v1.56.0 claims nothing about it (Q6);
  - nothing new shows the mute while Divora is hidden: no tray item or tooltip (Q2);
  - S2's 440/660 Hz tone at −30 dBFS on the SMSL (never the cable, up to 30 s a phase) and Ctrl+Alt+Shift+F24 sent four times by `SendInput` (Q8, Q9);
  - at the design's defaults: CCs act on crossings (Q3); hold-to-mute and separate on and off keys later (Q4); the mute never remembered (Q5); the Voice Convert cut accepted and stated (Q10). Q7's default, a toast when Windows refuses a hotkey, was not built (follow-ups).
- **Found by the code review and the bench, and fixed before S2:** the subnormals (above); the Mixer's button kept focus after a mouse click, so Space, the in-app push-to-modulate key, pressed it again (a pointer click now blurs it; a keyboard click keeps focus); a mute CC kept its side across a change of MIDI input, so a fader resting low could undo a hotkey mute; the monitor-device note was wrong with no monitor device (it now says what Monitor does with and without a separate one), and the MIDI intro overclaimed what a CC does on its first move (it now says its first move after launch, a device pick or a mapping change mutes if it is past halfway). Two tests were tightened: the needle pins the gate's construction outside the callback's closure, and the hotkey's mute step (`mute_on_hotkey`) runs against a real engine, with a needle that the handler passes it the real press state.
- **Done when** rests on M1 (a Stream Deck key in a real Discord call, a clip heard while muted, no click at unmute), not run yet; R1 and R7 stand in for parts of it.
- **Not measured** (S2's gaps; the owner checks M1–M9 in MANUAL_TESTS → Mute haven't been run yet): real speech, and a voice as loud as speech at the mic — the click (R2's room-noise evidence only) and the echo leak (R4) rest on unit tests; the hotkey while hidden to the tray (R7's hide step didn't take) and a Stream Deck **Hotkey** action (M1); the headphone monitor (M2); MIDI (M3); a glyph (M4); a real unplug (M5; R5's rebuilds were direct restarts); Voice Convert (no model loaded) and its end-of-word cut (M6, M6b); Voice reading in the app (M7); Test my setup's muted message in the app; screen readers (M8); the light theme (M9); a game; OBS's own mic source; the far end of a Discord call (no call was live); a Bluetooth or high-rate device (only 48 kHz ran) and the device signal after the resampler when the rates differ (the WAV is taken before it); the overlay's Muted on the stream with main hidden; a mute held for minutes in the real app (B1 covers the CPU only); a take's MP4 audio (R5 measured its voice stem); Space after a mouse click on the Mixer's button, and the button from the keyboard; B1 on other CPUs; Windows 10.
- **Follow-ups:**
  - **Push-to-modulate's own release, next.** *(Done in v1.58.0, below, after v1.57.0 shipped the Voice Convert fixes.)* The release order changes: it comes before Hourglass (v1.59.0 below, planned as v1.57.0). Push-to-modulate is cosmetic too: its key changes the status, not the sound, so the pill can say Clean while the call hears the preset. What still claims a bypass: the app's own copy (Settings → Hotkeys: "Hold to apply the chain. Lifts back to clean."; the Mixer's Push to modulate card: Hold to apply / Hold to bypass, APPLYING / BYPASSED), README.md's feature list, MANUAL_TESTS' Voice reading check "Push-to-modulate with the key up" and its Hotkeys survive a restart check ("the voice modulates"), and comments in `dsp/reading.rs`, `dsp/reading/analyzer.rs` and `examples/voice_reading.rs` (this release corrected engine.rs's).
  - **Keep the last words at the press** (K17): needs `VoiceConverter::latency_samples` to report its downsampler's chunk (about 256 ms, not the 13 ms it reports for the shipped streaming model), which also moves the latency readout and the take timeline's compensation; then the send's close can wait for the chain's real delay.
  - **Hotkey conflicts and refusals** (K4, Q7): a hotkey Windows refuses looks bound but does nothing (it is only logged); a toast for the row being set when its register fails.
  - **`mute:hold` / `mute:on` / `mute:off`** (Q4), under the reserved prefix.
  - **A tray mute,** a tooltip (about 2 hours, no menu change) or an item (about half a day, a frozen-menu change), if the owner wants one (Q2); it fits v1.60.0's BRB.
  - **v1.60.0's "restore what it was"** needs a compare-and-set on the word inside `AudioEngine` (no wire change); the setters already return `MuteChange {before, after}`.
  - **A latching MIDI button out of step** after another surface changed the mute (one dead press): a "pickup" mode if owners ask.
  - **The flush is x86-64 only,** and B1 has run on this machine's CPU only.

### v1.58.0 — Push-to-modulate that modulates (added after v1.56.0)

**Goal.** Make push-to-modulate real: its key, a MIDI pad and the Mixer's button switch your effects in the engine, so the call, the headphone monitor, the WAV, screen takes and Replay hear what the status says. Through v1.57.0 its key changed only the status (v1.56.0's follow-up above): in the default **Hold to apply**, the pill said Clean while the call heard the preset.

**Why here.** The owner's v1.56.0 decision (Q6): its own small release right after the mute, before Hourglass. v1.57.0 (Voice Convert keeps up, outside this plan) shipped first, so this is v1.58.0, and the later labels here moved by two (see [Releases](#releases)). The plan had no section for it, so there are no plan deltas: the release's design (decisions D1–D24, risks K1–K21) stands in for one, and its differences are listed below.

**As built (2026-10-27).**
- **The switch** sits inside `mix_voice_and_soundboard`, after the chain and before loudness and the mute: `feed` → `modulator.apply` → the chain's reported latency, read once → `ptm.capture` → `chain.process` → `ptm.blend` → `loudness.process` → `run.mute` → the soundboard. Everything downstream gets the switched voice: the call, the headphone monitor, the WAV, screen takes, Replay and Voice reading's after-effects half. The soundboard, Speak and Critter Chatter never pass through it, and the mute still wins (D1, D6).
- **The chain keeps running while the effects are off** (D2), so a press has no gap and no stale tail, and an echo of what you said with them off is heard after a press (K10). With them off the call hears your voice with every effect off, the noise gate and the Denoiser included; loudness, if on, works on it as on the effects (D7, owner Q4).
- **The dry voice is delayed by the chain's reported latency** through a per-session ring, the power of two above the rate (65,536 samples at 48 kHz, so up to 1.37 s; a longer latency is clamped), so the effects and your own voice carry the same moment of speech. The screen recorder's tap is given the latency the chain reported, which the line moves to, so takes place the voice right with the effects on, and with them off once the line has settled within its clamp (D3; from the code, not checked in a take). A latency change moves the line's read point: a shrink crossfades the two taps over L, skipping the difference; a growth fades out, stays silent for the difference, then fades in, so nothing is heard twice (D4; a 2,048-sample growth is silent for 2,049 samples, the fade-out's last zero-gain sample and the 2,048). A new session starts at delay 0, so the preset's delay arrives as a growth, almost always during start-up silence. With the effects off, your own voice keeps the preset's delay: 21 to 71 ms on the bundled presets that delay you at all, and a Voice Convert voice that isn't streaming adds about a quarter-second (K21).
- **The crossfade** (D5): linear, one position step per sample, L twice the mute's: 512 samples at 44.1 and 48 kHz (11.6 and 10.7 ms), at least 10 and under 20 ms at every rate from 12.8 to 409.6 kHz. A toggle mid-fade turns round. Settled on the effects the buffer isn't touched, so with push-to-modulate off the stage sends what it did before (`the_applied_stage_matches_the_chain_alone`, and the 48 stage fingerprints below).
- **Where a switch lands in what you said** (D8b, owner Q9): an edge toward the effects starts crossing in the first buffer that sees it; an edge away from them keeps the effects for the chain's latency first (taken at the edge; a flip back meanwhile cancels it), so the effects cover everything said while they were meant to be on. The word, the ticks and the pill change at the edge, so the pill leads the sound by that delay when the effects turn off. What remains is the input's own buffering less how late the key reached Divora: up to about 20 ms said just before the effects turn off (the release in Hold to apply, the press in Hold to bypass) can reach the call in your own voice (K18). With a delay under L, the fade toward the effects covers about 11 ms around the switch partly dry (ptm.rs module doc).
- **The state** (D10): one engine-lifetime `AtomicU64`, `seq << 16 | mode << 8 | held`, a held bit per source (0 the global key, 1 the in-app key, 2 the Mixer's button, 3 MIDI; 4–7 reserved), so one source letting go never lets go of another. Modes `always` (push-to-modulate off, the launch default), `apply` and `bypass`. Hand-written compare-and-swap setters, from any thread; a set to the value it has changes nothing, `seq` included. The callback reads it once per buffer, the level emitter once per tick (`LevelUpdate.ptm`), and teardown never touches it.
- **Two sync commands** (D23): `set_ptm_mode {mode, restore?}` and `set_ptm_held {source, held}` (`"hotkey"` refused: only the backend holds that source), sync so that a press can't overtake its release (`ptm_commands_stay_sync`). Being sync does that only for commands sent one after another: v1.59.0's real-app run measured that commands in flight at the same time can reach Rust out of the order they were sent (40 sent at once), so a release sent before its press answered could land first and leave the hold on. Since v1.59.0 the page sends these one at a time, each once the one before it has answered or failed (v1.59.0's As built, below). A press made while another sync command holds the main thread (device enumeration just after the window gains the focus, a Start opening a device) waits for it (K8). Without `restore` (the Mixer card) the mode set lets go of every hold, the global key's included, which is the way out of a stuck hold (D13). The page's boot sends `restore: true` with the saved mode: it lets go of the page's holds and keeps a global key still held (D14).
- **The global key** is acted on in Rust, press and release (`ptm_on_hotkey` in `global_shortcut_handler`; the release comes from global-hotkey's poller thread, which checks every 50 ms whether the key is still down); the page ignores the event (D15). Binding `ptm` to another key, to a key Divora can't read, clearing it or clearing every key lets go of its hold at the map update. Re-registering the same key while it is still registered only checks that it is (no unregister, no register), so a reload's re-sync keeps a held key held. `ptm:` is reserved. **The lock order** (D16; built in S1 rather than track B): the three shortcut commands call the plugin only after `AppState.shortcuts` is unlocked, which removes a deadlock found by reading the code (a key's release on the poller thread during a rebind), never seen.
- **The page's sources** (D17–D19). The in-app key (Space, or the global key's accelerator) holds on a keydown on the page with exactly the bound modifiers, ignoring autorepeat. It lets go on the trailing key's keyup anywhere, or on the keyup of the physical key that started the hold: that second rule came from the review, for a character that changes with Shift or AltGr (Shift+1 binds as "!", and letting go of Shift first makes the keyup say "1"). The Mixer's **Hold to switch** (was **Hold to test**) holds while the pointer is down on it, or while Enter or Space is down with it focused; it lets go on pointerup, pointerleave, pointercancel, its own blur and the card unmounting, and a mouse press hands the keyboard back to the page. The page lets go of both on window `blur`, on `visibilitychange` to hidden and on Tauri's `onFocusChanged(false)`; whichever comes second sends nothing. WebView2 keeps a window hidden to the tray `visible` (v1.54.0, and P3 again), so a hide lets go through the focus it takes away, not through `visibilitychange`. MIDI: the store keeps the set of `ptm` mappings that are down and sends only when it turns empty or non-empty; a CC holds at 64 or more and lets go at 59 or less (the mute's dead band); an input pick or MIDI-learn on any mapping forgets them all, editing or removing a mapping forgets that one, and the card's set forgets the pads but keeps each CC's side, so a control left up must come down and go back up.
- **The status** (D21, owner Q5): Modulated when an effect other than the noise gate is on and push-to-modulate isn't taking the effects off. `ui.pressed` and `ui.ptmMode` are gone: the pill reads the store's mirror of the word. With push-to-modulate off by default (owner Q2), a preset with effects reads Modulated, so the titlebar, the Mixer and the overlay's spell circle say Modulated and animate where they said Clean (K2); Clean Passthrough, the noise gate only, reads Clean.
- **Saved** (D14, owner Q3): `divora.ptm = {on, hold}`, written only from an engine answer, from the setting as it stands when the answer lands. Holds are never saved. `ptmKnown` (the boot restore's answer or failure, or 500 ms; a tick doesn't count) holds the overlay's pushes beside the mute's.
- **The Mixer card** (U2): a switch, a line under it, Hold to apply / Hold to bypass and Hold to switch, the last two disabled while it is off. With no global key the line says the key works only in Divora's window and where to set one; with only a learned `ptm` MIDI mapping, and a MIDI input picked, it ends "…in Divora's window, or with your MIDI control." Settings → Hotkeys' row and the Control surfaces intro say what the key and a mapping do; the intro also says the card's set lets go.
- **Voice reading** (D9, U6): the microphone half is now the dry voice after the mute's `feed`, delayed by the chain's latency, pushed just before the after-effects half, so the two halves are compared at the same moment, and with the effects off the after-effects half is the same samples as the microphone half (scaled, if loudness is on), so it reads as a passthrough whatever the preset. A crossing the stage reports drops the after-effects window. The "passing the signal through" flag goes with the after-effects numbers it describes and changes only when they refresh, so it turns about 0.45 to 0.75 s after a switch (the worker test saw 0.53 to 0.72 s), and a held reading keeps its window's verdict. The cost (K3): effects that change nothing but still report latency (pitch or formant at exactly 0, the Denoiser at mix 0) no longer read as a passthrough.
- **Allocation** (D24): the voice stage allocates nothing with push-to-modulate switching, fades turned round, waits cancelled and latency changes mid-move. That is the claim, not the callback (Voice Convert's inference still allocates there).
- **The real-app run (S2)**, owner-set scope: no audio. The owner declined a test tone this time, and the SMSL DAC's loopback has recorded only silence since 2026-10-24, so P1 (the switch in a WAV) and P2 (takes and Replay) weren't run. The sound itself (the switch in a recording, takes, Replay, the dry voice lined up with the effects, clicks) rests on the unit tests through the real stage; the stage fingerprints show only that, settled on the effects, the stage sends what it did before: one hash per bundled preset, loudness off and on, 48 lines identical to `main` before the change, run on this machine before S1 and again after S1, after track A and after the review's fixes. What S2 ran: the app checks in debug builds of `93ab993` (P6, P5) and `6c180bf` (P4, P3; the claims audit's fixes), over WebView2's DevTools protocol, and B1 as a release-mode test:
  - **P6, the commands with real arguments:** every answer as expected (`seq` +1 on a change, unchanged on a repeat; the `hotkey` source refused; the restore letting go of the key); ticks within 1 to 24 ms.
  - **P5, a reload** with it on in Hold to apply: the restore sent at 24 ms and answered at 27 ms, before `list_audio_input_devices` (29 ms); the pill read Modulated from 38 to 49 ms after the reload, then Clean; the overlay received one `overlay:state`, clean.
  - **P4, the window losing the focus** with the in-app key held: `set_ptm_held {key, false}` 8 ms after the move (the page's `blur` at 7 ms, Tauri's focus event at 8), the next tick not held; harness check: with the page's `blur` listener removed Tauri's event let go alone at 8 ms, and with both removed nothing let go (the planned check removed only the page's `blur` listener; with Tauri's focus event letting go too, it has to remove both). `blur` alone wasn't isolated.
  - **P3, the real hotkey** (Ctrl+Alt+Shift+F24, 8 one-second holds by `SendInput`, 4 with Divora showing and 4 hidden to the tray): held on the first tick after each key-down, 5 to 33 ms after it; let go within 12 to 76 ms of each key-up (an upper bound on the 50 ms poller plus the tick wait; no audio timing in it); `seq` +2 a hold; the page sent no push-to-modulate command. The hidden four ran in a second run: the harness's check that the page had no focus refused them in the first, since the page read as focused just after the CDP click that hid it; the rerun checked the foreground window instead.
  - **B1, the release bench** (`a_long_mute_costs_at_most_twice_speech_or_a_pause`, its voice stage now carrying the dry line, settled on the effects): passes on all 24 bundled presets: the last 10 s muted cost 0.70 to 1.00 × max(speech, pause) (the bar is 2 ×), for example Zombie 277.3 µs on the tone, 293.2 µs on the pause and 241.7 µs muted per 480-frame buffer, Clean 4.3, 12.1 and 11.9 µs (`cargo test --release`, this machine).
- **Differences from the design:**
  - **D17:** the page also lets go on Tauri's `onFocusChanged(false)`, which the design kept out so P4 could show whether DOM `blur` alone works. P4 therefore tested both together, then with paths removed: Tauri's event lets go on its own, and with both removed nothing does; the DOM's `blur` alone wasn't isolated.
  - **D17, from the review:** the in-app key also lets go on the keyup of the physical key that started the hold (above).
  - **U2, from the review:** with only a learned MIDI mapping the card's line names the MIDI control; it had said the key works in a call. A mapping that hasn't learned a control doesn't count, nor does one while no MIDI input is picked.
  - **D9 / U6, from the review:** the "passing through" flag travels with the after-effects numbers. The design's version could pair the disguised numbers from just before a release with "passing the signal through", and hold that for 30 s in a pause. It now turns 0.45 to 0.75 s after a switch, not within 500 ms.
  - **Copy (§9 said unchanged):** Voice reading's after-effects line now reads "What the call hears: your voice after your effects, or as it is while Push to modulate has them off.", and its note "No measured difference from the microphone half — your effects are off, or they are passing the signal through." (it said the chain passes the signal through). Settings' MIDI sentence gained "Turning Push to modulate on or off on the Mixer, or picking its hold, lets go: …" (review).
  - **D14, from the review:** the saved setting is built when the answer lands, keeping a hold picked just before. An off sent while a hold pick was in flight had saved the old hold.
  - **K1:** the Denoiser reports 480 samples and delays 480 at 480-frame buffers, 928 at 512 and 864 at 1056 (`the_denoisers_real_delay_is_pinned`; the design said 928–957). So with the Denoiser on, a switch skips or repeats about 9 ms at 512-frame buffers and 8 ms at 1056. The other causes are as designed: Voice Convert's model about 5 ms past what it reports; Warble, which wavers by up to 8 ms; a pitch or formant shift set to exactly 0, 21 ms. Together, up to about 14 ms (Deep Narrator at 512-frame buffers, worked out from these figures). Not in the design: the harmonizer reports 0, and its harmony voices lag one vocoder window (21 ms) while its dry root doesn't, so at a switch the harmony, not your voice, is off by that much.
  - **The review** (step R): six confirmed findings, none refuted, all fixed before S2: the reading flag, the in-app key's release, the card's MIDI line, the reading's note and the saved setting above, and the Settings MIDI sentence.
- **Owner decisions** (the design's Q1–Q9, every default taken except Q8's tone): v1.57.0 shipped first (Q1); off by default (Q2); remembered (Q3); "effects off" is the whole chain, cut within the crossfade, with the chain running and your own voice keeping the preset's delay (Q4); a gate-only chain reads Clean (Q5); a latch later, `ptm:` reserved (Q6); the hotkey lock-order fix in this release (Q7); S2's eight `SendInput` holds yes, the test tone no (Q8); a switch lands where you spoke (Q9).
- **Done when** rests on N1 (a global key in a real Discord call: the preset while held, your voice after, no click, nothing doubled or lost mid-word), not run yet.
- **Not measured** (S2's gaps; the owner checks N1–N11 in MANUAL_TESTS → Push-to-modulate (v1.58.0) haven't been run yet): any sound in the app — the switch on the call, in the headphones, in a WAV, a take or Replay, its alignment and clicks (unit tests only; the fingerprints cover only the stage settled on the effects); the far end of a call (N1); the headphone monitor (N2); MIDI with a device (N3); a Stream Deck **Hotkey** action, expected to blip (N4); a key held with Shift (N5); an elevated window (N6); Voice reading in the app (N7); Voice Convert (N8); Discord's own push-to-talk on the same key (N9); the lock screen (N10); sleep and UAC; a hide to the tray with a key held; a preset switch with the effects off (N11); the card's line, Hold to switch and the in-app key's modifiers in the app (unit and e2e tests; P5 only clicked the card's switch); a relaunch; any rate but 48 kHz; the device signal after the resampler; Windows 10; B1 on other CPUs.
- **Follow-ups:**
  - **Report actual delays** (K1): the Denoiser at other buffer sizes, Voice Convert's model, the harmonizer's pad. The switch, the latency readout and the take's placement would then all use the real figure.
  - **A latch** under `ptm:` (Q6), for a glyph, or a Stream Deck's stock **Hotkey** action, which is expected to send a tap (N4).
  - **Keep cleanup effects while the effects are off** (Q4): the noise gate and the Denoiser; needs the chain split.
  - **A warning when Windows refuses a hotkey** (K20; v1.56.0's Q7): a refused push-to-modulate key now changes the sound, not only the status.
  - **A hold stuck across Win+L,** if N10 finds one with both the DOM and Tauri focus paths in place: a release on the session lock.
  - **The voice timers** (v1.60.0): `AudioEngine::set_ptm_mode` and `set_ptm_held` work from any thread, and source bits 4–7 are free. If Rust ever changes the mode itself, the page's boot restore must become a read, or a reload would put back the page's saved mode.

### v1.59.0 — Hourglass: countdowns and stream clocks on a hotkey (5–7 days)

**Goal.** A countdown *and* a count-up (stream uptime is the most common stream timer), each on hotkeys so Stream Deck works unchanged. Both are visible on the overlay and in an OBS text source while the main window is hidden to the tray.

**Deliverables.**
1. **`TimerCore`,** pure, in src-tauri/timers.rs. Countdown and count-up/stopwatch timers are held as `Instant` deadlines, advanced by an injected `tick(now)`.
   - **A countdown fires exactly once, never early.**
   - **A countdown that expired while the machine slept** is reported, not fired late.
2. **The `divora-timers` thread** (100 ms). It emits `timer-state` on every transition plus 4 Hz while anything runs.
3. **The `.txt` bridge.** `timers/<id>.txt` is written **on whole-second boundaries**, via temp file + rename. A rename that hits a sharing violation is retried quietly, because OBS opens the file without delete-sharing.
4. **Wire surface.**
   - **Commands:** `sync_timers {timers}` (pushes the definitions to Rust at boot and on every edit), `arm_timer`, `pause_timer`, `resume_timer`, `reset_timer`, `adjust_timer {deltaMs}`, `timer_status` and `timers_dir`.
   - **Storage:** definitions live in `divora.timers`. Persisted epochs re-seed Rust on boot, and Rust wins while a timer runs.
5. **Hotkeys:** `tm:<id>:toggle` / `tm:<id>:reset` through the existing register path. Defaults are empty, and the `tm:` prefix is reserved in STABLE-SURFACE.
6. **The overlay `timer` element** renders mm:ss in Space Mono from `timer-state`. These are JS text updates, not CSS keyframes, so the reduced-motion setting does not freeze the clock.
7. **Settings → Hourglass** section: the list, duration, Start/Pause/Reset, ±30 s, two hotkey rows, "Show on overlay", and the `.txt` folder with the hint *Text (GDI+) → Read from file*.
8. **New Sigils:** `hourglass` and `pause`.

**Tests.**
- **Rust:** `TimerCore`'s fires-once, never-early and expired-while-away cases, and the `.txt` formatter.
- **Vitest:** `tm:` parsing and sync.
- **E2E:** `timer-state` through `__E2E_EMIT__`.
- **Manual:** accuracy while hidden to the tray, and sleep/resume.

**Done when.** With the main window hidden, the overlay counts down within ±1 s of a stopwatch. An OBS text source counts down within ±2 s, because OBS reloads one 1 s poll after a seconds-resolution mtime change.

**As built (2026-10-28; S2 and the IPC-order fix 2026-10-29).** The design's decisions D1–D36 and risks K1–K21 stand in for the deliverables above; step R's changes (below) override them where they differ.
- **What shipped.** A pure `TimerCore` in `src-tauri/src/timers.rs` and its shell in `src-tauri/src/timer_thread.rs`. Up to 16 timers in **Settings → Hourglass**, after Stream overlay: countdowns (1 s to 24 h) and count-ups, each with Start / Pause / Resume, Reset, ±30 s, two hotkeys of its own (`tm:<id>:toggle`, `tm:<id>:reset`, unbound by default), **Show on overlay** (a live `timer` element, placed in Arrange like a picture, its 3:1 shape locked) and a text file, with **Copy path** and **Open folder**. The plan's eight commands and its `timer-state` event, `divora.timers`, and the sigils `hourglass` and `pause`. At zero: `00:00` and **Done**, nothing else (owner Q1). One fixture, `src/overlay/timer.fixture.json`, holds the names, keys, limits, the id and `tm:` tables and run samples, read by the Rust and the TS tests.
- **Plan deltas:**
  - **The clock** (D3): live deadlines are a `Duration` since a process origin, not `Instant`s, so the core is pure and tests can build any reading. Wall time (Unix ms) only sets the anchors a restart seeds from, and checks a sleep the clock missed. A wall-clock step over 1 s that holds 10 s moves the running timers' anchors, so a restart doesn't jump by it.
  - **Emits** (D13, D18): on every change and at each running timer's next second boundary, not 4 Hz while running. `divora-timers` waits until that boundary or 100 ms, computed from the reading it ticked with, so emitting and handing off can't push a deadline back. A paused timer emits nothing, so the page and the overlay each read `timer_status` once after they start listening (D25, D32).
  - **Text** (D22): `MM:SS` under an hour and `H:MM:SS` from an hour (the plan said mm:ss; owner Q4). A countdown rounds up, so it reads `05:00` for its first second and `00:00` only once it has finished or run out while away; a count-up rounds down.
  - **`tm:` keys are acted on in Rust** (D19), inside `WM_HOTKEY`, on the press only, as the mute's is (push-to-modulate's is acted on in Rust too, at its press and its release); the page ignores the event, so a key works while the page reloads. Toggle: idle, done or ran out → start; running → pause; paused → resume.
  - **Sleep** (D5; the plan's "persisted epochs and the resume signal", [Risks](#risks-and-unknowns)): every reader — the thread, a command, a press — takes the core's lock, reads the clock once and asks v1.54.0's `SleepWatch` (now re-exported by `divora-capture`, with a stub that never reports a sleep off Windows) before it acts. (a) A gap over 500 ms on the clock: a countdown due by then is `expired`, never fired; one still ahead keeps running, the sleep counted; a count-up pauses where the PC slept (owner Q2). (b) A resume signal with no gap, unless an (a) was handled in the last 60 s: each running timer is checked against its wall-clock anchor with 2 s of slack, so on a clock that didn't count the sleep a countdown takes the time its anchor says it has left, or expires if its anchor has passed, and a count-up pauses. Each (a) and (b) that finds a timer running logs one line saying which clock decided. A countdown found late with the PC awake (a stall) fires late, logged at warn past 1 s (owner Q9), so only a sleep or a closed Divora expires one, except in the case under **Stated, not fixed** (below).
  - **Restarts** (D10, D11; owner Q3): `divora.timers` holds the definitions, each timer's keys by verb and its last run with wall-clock anchors, written only when the run changes, never on a text-only change. Every `sync_timers` sends the runs as `resume`; Rust seeds a timer from one only for an id it hasn't accepted in this process, so a page reload keeps Rust's timers ("Rust wins"). Time passes while Divora is closed: a countdown comes back with what it would have had left, or `expired`; a count-up includes the gap.
  - **The files** (D13, D21) are written by a thread of their own, `divora-timer-files`, fed through a latest-value slot, so no file write's disk wait reaches the timer thread (its own log writes still can). Each write is a temp file and a rename, never removing the target first. The plan said a rename is retried on a sharing violation; measured here with a reader that opens the file as OBS does (`_wfopen "rb"`; OBS itself wasn't run), a rename onto the held file fails with `PermissionDenied` (5), and a sharing violation (32) comes from a held temp or from removing a held file, so both are retried, at every pass of the writer (about every system timer tick) until the write lands or newer text replaces it, then once a second after 1 s of failures, with one warning per file and error. Every 5 s a written file that went missing is rewritten. Once, at the first texts after the first sync, the writer removes `timer-<n>.txt` files no timer has and `.timer-<n>.tmp` leftovers, and nothing else in the folder. `tauri dev` builds use `timers-dev/` (as `assets-dev/`).
  - **The file changes with the text,** not on a separate whole-second schedule: a running timer's text changes once a second, at its own boundary.
  - **Reading sync input** (D35, D36): `sync_timers` takes each entry on its own and never fails; a bad name or length falls back, a bad run seeds idle. A stored kind this build doesn't know, or a timer past the 16th, is kept, not run: sent with its id (so its file isn't swept), shown with Delete only, its keys not registered.
  - **Transitions** (D34): the core records each fire, expiry, sleep pause and seed wherever it happens (the thread, a command, a press) and only the timer thread drains them, logging one line each. That is v1.60.0's one place to act from.
  - **±30 s** (D7; owner Q5) changes only a running or paused timer, and −30 s stops a countdown at 00:01, so it never ends one; End now (v1.60.0) is the way to end it.
  - **The overlay face** (D24; owner Q6): white Space Mono Bold with a dark outline, sized to its box, text changed by JS (so reduced motion can't freeze it), drawn only once the overlay page has the backend's timers and the font (or 1 s); "Timer missing" only in Arrange, and only after the timers have arrived.
  - **Settings** (D26, D29, D30): Length and Name apply on Enter or leaving the field, never per keystroke; Delete asks twice (**Confirm?** for 3 s; owner Q10); every control names its timer; one polite live region says a timer finished or ran out while away, never a time; a key row says "Also set for {other}." when another binding has its key; a hotkey note under the cards states the keys' limits by their cause. File names are `timer-<n>.txt`, the lowest free number (owner Q7).
  - **`HotkeyCapture`,** used by Settings → Hotkeys and the Soundboard's tile hotkeys too, was fixed: its listener leaked when the field unmounted while armed, or was clicked again to cancel (the next key was still bound). It now has an optional `ariaLabel`, and a capture armed by a pointer gives the focus back when it ends.
- **Changed by the code review** (step R: 12 findings confirmed, some found twice, all fixed; each change of behaviour has a test that fails without it):
  - **Nothing slow under the core's lock.** The core logged its rare decisions under the lock, inside `WM_HOTKEY` too, and the app's log writer is a blocking file appender. It now logs nothing: it pushes notes (`ClockSlept`, `WallChecked`, `AnchorFollowed`, `DuplicateId`; at most 64) that the timer thread drains with the transitions and logs after unlocking.
  - **A late second resume signal after a stall.** Windows' notification can come more than 5 s after the jump the sleep check saw; if every reader was held up past 500 ms just then, (a) took the hold for a sleep and expired countdowns and paused count-ups with the PC awake. Within 60 s of a handled (a), a gap of 5 s or less is no longer an (a); it falls to (b), which the 60 s rule skips. The cost: a real second sleep of 5 s or less in that minute counts as time awake (its countdowns fire up to about 5 s late, its count-ups count it).
  - **A deleted kept-not-run timer's file** is removed; the writer had remembered only the files it wrote.
  - **The live region** was emptied by the next snapshot, often within milliseconds, so nothing was said. An edge now empties it and puts its words up on the next animation frame (as the toast does), joins edges that come before that frame, and empties it 2 s later; a snapshot with no edge leaves it alone.
  - **Delete** awaited both unregisters before sending its sync, so an Add in that gap took the id and inherited the deleted timer's run in Rust. It now saves, then sends both unregisters, takes the face off the overlay and sends the sync in one turn; and a delete that brings a timer under the limit registers that timer's stored keys right after the sync. The fix took it that the backend runs commands in the order they are sent. S2 then found that commands in flight together can arrive in any order, so since the IPC-order fix (below) these go through the timers' queue, one at a time, and an Add's sync goes after them.
  - **A capture also ends when the focus leaves the field,** so a key typed into another field is never bound.
  - **The overlay page's header comment** no longer says a first face never swaps fonts on stream: the font wait is capped at 1 s. Alongside, the "Timer missing" badge gained the hourglass sigil, as "Picture missing" has its own.
  - **Two tests:** the skip-and-cap of `sync_timers`' entries now runs through the real `TimerShared::sync` (the table had copied its reader), and a writer-thread test tolerates one held-up pass.
- **Smaller differences while building:**
  - `TEXT_FILES` (K1's kill switch, named but not defined in the design) is a constant: false keeps the writer thread from starting.
  - A failed write removes its temp. An error other than a reader's hold is warned at once and then follows the same retries and backoff (the design said "logged once, pending"). A read-only file's eventual write counts in the collision line: both fail with 5.
  - The writer thread holds its slot weakly, so it ends if the slot goes (D14; never in the app, where the timer thread holds it).
  - A stored running or paused countdown run without a total seeds idle; a done or expired one takes its length.
  - Settings asks for the folder (`timers_dir`, which creates it) only on **Copy path** or **Open folder**, so opening Settings makes no folder. **Open folder** is named "Open the timer files folder" for screen readers, apart from Recordings' **Open folder** on the same page. The Hide-from-recordings line shows only while that timer is on the overlay. Reset is never disabled. A pointer-armed capture also gives the focus back after Esc or a second click.
  - Rename, Length and key binds act only on timers Rust runs, and an unchanged commit sends nothing.
  - In the code: `TimerEvents`' method is `timer_state` (`state` would clash with `tauri::Manager::state`); `timers` and `timer_thread` are `pub` modules; the wake channel is `sync_channel(1)` with `try_send`, so a wake never blocks and can't pile up.
  - The always-mounted live region is a second `role="status"` on the page, so the e2e's What's-new check excludes it, as it does the mute's announcer.
- **Owner decisions** (2026-10-28): every default, Q1–Q7, Q9 and Q10: display only at zero; a count-up pauses at a sleep; time passes while Divora is closed; `05:00` under an hour, `1:02:03` past it; −30 s stops at 00:01; one look for the face; `timer-<n>.txt` names; a countdown due during a stall finishes late; Delete asks twice. Q8 (S2 on this machine, and H3's five `SendInput` presses): yes, asked again on 2026-10-29, with H5's Quit from the tray through UI Automation too.
- **The real-app run (S2, 2026-10-29),** on this machine (Windows 11), in debug builds of `de6d8ab` (every run) and `f22eb17` (H6 again, with the IPC-order fix below), driven over WebView2's DevTools protocol, the commands sent from the page through the real IPC. The engine on v1.58.0's silent route (the Focusrite in, the Realtek USB speakers out, Monitor off, no monitor device); nothing played. Divora was hidden with the titlebar's Close button for H1, H2 and H3. Every run passed except H6's raw sends; two cases passed only by the harness's own line, not the one MANUAL_TESTS planned (H2's simulated OBS, H7's early delete). Each run's setup and pass line is in MANUAL_TESTS → Hourglass (v1.59.0).
  - **H1, the overlay with Divora hidden** (4 min; a 3:00 countdown and a count-up, Locked): each face change timed against its second as the timer's `anchorMs` puts it, 7 and 8 ms late at the median and 16 ms at worst outside H1b's calls, 18 ms at worst inside them (the bar is ±1 s); no value skipped, neither face blank once drawn. The countdown finished once, the app's fire line 1 ms after its deadline. `divora.timers` was written 3 times, once per transition. The lag drifted +0.66 and −0.38 ms a minute.
  - **H1b, the main thread held:** 5 `list_audio_input_devices` calls took 11–21 ms each; the faces' lag around them 18 ms at worst, and all 248 file changes during H1 within 100 ms of their second. A longer hold isn't measured.
  - **H2, the file against a reader and a holder** (5 min; a reader opening it as OBS's `_wfopen "rb"` does every 10 ms; a holder keeping it open without delete-sharing for 40 ms at every second): 29,953 reads, none empty or partial; 5 opens failed with error 32, the read hitting the rename itself; 299 changes, every one collided, each 37–87 ms after its second (median 47 ms); about 60 writes a minute waited for a reader, the longest 62 ms. An OBS simulated from the logged write times (its two-poll rule, 40 random poll phases, polls 1000 and 1016.7 ms apart): median 563 ms behind, p95 1016 ms, worst 2108 ms; over 2 s in 16 of 23,760 samples (0.07 %), from OBS's own polling with on-time writes. So the plan's "within ±2 s" for OBS doesn't hold as written; the harness's line, two polls + 100 ms, passed.
  - **H3, the real hotkey** (with the owner's go-ahead; Ctrl+Alt+Shift+F23 and F22 bound through Settings, Divora hidden, 5 `SendInput` presses at least 1 s apart): each press in the hidden page's next `timer-state` 0.8–1.9 ms after its key-down; the page sent no timer command.
  - **H4, a reload mid-countdown:** the boot sent `sync_timers` with the run as `resume`, then `timer_status`; Rust kept its run (`anchorMs` unchanged); the face never blank, no value skipped.
  - **H5, a restart,** twice (the harness ending the process with `taskkill /F` 12 s after the last start, then Quit from the tray through UI Automation), closed 40.5 s and 40.7 s: the 3:00 countdown back with `anchorMs` − now left, within 1.8 and 2.2 ms; the 0:20 one `expired`, `fires` 0; the count-up including the time closed, within 1.8 and 2.2 ms; each file rewritten 7–14 ms after the first sync.
  - **H6, quick clicks:** the clicks through Settings passed on both builds (`data-timer-seq` never went down; the page's copy equal to `timer_status` after each batch). The raw sends failed on both: the finding below.
  - **H7, a delete with the file held 3 s:** the file stayed while held. Deleted 0.5 s before the release, it was removed 20 ms after it; deleted 0.2 s into the hold, 242 ms after it, outside the planned 100 ms, because after 1 s of failed removes the writer tries once a second (the harness held that case to 1.1 s). A launch's sweep of a file a delete left wasn't measured: none was left.
- **Found by S2: commands in flight together reach Rust out of order.** H6's raw half sent 40 `pause_timer` and `resume_timer` commands, and ±30 s adjusts, straight to Tauri's `invoke` at once without waiting. Every answer was right for its command, but Rust ran them out of send order: on `de6d8ab` 12 of 40 out of order (the 29th ran second, and the 29th–40th, mixed in with the 2nd–9th, all ran before the 10th–28th), and a `timer_status` sent after a countdown's two adjusts ran before them. Tauri 2 sends each command as its own request to `http://ipc.localhost`, and WebView2 doesn't keep the order of requests in flight at the same time. A sync command runs on the main thread when its request reaches Rust, so being sync orders only commands sent one after another, each answered before the next goes. The design (D16, D17, §6.3) took it for more, as v1.58.0 did for push-to-modulate (above). What that left open: a Start and a Reset clicked quickly could land reversed and leave the timer as the first click left it; a delete's unregisters and sync, sent in one turn, could land after the sync of an Add that took the id next; a push-to-modulate release could land before its press and leave the hold on until the next edge; two of the mute's sets could land reversed and leave the opposite of the last click. None of these was seen in the app; only the burst of 40 was measured.
- **The fix (`f22eb17`, in this release).** It doesn't change the IPC: it keeps the page from having two of a feature's commands in flight. `src/data/inOrder.ts` is a queue that sends a command in the caller's turn when nothing is in flight, and otherwise once every command queued before it has answered or failed; a failure doesn't hold up the rest, and each caller gets its own answer or error. The store keeps one per feature: Hourglass (its syncs, each carrying the timers as they were when it was queued; the timer commands; its keys' registers and unregisters), push-to-modulate (holds from every page source: the in-app key, the Mixer's button and MIDI; the card's mode; the boot restore) and the mute (set and toggle). Reads (`timer_status`, `input_mute`) aren't queued: their answers are adopted by `seq` wherever they land. The cost, from the code: a command waits for the one before it in its queue, and one held up on the main thread holds up the rest of that queue, as it would hold up any sync command sent after it anyway. The rerun on `f22eb17`: 20 clicks on Settings' **Pause** in one burst went out one at a time, the answers' `seq` strictly increasing, and the timer ended in the phase the clicks imply; 10 presses and releases of the Mixer's **Hold to switch** went out one at a time, each pair reaching Rust in order, and the last answer and the engine's tick said not held. The raw sends still reorder there (21 of 40), as they must: they call `invoke` past the store.
- **What the queues don't cover:** any other command the page sends that changes something can still land out of order when two are in flight together, for example a slider's values or a key bound in Settings → Hotkeys (from the code). To check one, find its send: a queued one goes through `muteQueue`, `ptmQueue` or `timerQueue` in `src/stores/app.tsx`, or through the overlay controller's `placement` or `assets` queue in `src/overlay/controller.ts` (open, close, Fit, the background and pictures; not the mode or Hide from recordings). A later feature's commands need a queue of their own (v1.60.0 and v1.61.0 below say where).
- **Done when** rests on H1 (the overlay within ±1 s with main hidden) and O1 (an OBS text source within ±2 s; since H2, O1's line is about 2.1 s behind, two of OBS's polls + 100 ms). H1 passed, timed against Divora's own anchors rather than a stopwatch: 18 ms at worst over 4 min (O5 is the hour against a stopwatch). O1 hasn't been run; H2's simulated OBS was over 2 s in 0.07 % of samples.
- **Stated, not fixed** (from the code): on a clock that doesn't count sleep (this PC's does), with no counted sleep in the last minute, a resume signal that arrives while every reader is held up for more than 500 ms with the PC awake is still taken for a counted sleep: countdowns due in that hold expire instead of finishing late, count-ups pause, and the wall-clock check is skipped. Two Divoras running at once (the installed app and a `pnpm tauri build --debug` build) share `timers/`: same-id files hold whichever wrote last, and each one's first sync can remove the other's other `timer-<n>.txt` files, which the other's 5 s check then writes again (from the code; never run).
- **Not measured** (S2's gaps; the owner checks O1–O7 in MANUAL_TESTS → Hourglass (v1.59.0) haven't been run): a real sleep (O3; this PC has only S3 sleep), hibernate (this PC can't), a clock that doesn't count sleep; OBS itself, GDI+ and FreeType 2 (O1, O2; not installed here; H2 simulated OBS's polling); a Stream Deck and a key's autorepeat (O4; `SendInput` doesn't autorepeat); an hour against a stopwatch (O5); End task right after a change (O6; H5's first close was a forced end, `taskkill /F`, 12 s after the last start); a key set twice in storage (O7); a main-thread hold longer than H1b's; two commands sent tens of ms apart while the main thread is busy (H6 sent 40 at once); a launch's sweep of a file a delete left (H7 left none); screen readers; Windows 10; Modern Standby; the overlay on a second monitor; two Divoras at once; the non-Windows `SleepWatch` stub (no CI job compiles it).
- **Follow-ups:**
  - **Overtime** (counting on past zero) and **styling the face** (owner Q6's alternative).
  - **A sound at zero** comes with v1.60.0's tile action (shipped: Play a clip, below).
  - **A warning when Windows refuses a timer's key** (K20; v1.56.0's Q7): today a refused key shows as bound and does nothing, and the hotkey note tells you how to check one.
  - **The stall case above,** if a PC whose clock misses sleep turns up (O3's log line says which clock decided).
  - **Compile the non-Windows stub** (K4): in CI, or once by a local `cargo check -p divora-capture` for another target.
  - **Queue the page's other commands that change something** (the IPC-order finding above), for example a slider's values, the effect chain's sends and the hotkey bindings'; the self-check above finds the rest, counting v1.60.0's `chainQueue` and `soundboardQueue` in `src/stores/app.tsx` as queues too. (v1.60.0 queued the effect chain's sends, its effect sliders' included, and the soundboard's plays and stops; the master soundboard volume (`set_soundboard_master_gain`), Reactive effects' config, which a chain change can send while Reactive effects is on (`set_reactive_config`), other sliders and the hotkey bindings' are still open.)
  - **v1.60.0's open question** (below): what an expired timer runs. (Answered in v1.60.0: only its restoring actions.)

### v1.60.0 — Timers that change your voice (4–6 days)

**Goal.** The slice nothing else offers: a timer that switches preset for its duration and reverts, fires a tile, speaks a line, saves a replay, panics, or mutes for a break.

**Deliverables.**
1. **The `TimerAction` union.** It is the glyph actions plus:
   - `revertPreset` and `randomPreset`;
   - `speak {text, voiceId}`;
   - **`saveReplay`**, from v1.54.0;
   - an **idempotent `setMuted {muted}`**. The glyph mute toggles, which would *unmute* an already-muted voice on BRB.
2. **Hooks and dispatch:** timers gain `onStart`/`onFire`, all dispatched by one generalised `dispatchAction`. *(Since v1.59.0:)* they reach Rust as new fields of `sync_timers`' entries, which Rust reads one by one: an unknown action kind is skipped without failing its timer (v1.59.0's D35).
3. **Where actions run.** *(Amended by v1.59.0's design, D33: nothing may block `divora-timers` or `WM_HOTKEY`, where a `tm:` press already acts.)*
   - **A Rust worker of its own,** fed from the transitions `divora-timers` drains (v1.59.0's D34; a fire is recorded wherever it happens, the thread, a command or a press): mute, unmute, panic and saveReplay. Not the timer thread itself: `saveReplay` can wait up to 90 s, and speech and OBS requests can wait too. Rust-side panic emits `soundboard:stopped` so the store clears its playing clips.
   - **JS, on the `timer:fired` event,** emitted from the same drained fires: everything else. Events reach a hidden page promptly; it is JS *timers* that get throttled. *(Since v1.59.0:)* every command these actions send from the page that changes something goes through its feature's queue in the store (`src/data/inOrder.ts`; the mute's, push-to-modulate's or the timers'), and a feature without one gets one: commands in flight together can reach Rust out of the order they were sent (v1.59.0's S2, 40 sent at once), so a fired action and a click on the same control could otherwise land reversed.
   - **Late fires:** each fire carries `late_ms`. Since v1.59.0 a countdown found late with the PC awake fires late (owner Q9), so its actions run late rather than not at all.
   - **Open question, from v1.59.0:** when a timer whose `onStart` changed something (`setMuted true`, a preset, an OBS scene) ends `expired` — the PC slept through its end, or Divora was closed — should its restoring actions (`setMuted false`, `revertPreset`, `sceneBack`) still run, while one-shot actions (`speak`, a tile, `saveReplay`) are skipped? Skipping all of them leaves a break muted and OBS on the BRB scene with nothing to undo it. v1.59.0 keeps `done` and `expired` apart on the wire so this can be decided, and its `Seeded` transitions let v1.60.0 re-apply `onStart`'s state to a timer restored running after a crash.
4. **`speak` limits:** Kokoro and Critter Chatter voices only.
5. **"Break (BRB)" template.** It mutes on start, then unmutes and optionally speaks a line at zero.
6. **End now.** `finish_timer {id}`, plus the hotkey id `tm:<id>:finish`, runs a countdown's at-zero actions immediately. Ending a break early therefore unmutes, while Reset still cancels without firing. *(Since v1.59.0:)* the `Finish` op calls the core's one private `fire()`, so "fires exactly once" covers it too, and the page sends `finish_timer` through the timers' queue, as every timer command, so an End now clicked just after a Pause lands after it. v1.59.0 keeps a stored `finish` key and writes it back unchanged, but never registers it, and its `tm:` table ignores `finish`; a downgrade doesn't lose the key.
7. **Defined behaviour** for a deleted preset and A/B compare open. Push-to-modulate keeps switching your effects under a timer's preset (since v1.58.0 it switches them in the engine), and the status says which the call hears.

**Done when.**
- A 5:00 "Villager" timer switches preset at start and reverts at zero.
- A BRB timer really silences the send, and End now brings the voice back early.

**As built (2026-10-30; S2 and the espeak-ng fix 2026-10-31).** The design's decisions D1–D37 and risks K1–K17 stand in for the deliverables above; step R's changes (below) override them where they differ.
- **What shipped.** A countdown in **Settings → Hourglass** has two lists, **At start** and **At zero**, of up to 8 actions each, from eight kinds: `preset {presetId}`, `randomPreset`, `revertPreset` (At zero only), `tile {tileId}`, `speak {text, voiceId}` (one per list, up to 200 characters), `saveReplay`, `setMuted {muted}` and `panic` (Stop all clips and lines). **Add break (BRB)** makes "BRB", 5:00, Mute my voice at the start and Unmute my voice at zero, in one sync. **End now** — a button, `finish_timer {id}` and the hotkey `tm:<id>:finish`, now registered — finishes a running or paused countdown through the core's one `fire()` and runs its At zero list. Count-ups have no lists and no End now. The lists travel in `sync_timers`' entries; Rust reads each action on its own, and one it can't read is skipped there, kept in storage and never sent by the page.
- **Rust decides what runs.** The core records `Started` and `Cancelled` beside `Fired` (which now says whether End now finished it), `Expired` and `Seeded` (which now says what phase the stored run had), each carrying the timer's lists. The timer thread drains them as in v1.59.0 and, unlocked, turns each into a hook run with one pure table, `plan` (`src-tauri/src/timer_actions.rs`), and splits it with another, `route`: the backend's half goes to `divora-timer-actions` by `try_send` on a bounded channel of 32, and the page's half is emitted as `timer-actions`. The thread writes its log lines after the pass's hand-offs (a dropped batch's warning excepted, written at its hand-off), so a slow disk can't delay that pass's unmutes, though a stuck write holds up the next pass. The worker mutes and unmutes (one atomic op on the engine's word), stops every clip and line and emits `soundboard-stopped`, hands a replay save to the blocking pool, and puts a line in the latest-wins slot of `divora-timer-lines`, which renders one line at a time with Kokoro or Critter Chatter (never a clone) and plays it in a mixer slot of its own, `"timer-line"`, at gain 1.0 (100 %, Speak's default; the master soundboard volume still scales it), not kept in Saved clips. The page runs preset switches and clips, each through its feature's queue (a switch's `set_reactive_config` excepted: with Reactive effects on, it goes out unqueued, as on any chain change), and preset work only once the real preset list has loaded. `TIMER_ACTIONS = false` turns all of it off; End now still finishes.
- **What runs when** (owner Q1–Q3, Q12). At start runs when a countdown starts from the beginning, never on Resume. At zero runs whole at 00:00, up to 30 s late, and on End now. A countdown that ran out while the PC slept or Divora was closed, or finished more than 30 s late, runs only the actions that put something back: Unmute my voice and Back to the preset before. A Reset, a kind change or a delete while it runs or is paused runs only those that undo what that run's start did. A restart mid-run runs At start's mute again and nothing else (owner Q4), and the page names the timer in a notice. Pause and Resume run nothing, so a paused break stays muted, and the card says so. An edit to At zero counts for the run that's going on; an edit to At start, from the next start, or from a relaunch while it runs or is paused (owner Q12): the restart's mute and a later Reset's or delete's Unmute my voice follow At start as it was at the relaunch.
- **Back to the preset before** switches back only if you're still on the preset the timer chose, the rule v1.61.0's `sceneBack` uses (below). Two timers that each switch and switch back nest. The return is stored in `divora.timers` (`presetReturns`), so one whose event no page heard (a reload, a run-out while Divora was closed) is settled by the next page's reconcile, once its preset list has loaded.
- **Plan deltas:**
  - **Names** (D31): the events are `timer-actions` (it carries starts and ends, not only fires) and `soundboard-stopped`, hyphenated like `timer-state`, not `timer:fired` and `soundboard:stopped`.
  - **The union** (D1, owner Q8): the glyph actions *but the two toggles* (Monitor, and Mute / unmute), plus the plan's four. A timer runs unattended, and a toggle can do the opposite of what you meant; Mute my voice and Unmute my voice set the state they name.
  - **Speak runs in the backend** (D5, D21; owner Q6), not in JS: `tts::synthesize` handles exactly Kokoro and Critter Chatter, so deliverable 4's limit holds by construction, where the `speak` command looks a voice up as a clone first. A line plays during a page reload, stays out of Saved clips and out of the Speak screen's slot, and holds no page queue behind seconds of synthesis. One line per list (D2), since a newer line from any timer replaces one still being prepared or playing.
  - **Starts, cancels and restarts are transitions** (D6), so a Reset or a delete can undo a start, and a restart can mute again.
  - **The open question** (D7–D10, owner Q1–Q3): as proposed, only the restoring actions run after a run-out while away. A fire found more than 30 s late runs only those too; the plan had late fires run everything, late. Reset and delete still don't fire ("Reset still cancels without firing"), but undo what the start did.
  - **Queues** (D19): two more in the store, one for the effect chain (`set_effect_chain`, `set_effect_param`, `set_effect_enabled`, `set_voice_model`) and one for the soundboard (`play_soundboard_clip`, `stop_soundboard_clip`, `stop_all_soundboard_clips`, a saved Speak clip's play); `finish_timer` and the finish key's register go through the timers'. A preset switch now sends its chain once: it sent it twice, and in a queue the second would have landed after the voice model and cleared it.
  - **The Break template** (owner Q5) has no line; a line at zero is an action you add.
  - **Defined behaviour** (deliverable 7; D18, D29): a timer's switch is a preset switch like any other, so Compare goes back to A with both slots the new preset's sound; on the Presets screen the view stays where it is. A preset that's gone switches nothing, and its picker reads "Missing preset". While a timer's switch is live its card says what your call hears: the mute, a preset that doesn't change your voice, and push-to-modulate holding it off included.
  - **Not in the plan:** `espeak-ng` starts with no console window (D35, `CREATE_NO_WINDOW`), since a line now fires unattended over a game; and `Select` gained a keyboard, in every place it's used, and an accessible name that carries its value (D28), which a timer card's preset, clip and voice pickers give it; the others are named, as before, by the text they show.
- **Changed by the code review** (step R, `9014df0`: 9 of 17 findings confirmed by a skeptic, all fixed, each with a test seen failing without its fix):
  - **Reset and delete judge the undo against what this run's start did.** They had read "did At start mute?" from the list as edited since the start, so removing Mute my voice mid-break and then pressing Reset left you muted. The core now keeps the lists a run started with (`Timer.started`, set where `Started` is recorded and where a running or paused run is seeded), `Cancelled` carries them, and `plan` takes the undo from them and the actions from the current At zero. At start's "a change here counts from its next start" now holds, except across a relaunch mid-run, which takes At start as it is at the relaunch for both the re-mute (the help's second clause) and a later Reset's or delete's Unmute my voice.
  - **Reset's row** names only what the card's At zero holds, so it no longer names an unmute or a switch back that At zero doesn't have. Its Unmute my voice reads "if its start muted you", going by the run's start as Reset does, and At zero's help says the same (since the claims audit: both had read "if At start mutes you", At start as it is now, so after an At start edit mid-run they could name an unmute Reset won't do). **The ran-out line** says what runs when it runs out like this, in the present tense, instead of claiming what ran: the card keeps no record of that.
  - `Select` and Add action scroll the highlighted option into view on keyboard moves (only then, so a still pointer can't make it scroll again). A clip row reads "Loading clips…" until the soundboard folder's scan finishes, and a disabled Play a clip gives one of three reasons. Re-picking the current MIDI action or glyph kind keeps its targets.
  - Two tests: a start-revert test that can fail, and a source test pinning that the worker host's mute, Stop all and `end_run` call the helpers the engine tests cover.
- **Smaller differences while building:**
  - Save a replay in At start says it saves the seconds up to the moment the countdown started (the design's copy said zero). Back to the preset before stored in At start (a hand edit) is a row that doesn't run, with its reason.
  - A preset picker reads "Loading presets…" until the real list loads, so one of your own presets isn't called gone. A switch back to the preset that's already live does nothing. A start's switch to the preset you're already on still acts as a Use of it: Compare goes back to A with both slots the sound you have now, and the return it stores leads back to that same preset, so only its switch back is skipped.
  - Reconcile leaves alone any timer the page has heard an event for since it loaded: the page can hear a start before its `timer-state`, and reconcile, reading the timer as idle, had switched straight back (found and fixed in the store's track).
  - A countdown that stops running without being deleted (pushed past the 16th by a sync, or turned into a kind this build doesn't know) is cancelled as a delete.
  - The thread logs a fire planned as more than 30 s late at warn ("a timer finished late; only what puts things back runs"), and the line thread logs each line's outcome: `played`, `replaced`, `ended`, `stopped` or `failed`.
  - The restart notice uses the error toast's tone, which gives more time to read. Every `Select` whose value matches no option shows its own missing text ("Missing preset", "Loading presets…"), else "—", instead of its first option (a MIDI preset mapping with none set shows "—", for example).
- **Owner decisions** (2026-10-30): every default, Q1–Q12, with nothing dropped from the design's shrink list: only the restoring actions after a run-out while away (Q1); Reset and delete undo only what the start did (Q2); 30 s for a late fire (Q3); a restart mid-break mutes again, running or paused, with a notice (Q4); the Break has no line (Q5); lines in the backend, one per list, at a fixed 100 % (Q6); a break's end unmutes whatever muted you (Q7); no toggles (Q8); no count-up actions (Q9); random means any preset that changes your voice but the live one (Q11); At zero edits count for this run, At start edits from the next start, and its Mute my voice at a relaunch while it runs or is paused (Q12). Q10, S2 on this machine: asked again at S2 on 2026-10-31, yes to all four, B2's WAV take, B3's one `SendInput` press and B5's tray Quit included.
- **The real-app run (S2, 2026-10-31),** on this machine, over WebView2's DevTools protocol: a debug build of `9014df0` for every run but L1, which ran on a release build (`pnpm tauri build --no-bundle --config src-tauri/tauri.bundle.conf.json`, the release workflow's command without the installer). The engine on the silent route (the Focusrite in, the Realtek USB speakers out, Monitor off, no monitor device); nothing played on a device; X1 and L1 with the engine stopped. Each run's setup, pass line and figures are in MANUAL_TESTS → Timers that change your voice (v1.60.0).
  - **V1** (done-when 1, 5:00, Divora hidden after its start) passed: one chain to Villager at the start event and one back to Hollow King at the end, each within 4.6 ms of its event across three runs.
  - **B1** passed: the first level tick reading muted 10 ms after the click on Start, the first unmuted 5 ms after the click on End now (each to the 33 ms tick), every tick between muted, no mute command from the page. **B3** passed: End now's key by `SendInput` with Divora hidden, done 1 ms and unmuted 29 ms after the key-down.
  - **B2** was inconclusive: a take of the send held exactly one run of exact zeros, 20,078 ms against 20,039 ms between the events, but a quiet room through the noise gate left the 2 s either side only 96.3 % and 96.2 % non-zero, under the 99 % line.
  - **B4, B5, X1, R1, V1b** passed: across a reload the page heard neither end, the backend unmuted alone, and reconcile put the user preset back; a restart mid-break muted again with the notice, and a switch that ran out while Divora was closed was put back; one `soundboard-stopped`; "Replay is off."; one chain, then the voice model.
  - **L1** passed on its second run, on the release build: a Critter Chatter line rendered in 37 ms and a Kokoro line in 1,624 ms with the engine stopped, nothing in Saved clips, and no new window from any process. V1 and V1b passed on reruns too; their earlier runs failed on the harness's own bugs (it counted CORS preflights as commands, compared a bundled preset's chain too strictly, and its window for V1's middle took in the end's chain).
- **Found by S2: Kokoro voices failed in a built app.** L1's first run failed on the Kokoro line: espeak-ng exited 1 on `\\?\C:\…\target\release\tts\phontab`. A built app's resource folder comes from tauri-utils' `StartingBinary`, which canonicalizes the exe's path, so on Windows it is verbatim (`\\?\C:\…`); espeak-ng joins `/espeak-ng-data` onto `--path` with a forward slash, which a verbatim path never treats as a separator, so it missed its data. A debug build uses the source-staged `src-tauri\resources\tts`, a plain path, which is why development never showed it. Reproduced by hand with the bundled `espeak-ng.exe` (the verbatim path exits 1, the plain one prints the phonemes, both with no console window). Speak's Kokoro voices, clones with a Kokoro base and timer lines all go through the same `Phonemizer::from_dir`, so, from the code, every one failed in a built app, and tauri-utils has been 2.9.2 at every tag from v1.17.0 to v1.59.0; no installed Divora was tried here. Fixed in this release (`9934825`): `plain_path` turns `\\?\C:\…` into `C:\…` and `\\?\UNC\host\…` into `\\host\…` (a volume path stays as it is) before `--path`; `args_pass_a_plain_path_never_a_verbatim_one` (CI) and the ignored `espeak_reads_its_data_from_a_verbatim_dir` (the real binary on a canonicalized folder) both fail with the strip removed. L1's second run, on a release build with the fix, rendered the Kokoro line in 1,624 ms and handed it to the engine, which was stopped, so nothing was heard. MANUAL_TESTS' P7 checks the installed app.
- **Done when.** (1) V1 passed. (2) B1 and B3 show Divora's mute set from Start until End now, by click and by key, and v1.56.0's runs measured that mute turning the send to exact zeros; B2, this release's own take of the send, was inconclusive, and P1 (a real call) hasn't been run.
- **Not measured** (S2's gaps; the owner checks P1–P5 and P7 in MANUAL_TESTS → Timers that change your voice (v1.60.0) haven't been run): any sound, since nothing played: the far end of a call (P1), a line or a clip heard (P3, P5), the send in a recording (B2); a Stream Deck (P2); a real sleep (P4); an installed build (P7); a game in front during a line; Kokoro's render time with the engine running or on other PCs; in the app, a late fire, Reset and delete mid-break, nesting, Random preset, A/B compare under a switch, the card's voice line and warnings, a paused break across a restart, the restart notice for a countdown that mutes with no unmute at zero, a hard kill, a save with Replay on or while one runs, End now past a deadline not yet ticked, the mute hotkey during a break, the Presets screen under a switch, a timer's clip at all (no run had one), a full action queue and the keyboard (unit and e2e tests); screen readers; Windows 10; two Divoras at once.
- **Follow-ups:**
  - **A mute that puts back what it was** (owner Q7): a break's end unmutes even a mute you set yourself; a compare-and-set "restore if nobody touched it" needs an engine change.
  - **Toggles as timer actions** (Q8), **count-up actions** (Q9; v1.61.0's `followObs` may want an At start), and **several lines per list** played in turn (D2).
  - **A gain for timer lines** (Q6): they play at a fixed 100 % (only the master soundboard volume scales them), since the worker can't read Speak's stored volume.
  - **Page actions lost in a reload** (K1): a switch (at a start or at zero, Random preset included) or a clip whose event no page heard is lost; only a revert is caught up. Replaying them would need acknowledgements.
  - **Snapshot A/B into a return** (D18), so a timer's switch back restores Compare, and **fix a run's lists at Start, At zero included, and store them for a relaunch** (Q12's alternative; since step R the core keeps a run's start lists in memory, for the undo only).
  - **`readTimerName` refuses no lone surrogate** (D2), the hole a line's reader closes: from the code, a stored name holding one (a hand edit) would make every later `sync_timers` fail, since Tauri's JSON reader refuses the whole call.
  - **One "where push-to-modulate works" rule:** the timer card and the Mixer card each compute it (identical today).
  - **B2 again** with someone talking near the mic, so the take can decide; and P7 before anyone relies on Kokoro in an installed build.
  - **For v1.61.0:** `obs` as a backend kind, and `sceneBack` across a restart (both under v1.61.0's deliverables 6 and 7, below; both shipped, v1.61.0's As built).

### v1.61.0 — Talk to OBS: timer text, break scenes, live uptime (opt-in; 6–9 days)

**Goal.** For people who stream with OBS, let the Hourglass and the Break template reach into OBS:
- a timer's time appears in an OBS text source as soon as it changes;
- a break switches OBS to a chosen scene and back;
- an uptime clock starts when OBS goes live.

It stays **off until switched on**, and nothing connects anywhere while it is off. The `.txt` bridge and the overlay timer are unchanged and stay the default.

**Why a small client of our own, not obws.** obws 0.15.0 is MIT and it works, but four things weigh against it.
- **One maintainer, and an archived repo:** a single crates.io owner, and the GitHub repo is archived (last push 2026-05-03) with a pointer to a self-hosted forge.
- **Size:** 9,673 lines to reach the nine requests Divora sends. (This line said 12, a leftover; the deliverables below list eleven, and v1.61.0 sends nine: its As built, below.)
- **Version gate:** it refuses OBS older than 30.2 unless "dangerous" options are set.
- **No timeouts:** its requests have none.

The protocol Divora needs is small and frozen: rpcVersion 1, unchanged since 2022. The client sits on **the transport obws itself uses**, `tokio-websockets = "=0.13.3"` (MIT, three owners), with `features = ["client", "fastrand", "sha1_smol"]`. Cargo.lock gains +3 crates (tokio-websockets, sha1_smol, simdutf8), against +6 for obws and +13 for tokio-tungstenite.

**Deliverables.**
1. **Protocol core** (`src-tauri/src/obs/proto.rs`, pure).
   - **Envelopes:** ops 0/1/2/5/6/7.
   - **The auth string:** base64(sha256(base64(sha256(password + salt)) + challenge)), using `sha2` 0.10 and `base64` 0.22, both already locked.
   - **Events decoded (six):** ExitStarted, SceneCreated, SceneRemoved, SceneNameChanged, StreamStateChanged and RecordStateChanged.
   - **Requests built (eleven):** GetVersion, GetSceneList, GetCurrentProgramScene, SetCurrentProgramScene, GetInputList, GetInputSettings, SetInputSettings, GetSceneItemId, GetSceneItemEnabled, GetStreamStatus and GetRecordStatus. Nothing else is ever sent.
   - **Unknown events and fields** are ignored.
2. **Version gate, read from Hello** *before* Identify, so Divora never sends an auth string to a server it will refuse.
   - **Floor:** obs-websocket 5.4.0 = **OBS 30.1**, the first version with `sceneUuid`/`inputUuid`. Below it the status says *"Update OBS to 30.1 or later."*
   - **Close codes:** 4009 → "OBS rejected the password". 4010 means the server dropped RPC 1, so it reads *"This OBS needs a newer Divora."*
3. **The link** (`obs/link.rs`): one connection task on `tauri::async_runtime` (Tauri's own tokio runtime), plus one task that runs operations in order.
   - **Dependencies:** src-tauri gains a direct `tokio = { version = "1", default-features = false, features = ["net", "time", "sync", "rt"] }`. `time` is on today only through tauri-plugin-shell. Cargo.lock gains no new tokio entry, but the Windows build newly compiles `mio` and `socket2`.
   - **Connecting:** `TcpStream::connect` to an IP literal (no DNS lookup, ever) with a 3 s timeout, then `connect_on`.
   - **Frame cap:** 4 MiB via `Limits::default().max_payload_len(Some(4 << 20))`; the crate default is 64 MiB.
   - **Keepalive:** a ping every 15 s.
   - **Requests:** each has its own timeout, 3 s on loopback and 5 s on a LAN.
   - **Backoff:** 1, 2, 4, 8, 15, then 30 s, with ±20 % jitter **capped at 30 s**, reset after 10 s connected.
   - **Stops retrying** on 4009, on a Hello that wants auth when no password is stored, or on a version below the floor.
   - **Event mask 69:** General, Scenes and Outputs. Inputs is left out, because it would echo every text push.
4. **Nothing waits on OBS.**
   - **Timer text** goes through a `tokio::sync::watch`, so the latest value wins and a stalled OBS builds no backlog.
   - **Operations** go through a bounded `mpsc(16)` with `try_send`. A full queue drops the operation and the status line says so.
   - **From the std timer thread:** both sends are non-blocking.
   - **While disconnected:** operations are dropped, never replayed.
   - **Engine:** the audio engine and `divora-core` are untouched.
5. **The password.**
   - **Storage:** `obs_set_password` encrypts it with DPAPI `CryptProtectData` (current user, `CRYPTPROTECT_UI_FORBIDDEN`, a fixed app entropy) into `app_data_dir()/obs/password.bin`, via temp file + rename. The FFI lives in `divora-capture::ffi::dpapi` (`Win32_Security_Cryptography`), so the tree keeps its FFI in one unsafe scope.
   - **Use:** it is decrypted only inside a handshake, and wiped with `zeroize` as soon as the auth string exists.
   - **Where it never appears:** localStorage, a log line, an event payload, a command result or a store signal. The password field is cleared after Save or Forget.
   - **A blob that will not decrypt** (after a profile reset) shows *"Re-enter your OBS password."*
   - **The copy** says *"stored encrypted for your Windows account"* and never "secure", because OBS keeps its own plaintext copy.
6. **What Divora does in OBS (all of it).**
   - **Timer text.** A timer definition gains an optional `obsText: {uuid, name}`, delivered to Rust through v1.59.0's `sync_timers`.
     - **What it sends:** the timer thread pushes the same string as the `.txt` bridge, only when it changes, as `SetInputSettings {inputUuid, inputSettings:{text}, overlay:true}`. *(Since v1.59.0:)* through the thread's `TextSink` hand-off, which today fills the file writer's latest-value slot; the `watch` sender goes beside it.
     - **Which sources:** text sources are found by `unversionedInputKind` ∈ {`text_gdiplus`, `text_ft2_source`} from an unfiltered GetInputList, re-fetched every time a picker opens.
     - **What it leaves alone:** a source set to read from a file is labelled *"reads a file — it's using the .txt bridge"* and never switched over.
     - **Reconnect:** the current text is pushed at once.
   - **Scenes.** `ObsOp` = `{op:"scene", sceneUuid, sceneName}` | `{op:"sceneBack"}`.
     - **`sceneBack`** returns to the replaced scene **only if the program is still the scene Divora set**. If you moved on during the break, Divora leaves OBS alone.
     - **Not persisted:** the return slot does not survive a Divora restart.
     - **Timers:** `TimerAction` gains `{kind:"obs", op}`, run where v1.60.0 runs timer actions (its own worker, not `divora-timers`) and handed to the link by `try_send`. *(Since v1.60.0:)* `obs` is a backend kind, run by `divora-timer-actions`: the action table gains one row and the worker's host an `obs(op)` that `try_send`s to the link. `sceneBack` is a restoring action, so End now, a fire found more than 30 s late and a run-out while the PC slept run it; it undoes a start that set a scene, so a Reset or a delete mid-run runs it when that run's At start had a `scene`. A restart re-applies only the mute, never a scene (OBS's program scene doesn't change when Divora restarts). Until v1.61.0, a stored `obs` action is an unknown kind: kept in storage and shown as a row that doesn't run, never sent by the page, and skipped by the backend.
   - **Uptime.** A count-up timer gains an optional `followObs: "stream"|"record"`. It is driven by `outputState` (STARTED, STOPPED, PAUSED, RESUMED); RECONNECTING is ignored. When Divora connects to an OBS that is already live, the clock is seeded from `outputDuration`, which counts frames, so the copy says "about". *(Since v1.59.0:)* that seed needs a core op of its own (a `SetElapsed`, reusing the arithmetic of v1.59.0's one `seed()` helper): a sync's stored run seeds only an id the core hasn't accepted since Divora started.
7. **The Break template gains** *"Switch OBS to [scene] for the break"* (shown only when OBS is on). onStart runs `scene`, and onFire/End now run `sceneBack`.
   - **Follow-up from v1.60.0: `sceneBack` across a restart.** The Break then promises `sceneBack` at zero, on End now, after a sleep, a Reset and a delete, but not across a Divora restart or a run-out while Divora was closed: the return slot isn't persisted (above). v1.60.0 mutes again after a restart mid-break and unmutes at zero, and a break that ran out while Divora was closed leaves you unmuted at the launch, so either way the voice would come back while OBS stays on the BRB scene, the hazard v1.60.0's open question named. v1.61.0 should store the slot (the scene it replaced and the one it set) beside the timer, as v1.60.0 stores a preset's return in `divora.timers`, or say in the Break's copy that a restart leaves OBS where it is.
8. **Settings → OBS** section, after Hourglass.
   - **Consent card,** off by default. It explains:
     - what is sent and what is read;
     - that nothing goes anywhere but the OBS address in Settings;
     - the OBS-side step, because OBS ships with its server **off**: *"In OBS: Tools → WebSocket Server Settings → Enable WebSocket server, then Show Connect Info for the password."*
   - **Fields:** address (IP literals, or `localhost` → 127.0.0.1; default 127.0.0.1:4455), port and password. A status line shows, for example, *"Connected to OBS 32.2.2 (WebSocket 5.7.4) · live."*
   - **Warnings:**
     - a non-loopback address warns that the connection is unencrypted, so timer text and scene names can be seen on the network, though the password itself is never sent;
     - an OBS with no authentication gets *"anything on your network can control it."*
   - **What stays in OBS:** *"Divora never creates, deletes or restyles anything in OBS, and never starts or stops your stream or recording."*
   - **New Sigil:** `link`, drawn in-house, never OBS's logo.
9. **Wire surface.**
   - **Commands, async:** `obs_configure {config}`, `obs_set_password {password}`, `obs_forget_password`, `obs_status`, `obs_inventory`, `obs_run {op}`. *(Since v1.59.0:)* the page sends the ones that change something through one queue of their own in the store (`src/data/inOrder.ts`): commands in flight together can reach Rust out of the order they were sent (v1.59.0's S2), so a password's Save then Forget, or an `obs_run` of `scene` then one of `sceneBack`, could otherwise land reversed.
   - **Event:** `obs-status`, on change only, never periodic.
   - **Wire types:** `ObsConfig`, `ObsStatus` (no field can carry a secret), `ObsInventory` and `ObsOp`. All four join the serde-freeze list.
   - **Key:** `divora.obs` = `{enabled, host, port}`. While it is absent or disabled, **boot invokes no `obs_*` command**.
   - **File:** `obs/password.bin`.
10. **Docs.**
    - **ARCHITECTURE.md:** an "OBS link" section.
    - **README:** *"OBS control (optional, OBS 30.1+) talks to OBS's built-in WebSocket server; obs-websocket is GPL-2.0-or-later and is not part of Divora,"* in the licence paragraph, with tokio-websockets in the crate list.
    - **Changelog:** the Added bullet says it connects only to *"the address in Settings → OBS (this PC unless you change it)."*

**Tests.**
- **Rust, pure (CI):**
  - the auth string against the protocol doc's inputs, with the expected value computed independently (Node `crypto`, Python `hashlib`); the doc's own example output does not match its inputs;
  - the close-code mapping;
  - the version gate: 5.3.3 refused; 5.4.0, 5.4.2 and 5.7.6 accepted;
  - backoff with the 30 s cap;
  - host validation;
  - the return-to-scene rule;
  - the follow mapping;
  - text coalescing;
  - serde freezes.
- **Rust, fake obs-websocket v5 server (CI, loopback):** built on tokio-websockets' `server` feature as a dev-dependency. It runs Hello/Identify with a challenge, and its scene collection includes a bare `text_gdiplus` (v0), a `text_gdiplus_v3`, a `text_ft2_source_v2` and a `browser_source` that must be filtered out. Cases:
  - a happy handshake;
  - a wrong password → 4009, with no retry storm;
  - auth requested with no password stored → closes without an Identify;
  - the server killed → reconnect;
  - an unanswered request times out;
  - 50 text updates while stalled → at most 2 pushes;
  - stream-state follow;
  - disabled → zero connection attempts.

  These run in real time with short injected timeouts, and each test builds its own runtime (no `#[tokio::test]`, which would pull tokio-macros and syn).
- **Secrets:** a TRACE-level capture proves no password, base64 secret or pushed text reaches a log line or an `ObsStatus`. A DPAPI round trip runs too, returning early if the runner has no loaded profile.
- **Vitest and e2e:**
  - a default boot invokes no `obs_*` command;
  - the consent card renders off;
  - `obs-status` states render through `__E2E_EMIT__`;
  - the pickers fill from a mocked inventory;
  - command-name freezes.
- **Manual (this machine):**
  - **OBS versions:** 30.0 (refused), 30.1.2, 31.1.2, 32.2.2 and 33.0.
  - **Text and modes:** GDI+ and FreeType 2 text; Studio Mode.
  - **Resilience:** OBS quit and reopened.
  - **Two PCs:** Divora on this machine, OBS on the second PC. OBS installs without a toolchain, and its PC must accept inbound TCP 4455.
  - **Editing in OBS:** a mapped source's Properties dialog while a timer runs.

**Done when.** A screen recording against OBS 32.2.2 on this machine shows:
1. Settings → OBS switched on and the password pasted; the status reads Connected.
2. A 1:00 Break: OBS switches to the BRB scene, your voice is muted, and the mapped text source tracks the overlay within ±0.5 s.
3. End now: your voice unmutes and OBS returns to the previous scene.
4. Recording started in OBS: the uptime Hourglass starts within 1 s, and stops when recording stops.
5. OBS quit and reopened: Divora reconnects within 40 s, untouched.
6. A wrong password: the status reads "OBS rejected the password", after one attempt, not a storm.

**Not.**
- **No starting or stopping of streams or recordings.** A misfired timer ending a live stream is the worst failure available.
- **Not relayed:** OBS's replay buffer, OBS's audio mixer, and source show/hide (each could be a later additive `ObsOp`).
- **No changes to the OBS setup:** no creating, deleting, renaming or restyling anything in OBS.
- **No glyph or MIDI OBS actions** (bindings tail), and no `obs:` hotkey ids, since OBS has its own.
- **Connection limits:** no hostnames, mDNS or `wss://`; no multiple OBS instances.
- **No OBS → Divora control.** That would need a listening server in Divora.

**As built (2026-11-04; S2 2026-11-05).** The design's decisions D1–D42 and risks K1–K28 stand in for the deliverables above; step R's changes (below) override them where they differ. OBS isn't installed on this PC, so nothing here was tried against OBS itself.
- **What shipped.** **Settings → OBS**, after Hourglass, off until you turn it on: what Divora sends, reads and keeps, the step in OBS, **Connect to OBS**, and Address (an IP address or `localhost`; 127.0.0.1 by default), Port (4455) and Password (Save, Forget), each of which works while off and connects nothing then. Turned on, Divora's own client keeps one WebSocket connection to obs-websocket 5.4.0 or later and does three things. **Show in OBS**, on any timer's card, puts its time into a Text (GDI+) or Text (FreeType 2) source, one `SetInputSettings {inputUuid, inputSettings:{text}, overlay:true}` per change OBS hasn't confirmed. **Switch OBS scene** (At start or At zero) and **Switch OBS back** (At zero) are countdown actions, and a scene picked beside **Add break (BRB)** puts both on new breaks. **Follow OBS** (Streaming or Recording) on a count-up starts it at 0:00 with OBS's start, pauses it with a stop or a paused recording, and sets it from OBS's own count when Divora connects mid-stream. Divora never creates, deletes or restyles anything in OBS, and never starts or stops a stream or recording. The surface: six async commands (`obs_configure`, `obs_reconnect`, `obs_set_password`, `obs_forget_password`, `obs_status`, `obs_inventory`), the event `obs-status`, the key `divora.obs = {enabled, host, port, breakScene?}`, the timer fields `obsText` and `followObs`, the action kind `obs` (`op` `scene` or `sceneBack`, flat like every other kind), and `obs/password.bin` and `obs/returns.json` (`obs-dev/` for `tauri dev`). `Select` gained an `onOpen` and disabled options, `Segmented` an `ariaLabel` that makes it a radio group, and the sigil set a `link`.
- **The client** (`src-tauri/src/obs/`): `proto.rs`, `machine.rs` and `scenes.rs` pure; `link.rs`, two tasks per running link on Tauri's tokio runtime (the driver, and the session that owns the socket), each waiting on a hand-written `poll_fn` with the stop among its sources; `files.rs`; `log.rs`, the only file there that names a `tracing` macro; `mod.rs`'s `ObsShared`, managed by `obs::init` in setup. About 4,500 lines of Rust, not counting comments, blank lines and the test modules, against the 600–800 the Decisions row expected, plus a fake obs-websocket server (`fake.rs`, about 4,300 lines, tests only) written by a different hand from the protocol document and obs-websocket's sources. Nine requests, built only through `proto::Request`, at most one outstanding per connection even after a timeout (its late answer is waited for up to 15 s from sending, then the connection ends); six events, under mask 69 (General, Scenes, Outputs). Divora reads the version from Hello before any Identify, so no password string goes to a server it refuses, and always Identifies with RPC 1. It connects only to a `SocketAddr` made from the IP and port in Settings → OBS (`the_obs_link_never_resolves_a_name`); no production code in Divora listens (from the code).
- **Nothing waits on OBS.** Text: the timer thread's sink stores each feed in a `watch` beside the files' slot (latest wins), and the link pushes one source per turn, re-reading the newest feed first. Scene ops: `divora-timer-actions` hands each to `ObsShared::offer_run`, which calls `offer`, which, without waiting, `try_send`s it into a connected link's queue of 16, or else records a skip or a due mark. The engine and `divora-core` are untouched.
- **Slots outlive links** (owner Q1, Q2). Which scene each timer's start replaced lives in `ObsShared` for the whole run, loaded in setup from `obs/returns.json` (24 hours at most; written by temp file and rename through one latest-wins saver), so a reconnect, Try again, a saved password or a Divora restart keeps a break's way back. A switch back that finds OBS not connected is due for up to 2 minutes from the moment it was offered, while Settings → OBS is on; a switch *to* a scene is never made late (not while disconnected, and not when the link gets to it more than 5 s after it was offered).
- **The password**: DPAPI for the current user with a fixed entropy, in `divora-capture`'s `native::dpapi`, with crypt32.dll loaded at run time (the import guard refuses a load-time import of it); decrypted only when OBS's Hello asks for auth. Divora wipes its own copies, the plaintext, the derived secret and both digests, with `zeroize`; sha2's internal buffer, the IPC message and the WebView's field are outside its reach.
- **Status and order.** One `ObsStatus` with a `seq`, emitted on change only (scene-list changes at most once per 250 ms) and answered by every OBS command; the page keeps the larger `seq`. No field carries a secret or free text from OBS. The page sends the four commands that change something through a sixth queue, `obsQueue`; none of them waits on OBS, and only `obs_inventory`, a read, does (15 s at most).
- **Plan deltas** (the design's §2.1):
  - **Nine requests, not eleven** (deliverable 1): GetSceneItemId and GetSceneItemEnabled served source show/hide, which the plan had dropped. "Why a small client" said 12, corrected above.
  - **More stops** (deliverable 3): 4010 (`needsNewerDivora`) and 4011 (OBS's own disconnect from its list of clients, after which the protocol says a client must not reconnect on its own) stop retrying too, as do a password Windows can't read and an `unsupported` server (a version Divora can't read, or one of the nine missing from GetVersion's list).
  - **Output states** (deliverable 6): the wire's values are `OBS_WEBSOCKET_OUTPUT_*`; PAUSED and RESUMED come only on recordings; a stream's RECONNECTED sets the count-up again from GetStreamStatus; STARTING, STOPPING, RECONNECTING and UNKNOWN do nothing.
  - **`obs_run` waits for v1.64.0** (deliverable 9, owner Q8), with the MIDI and glyph OBS actions that would send it: nothing in this release would. `obs_reconnect` (Try again) is new.
  - **Switch OBS back waits up to 2 minutes** for a connection (owner Q2), where deliverable 4 dropped every op while disconnected; a switch to a scene still never runs late.
  - **The slot survives a restart** (owner Q1), where deliverable 6 didn't keep it: deliverable 7's follow-up from v1.60.0 is closed, a run-out while Divora was closed included.
  - **The version copy** names the WebSocket server: "Divora needs 5.4.0 or later" (owner Q7). That 5.4.0 came with OBS 30.1 isn't in any source read here, so "OBS 30.1" waits for U9.
  - **DPAPI under `native/`** (deliverable 5): `divora-capture` has no `ffi` module; its one unsafe scope is `native`.
  - **No `select!`** (deliverable 3): tokio keeps `net, time, sync, rt` and no `macros`, so the tasks wait with a hand-written `poll_fn`, and the link is two tasks, a driver and a session.
  - **A keepalive deadline:** a ping every 15 s, and a connection that sends nothing at all within 10 s of a ping is dropped; the plan named no deadline.
  - **`sceneBack` asks GetCurrentProgramScene** when it runs, rather than following CurrentProgramSceneChanged, which can cross a request's answer.
  - **The seam:** text comes from `divora-timers`, every text on every dirty pass, and the link diffs it per source; ops come from `divora-timer-actions`. Deliverable 4 had both from the timer thread, "only when it changes".
  - **A source that reads a file** is offered and can't be picked ("reads a file — untick Read from file in its Properties in OBS to show a timer here"), rather than labelled as the `.txt` bridge.
  - **About's No telemetry card** (owner Q4) reads "Nothing is collected or sent home. Audio stays on device." It said "Nothing leaves your machine", which OBS on another PC would make untrue.
- **Changed by the code review** (step R, `7c9dece`: 8 lenses, 34 findings, 27 confirmed by a skeptic and fixed, 7 refuted; the §10.8 audit's 5 weak rows each gained a test seen failing on its mutation, and every fix's mutation was put back and caught):
  - **A break started again** is never switched back by its run before: a start's `scene` drops that timer's earlier switch backs, due or in flight (`Slots::started`). A mark stamped before the latest start is ignored, and a re-marked switch back keeps its first stamp.
  - **The waiting line** counts a switch back that is taken and not yet answered, so a 207 retry doesn't drop it to 0 and repeat the announcement.
  - **A parked link wakes** for a mark made while it isn't connected (`LinkCmd::Due`), so the mark expires on time; marks whose 2 minutes ran out while no link ran are dropped at the next start, with no skip line; switch backs still in an ending driver's queue are due again with their own stamps, and a replaced driver that leaves one due tells the newest link.
  - **Follow OBS:** a status answer that an output event overtook isn't applied (the event's word stands and the follow is seeded again), and a seed applies only to follows still in the feed.
  - **The session** doesn't read while its own writes can't drain, so a peer that pings and never reads is cut off by the keepalive; more than 64 control frames in a row before Identified read as not OBS.
  - **The page:** leaving `connected` clears OBS's lists, so a reconnect's pickers offer only what is stored until the new connection's list answers; Switch OBS scene asks again once if OBS is connected again, else its note says "Couldn't read OBS's scenes: OBS isn't connected."; `Select` keeps its highlight on the same option when the list is rebuilt under it; a control that removes itself hands the focus on; the explainer's "keeps only…" sentence names the scene a timer switched OBS away from.
  - **Known, not changed:** a scene-collection switch inside one connection keeps the old lists on offer until a request fails or the next connection (mask 69 has no Config events); after a 207 for an earlier run's switch back, a later expiry of the new run's mark can read `notReady` instead of `notConnected` (low).
- **Smaller differences while building** (from the code): a switch back offered while Settings → OBS is off marks nothing (turning it off drops marks anyway; before the first configure of a launch it is still marked, for a run-out found at boot); a startup step that times out ends the connection as `noAnswer`; the 5 s Identify deadline covers the password's decrypt; `ObsShared` holds its link handle's lock while it takes another of its locks, such as the status's or the slots', never the reverse (the design had no lock held while another is taken). The log lines' words differ from the design's list (§9.5): "obs: connected", "obs: can't reach OBS; trying again", "obs: OBS didn't answer in time", "obs: a switch back waited too long and was dropped".
- **Tests** at `7c9dece`, on this PC (`CI=true cargo test --workspace --all-features`): divora-app's lib 480 passed and 1 ignored, divora-capture's 181 and 1, divora-core's 562 and 17, and the integration binaries; Vitest 2,120 tests in 60 files; Playwright 75, with Tauri mocked. Each track's tests were seen failing on the mutations in its table, and step R's on theirs.
- **Owner decisions** (2026-11-03): every default. Q1, the slot kept in `obs/returns.json` across restarts; Q2, a 2-minute grace for Switch OBS back, never a late switch to a scene; Q3, a source's text cleared once when a timer stops showing there, within one run of Divora; Q4, About's card; Q7, the copy names WebSocket 5.4.0 or later; Q8, `obs_run` deferred and `obs_reconnect` shipped; Q10, a count-up that follows OBS keeps its own buttons and keys. Q5: yes to one portable OBS of the current release, downloaded into the scratchpad once its size is shown and confirmed again, never installed, deleted after, and not OBS 30.0 or 30.1.x (U9 stays the owner's); it hasn't run, so U1–U15 stand as owner checks. Q6: yes to S2, with a `1c537fa` release build as T10's baseline, confirmed again right before it runs; T8's and T12's Quit from the tray by UI Automation: yes. Q9 and Q11 (not asked) take their defaults: U11 after the tag, and no firewall rule.
- **The real-app run (S2, 2026-11-05, 16:08–16:27 UTC, about 19 minutes with the app running),** on this machine, against a Node stand-in OBS server on 127.0.0.1:4455 only, written from the protocol document and obs-websocket's sources by a third hand (OBS isn't installed here). The stand-in announced OBS 32.0.2 / WebSocket 5.6.2, with authentication on and a throwaway password made for each run. Every run used a debug build of `7c9dece` with the copy-only fixes made after it (frontend strings and a lib.rs comment), and T10 a release build of `7c9dece` too, with the engine stopped. Playwright drove the app's own UI over WebView2's DevTools port, with no key events, behind a guard page that refused any engine start off the route, any hotkey registration and any OBS connection but 127.0.0.1:4455. The silent route: the Focusrite in, the Realtek USB speakers out, no monitor device, Monitor off; nothing played, and no CABLE device. T8's two Quits and T12's one came from the tray: UI Automation found the icon and two mouse clicks (`SendInput`) chose Quit, with the owner's consent given right before. Every run but T10 and T11 (which have none) had a harness mutation, and each failed, as it must; storage, the `obs/` and `timers/` folders and the window's position were put back after every run. Each run's setup, pass line and figures are in MANUAL_TESTS → Talk to OBS (v1.61.0).
  - **T1, T2, T5, T11** (a boot without `divora.obs`, a password saved, a wrong password, Off): T1 passed (no `obs_*` command, nothing listening for `obs-status` and no connection at the stand-in in 60 s); T2 passed (connected with the stand-in's versions, and `password.bin` holds neither the UTF-8 nor the UTF-16 password); T5 passed: one connection, closed 4009 by the stand-in, then none in the 60 s after; T11 passed: the open connection closed with 1001 from Divora 13 ms after the click on Off, then none in the 60 s after.
  - **T3, a timer's text, Divora hidden:** passed: 60 of 60 seconds arrived at the stand-in, one push each, 10 ms after each second's start at the median and 17 ms at worst (none early: the earliest came 2 ms after its second), and Divora's window stayed hidden throughout.
  - **T4, a break with a scene:** passed: the Set (BRB) reached the stand-in 16 ms after the click on Start and the Set back 16 ms after the click on End now, each after a GetCurrentProgramScene; the first muted level tick came 26 ms after Start and the first unmuted 20 ms after End now. A second break's End now, with the stand-in's program moved to another scene, asked and made no Set, and every Set named its scene by uuid only.
  - **T6, OBS away and back:** passed when re-run alone. Its first run failed one harness check for a harness bug: attempt 3's `connecting` and `retrying` statuses reached the page in the same millisecond, and the harness paired the failure with the wrong attempt (the stand-in's own accepts showed every attempt on time). In the re-run every wait sat inside the backoff table's 1, 2, 4, 8 and 15 s × 0.8–1.2: 1,037, 2,188, 3,626, 9,064 and 15,229 ms while the stand-in accepted and closed, then 973, 1,768 and 3,445 ms after a connection held 11.5 s had reset the count; each attempt started 5–13 ms after its `retryAtMs` (6–13 ms by the stand-in's accepts). Connected 11,202 ms after the stand-in served normally again, and 269 ms after it came back with a fresh salt; the mapped text was pushed 1 ms and 0 ms after each Identified. With nothing listening, both failed connects read `refused`, and each took about 2 s on this PC (2,013 and 2,031 ms; 2,056 ms twice in the first run), under the 3 s connect timeout (D8).
  - **T7, Follow OBS:** passed: the count-up's `timer-state` read running 1 ms after the stand-in's STARTED, from 00:00, and PAUSED, RESUMED and STOPPED each took 1 ms; a bare `"STARTED"` changed nothing; joining while the stand-in reported 125,000 ms of recording read 02:05.
  - **T8, a restart mid-break:** passed. A, a Quit mid-break and a relaunch at once: the break muted again (162 level ticks after the boot sync, none unmuted), and End now's switch back reached the stand-in 16 ms after the click, from the slot in `obs/returns.json`. B, Divora closed 30,918 ms while the break ran out: the page sent `sync_timers` before `obs_configure` (the order sent, from the IPC log; which reached Rust first wasn't recorded), and the switch back reached the stand-in 2 ms after Identified, after a GetCurrentProgramScene that read BRB.
  - **T9, a stalled OBS:** passed: while a SetInputSettings went unanswered for 10 s, exactly 1 push arrived and nothing else was sent, and the stand-in never saw two requests outstanding; the scene offered 2 s into the hold was skipped `late`, and the one offered 1 s before the release ran 1 ms after it (1 Set); the next push carried the newest text (04:48, not the held 04:57); and the app log held one "obs: OBS didn't answer in time" line.
  - **T10, a release build:** passed, with the engine stopped: a saved password connected; `password.bin` holds neither the UTF-8 nor the UTF-16 password; after a relaunch the link connected on it with no `obs_set_password` sent (DPAPI read it back). The release exe imports the same 25 DLLs as v1.60.0's (`1c537fa`), none added or removed (11 more functions within them), with no delay-load import, and crypt32.dll is neither imported nor delay-loaded. Release link time on this PC (i7-14700K; `lto` + `codegen-units = 1`): 84 s against v1.60.0's 79 s, same PC, same day (+5 s, +6 %); link.exe itself is 0.2 s of it.
  - **T12, Quit while connected:** passed: the open connection closed with 1001 from Divora 35 ms before the process ended, 7 ms and 41 ms after the click on Quit.
- **Done when.** The plan's evidence, a screen recording against OBS 32.2.2 on this PC, can't exist without OBS. At the socket, against the stand-in: (1) on, a password, Connected: T2 passed, and U1; (2) a 1:00 break: T4 passed for the scene and the mute and T3 passed for the text, at the stand-in, not on screen, and U2; (3) End now: T4 passed, and U2; (4) a recording's uptime within 1 s: T7 passed, and U13; (5) OBS quit and reopened, connected again within 40 s: T6 passed on its re-run (connected 11,202 ms after the stand-in served normally again, and 269 ms after it came back), and U5; (6) a wrong password, one attempt: T5 passed, and U6.
- **Not measured** (the owner checks U1–U15 in MANUAL_TESTS → Talk to OBS (v1.61.0) haven't been run): OBS itself, in any version: a pushed text drawn by OBS, GDI+ against FreeType 2, the Properties window, Studio Mode, OBS's real version strings and the 30.0 refusal, a real `outputDuration`, GetStreamStatus while a stream reconnects, whether OBS answers pings, the code OBS sends for a source it can't find, OBS's alerts, how OBS times its events, a scene-collection switch, a network and its firewall. The fake and the stand-in show Divora's side of the protocol as the document and sources state it; only U1–U15 show OBS's side. Also a real sleep with a following count-up, screen readers, Windows 10, and two Divoras at once (an installed one and a `tauri build` one share `obs/`). Every S2 run measured what its pass line asked; outside them: which of `sync_timers` and `obs_configure` reached Rust first (T8 recorded the order the page sent them), any address but 127.0.0.1 (the guard refused the rest), the mute at a device (S2 read it from the engine's level ticks, with nothing played), and an installed build (T10 ran the release exe from `target`, with the engine stopped).
- **Follow-ups:**
  - **OBS actions in MIDI and glyph bindings, and `obs_run`** with them, in v1.64.0 (owner Q8).
  - **Inputs events,** to see a shown source deleted or renamed as it happens: mask 69 leaves them out, since they would echo every text push, so a missing source shows only as a failed push.
  - **Config events,** for a scene-collection switch, which today shows only as failed requests until the next connection (step R's known item above).
  - **Pings and pongs,** after U10: if OBS answers pings, the keepalive could wait for a pong rather than any frame.
  - **The OBS 30.1 wording,** after U9 (owner Q7), and the 3 s connect timeout against T6's refused-connect time on this PC (2,013 and 2,031 ms, and 2,056 ms twice in the first run: about 2 s, about 1 s under the timeout).
  - **A file of the sources Divora wrote** (owner Q3's alternative), so a source unmapped while Divora was closed is cleared too.
  - **Two breaks to the same scene nest** (the What's-new gate, round 1): today, if the break that started first ends first, it switches OBS back while the other still runs. The fix needs `Slots` to know which timers are still running, so a stale slot (a countdown with Switch OBS scene and no Switch OBS back) isn't mistaken for a running one.

### Tail releases (optional; each its own minor; the plan can stop before any of them)

The tail was one row, "v1.62.0 Charms everywhere": one overlay window per monitor, text and video charms, snapping, recording a single window and a small "recording" charm. Sized against the code on 2026-11-07, it was about 49 design-days as one release, against v1.55.0's 19.5 and v1.61.0's 14, so it is split into six minors (owner Q1). The order puts first what this PC can measure, and last what multiplies everything else and needs the owner's hardware (mixed DPI, hot-plug): per-screen windows are then designed once, over the final set of charm kinds and the final hiding rule. Window capture comes before the border spike, because the border is drawn around a recorded window too. Each later minor is designed in full in its own session; the calls it needs early were settled in v1.62.0's design (D22–D61) and are asked again when it starts.

| Release | Scope | Gate |
|---|---|---|
| v1.62.0 Text charms | Text charms typed in Settings, snapping in Arrange, and the recording light (the row's small "recording" charm). Built: see below | Everything it claims measurable on this PC (S2: W1–W7 and W9; W8, the optional older-build run, skipped at the owner's choice) |
| v1.63.0 Video charms | MP4 (H.264) and WebM (VP8 or VP9, with or without alpha) charms, checked by their content when imported, always muted; the `charm` scheme answers byte ranges; at most 4 video charms | Cost gates set before measuring: one 1080p30 video drawn large, four looping, one VP9 with alpha. The sound never plays, checked in the real runtime |
| v1.64.0 Record a window, with R15 | Record a single window (`CreateForWindow`), picked in the Record screen card; Replay stays a screen both ways (no window while Replay is on, no Replay during a window take); R15's hiding by the recorded source and monitor, with an additive status key, `hideApplies`, which the recording light's gate then reads | A window's resizes, minimizing and closing (a closing window ends a take and keeps the file, so the light's self-ending edge is measured here); whether a window take ever contains the overlay |
| v1.65.0 Remove the yellow border (Windows 11) (was v1.63.0) | Spike: test `IsBorderRequired(false)` with *and* without `RequestAccessAsync(Borderless)`, and ship only if the border **visibly disappears** for the unpackaged app. Applies to takes and Replay. Client availability is Windows 11 22000+, gated by `IsPropertyPresent` | A drop outcome is acceptable |
| v1.66.0 Charms everywhere | One overlay window per screen the streamer turns on, up to 4 with the current one and never two on a screen; each placed in physical pixels (which already ships), moved onto its screen before it is sized; a screen known by Windows' stable monitor id, its connector path, its EDID ids, its `\\.\DISPLAYn` name and its overlap, never the name alone; a window whose screen goes is parked, not closed | Placement across monitors under `WM_DPICHANGED`, and a display watch of Divora's own (tauri and tao have no monitor-change event); cost gates set before measuring (a second window idle, the watch, both drawn large). Mixed DPI and hot-plug are owner checks |
| v1.67.0 More timers and bindings (was v1.64.0) | Laps, interval ("voice roulette"), wall-clock targets with clock-jump detection, the titlebar timer pill; MIDI and glyph actions for timers, Replay save (`replay`) and OBS scenes (`obs`), with router edge detection so a fader cannot fire a save repeatedly | — |

**Corrections to the row as it was** (v1.62.0's design, C1–C9, from the sources):
- **C1, its gate.** tauri #11718 (closed 2024-11-19, fixed by tao #1018) and #6784 (closed 2023-04-25 as a duplicate of #5228, which is still open) are about the units of a window's position when it is created, not hot-plug, and Divora places the window after building it, so it avoids both. The gate is now placement across monitors under `WM_DPICHANGED`, and the missing monitor-change event (v1.66.0's row).
- **C2, "monitor stored by name".** The name is `\\.\DISPLAYn`, which Windows renumbered on this PC between 2026-10-18 (DISPLAY11 and 12) and November (DISPLAY21 and 22).
- **C3, "physical-pixel placement".** It ships since v1.51.3 and v1.55.0; v1.66.0 adds it per window, with the move before the size.
- **C4, the `overlay*` capability glob.** The capability gates only core and plugin permissions, `overlay*` would also match `overlayfoo`, and the label is matched exactly in six places (a test asserts `overlay-2` gets a 403 from the `charm` scheme). v1.66.0 names `overlay` and `overlay-2` to `overlay-4` and checks a label in one function.
- **C5, "one WebView2 per monitor".** Divora's webviews share one user data folder, so one browser process; whether windows share a renderer is Chromium's choice. v1.66.0 measures two windows and caps them at four.
- **C6, a video's sound.** WebView2 runs with autoplay allowed without a gesture, so an unmuted video would play its sound outside Divora's engine, its mute and desktop audio's process exclusion. Video charms are always muted.
- **C7, two Decisions rows:** "Locked makes the window layered" and "four exits", corrected in the table below.
- **C8, rotation.** v1.55.0's "Not" named it, and the tail row dropped it without saying so. It is now under Not building (owner Q19).
- **C9, "record a single window".** The row said nothing of Replay, whose rules are keyed to a monitor. Replay stays a screen both ways (v1.64.0's row).

### v1.62.0 — Text charms: your words and a recording light on the overlay, and snapping

The tail's first minor (owner Q1; Q2 moved the name "Charms everywhere" to v1.66.0, when charms reach every screen).

**Goal.** Type a short text in Settings and show it on the overlay in one of Divora's own fonts and in colours OBS's key presets leave alone; snap charms into line while arranging; and show a small light that says when Divora is recording, without putting a text field on a window captures can see.

**Not.** No text typed on the overlay; no font size control (the box sets the size); no automatic wrapping; no free colour picker; no network fonts and no list of Windows' fonts to pick from (letters the bundled fonts lack fall back to them); no links, markup or animation; the light isn't a window of its own. No video charms, window capture or per-screen windows (later minors).

**As built (2026-11-07, `f5e02e6`; step R 2026-11-08; S2 2026-11-09).** The design's decisions D2–D21 and risks K1–K19 stand in for deliverables; where the code differs, it is below. OBS isn't installed on this PC, so nothing here was tried in OBS itself.
- **What shipped.** Settings → Stream overlay gains **Add text…** and **Add the recording light**. A text charm holds up to 200 UTF-16 units on up to 5 lines, typed in an editor under the charm list (never on the overlay), with a count, a preview of the look, one of the three bundled fonts (Display = Bricolage Grotesque, Sans = Space Grotesk, Mono = Space Mono), regular or bold, one of eight colours, an optional outline, and left, centre or right alignment. It is placed and sized in Arrange like a picture, its box keeping the text's shape. In Arrange a drag or a corner resize snaps within 8 CSS px to the overlay's and the other charms' edges and centres, with a guide; Ctrl places freely; keys never snap. The light shows "● REC" and the take's time during a take and "● REPLAY" while Replay arms or is armed; with nothing to show it draws nothing while Locked and a dimmed "● REC" in Arrange; with Hide from recordings on and the overlay Locked, it draws only while the page's own status says `capturing`. Settings → Appearance's **Color mood** is now **Colour mood**. The surface: element types `text` (`text`, `font`, `bold`, `color`, `outline`, `align`, `nw`, `nh`) and `rec` in `divora.overlayScene`, whose `version` stays 1 (v1.55.0–v1.61.0 keep both, unshown); a main-only `set` op that never crosses the wire; the add refusal `light`; fixture rows `textCharm` and `recLight`; and the overlay page now hears `screen-recording` and `replay-buffer` and calls `screen_recording_status` (sync) and `replay_buffer_status` (async) once after its handshake, which `capabilities/overlay.json`'s description names (no permission change). New pure modules `text.ts` (imports nothing), `snap.ts` (type imports only) and `recLight.ts` (imports nothing), and `textMetrics.ts`, the shared canvas. Kill switches: `TEXT_CHARMS`, `SNAP_PX` and `SNAP_BYPASS`, `REPLAY_FACE`.
- **No Rust change but two tests:** `fixture_names_the_recorder_events_the_overlay_hears` (`overlay_machine.rs`: the two event names and the fields the overlay reads, against the fixture) and `the_overlay_recorder_commands_are_registered` (`lib.rs`: both command names in `generate_handler!`, one sync and one async). No new command, event, storage key or file.
- **Differences from the design** (from the code; each has its tests):
  - **A new text or light is born the size `place` stores** (§7.8): after the 90 %-of-the-width cap, its short side is raised to 24 px where the window allows, then the box is fitted inside the design. A long one-line text can be wider than 90 %, up to the window's whole width, and a one-letter one taller than 10 %. Without it, such a text was born under 24 px tall, and its first drag or nudge became a resize to the full width.
  - **Half a surrogate pair** on its own becomes U+FFFD in `normalizeCharmText`, so in Add, Edit and Save, and in `readScene` (`wellFormedText`) when a stored scene is read: left in a text, it made every `overlay:scene` push fail at the IPC, whose JSON reader (serde_json) refuses the lone half's escape. It is the only change reading makes to an element.
  - **`set` keeps the aligned side where the overlay window draws it** (an optional `viewport`, which main fills with the window's size; the design size when fitted), not in design fractions, so an edge-snapped label stays on its edge in a window narrower than the design. In a window of the design's size, for a box stored inside the design, it is drawn where keeping the design's fractions would draw it (step R's rounds 3 to 5, below, keep the design's edge too).
  - **Room for ink:** `textBox` adds room above the first line and below the last for ink that reaches past the line's own box (in Display, Vietnamese capitals' stacked marks reach about 0.25 em above it), on top of the 0.12 em padding, from the canvas's actual and font bounding boxes (`textPads`); the face is padded to match. Latin text is unchanged.
  - **A corner-resize snap moves each side at most 8 px** from the preview: a line that would move the derived side further (snapping a 5 : 1 light's bottom edge 8 px would move its right edge 40 px) is passed over for the next. Its minimum is `resizeRect`'s own floor, not §7.5's `min(24, rect0's short side)`.
  - **Snapping starts 3 px from the press** (`SNAP_SLOP_PX`): a click that only selects a charm never snaps it; from then on the gesture snaps until it ends.
  - **The light waits for Space Mono Bold** as a timer face does (at most `FONT_WAIT_MS`), its Arrange placeholder too.
  - **The backstop reads the laid-out size** (`offsetWidth` and `offsetHeight`, with 1 px of slack), not the scroll size: Chromium counts each font's ascent and descent in `scrollHeight`, and Space Mono Bold's (about 1.47 em) read 1–2 px over a face that fitted.
  - **A stale `place` is a move only if its shape is off both the text's own shape and the shape it is laid out in,** so a text stored off its own shape (a hand edit) can still be resized.
  - **The overlay's measure has no cap** (`canvasMetrics(Infinity)`); Stage keeps its own 1 s wait and measures again when a slow font arrives. New words keep the face before them until their font is ready.
  - **The editor:** a refused Add (the overlay full) or a failed save keeps the editor and its draft (the store's two text calls answer whether the editor is done), with the focus put back on Save if it had fallen to the page; Edit on the row already open, or Add text… while adding, keeps the draft; a restack keeps a focused row button focused; **Add the recording light** and **Add the spell circle** hand the keyboard focus to **Add text…** as they go.
  - **Smaller:** `[data-text-face]` carries the text's styles, with one child holding the padding and the words; the snap hint shows only while `SNAP_PX > 0` and names `SNAP_BYPASS`'s key; guides carry `data-at`; the light's placeholder is `[data-rec-idle]` alone; `snapTargets` takes the laid-out charms and the dragged id; the source scan for HTML parsing is in `Stage.test.tsx`, not `state.test.ts`; a Ctrl, Alt or Shift key event mid-drag re-runs the last pointer point, and only Ctrl frees it.
- **Owner decisions** (2026-11-07): every default. Q1 the split; Q2 the name; Q3 and Q4 the light in the overlay, showing REC and REPLAY; Q5 the three bundled fonts; Q6 eight colours checked in OBS's own key distance (Violet gone) and "Colour mood"; Q7 200 characters and 5 lines, as creation limits; Q8 Ctrl; Q9 S2 as planned, with consent asked again right before it runs (given on 2026-11-09, right before S2, with W6's Hide on, one Replay clip and the stand-in; W8, the older build, skipped at the owner's choice); Q19 rotation under Not building. Q10–Q18 and Q20 are recorded as defaults for the later minors and will be asked again when each starts.
- **Changed by the code review** (step R; round 1 nine lenses, 24 findings, 20 confirmed and 4 refuted, 47 mutations caught; round 2 four lenses, 7 raised and confirmed, 22 mutations, 21 caught and the other given a test; round 3 three lenses, 2 raised and confirmed, 4 mutations; round 4 two lenses, 8 raised and confirmed, 15 mutations; round 5 two lenses, 2 raised and confirmed, 5 mutations): round 1's fixes are eight of the differences above (the 24 px birth size, half a surrogate pair, `set`'s viewport, the room for ink, the corner-resize bound, the 3 px start, the light's font wait, and the editor's refusals and focus), and in the copy: the light's help says "(not while it's held)"; its Hide line is scoped to the locked overlay and says that while you arrange it's drawn and recordings can see it; the editor's Edit sentence says "where it can"; the editor's reason is tied to the Text field and said aloud as it appears. Round 1 raised 24 findings and confirmed 20, which come down to 13 defects. Round 2 (7 raised, 7 confirmed): the overlay page measured a text with no Latin letter in it (Cyrillic or Greek alone, say) before its font's first face had loaded, so it padded the face by a Windows font's ascent and descent where Settings had padded it by the bundled font's (its load now asks for that face too, and e2e E1c measures six such texts); `set` still clamped the edited box in design fractions, which undid the window rule in a window wider than the design or one that draws the text shrunk (it then kept the aligned side and the vertical centre inside the window the text is drawn in); a refused Save (the overlay full) left the focus on the page (it now goes back to Save, or to the Text field while Save is off); and comments and the design that still stated the old rules were brought up to date. Rounds 3 (2 raised, 2 confirmed, both low) and 4 (8 raised, 8 confirmed: 1 medium, 7 low): a text edited while the overlay window is smaller than its design kept to the edge where the window draws it, but could leave the design's own edge, so a label on the design's edge (its top or bottom edge too, round 4's medium) sat off it once the window grew back. `set` then stored, when that window draws the old box pushed in from past an edge or across its whole width, of the positions that draw the edited box in the same place there, the one nearest the design's own rule (down, for a box drawn pushed in, its stored vertical centre), and the rule's tolerances, its comments and its tests were tightened; round 4 also stopped a hand-edited position of about 1.8e302 or more from becoming infinite on an edit, which made the text stop drawing. Round 5 (2 raised, 2 confirmed, both medium): down, where the window didn't draw the old box pushed in, `set` still stored the centre that drew the edited box inside the window, so a label lined up on a window's top or bottom edge, edited to words the window draws taller and edited back, ended 10 to 13 px off that edge, and a full-width one on the design's top or bottom, edited in a smaller window, 11 to 44 px off the design's edge once the window grew back. `set` now always keeps the stored vertical centre, taken inside [0, 1], which layout draws kept inside every window, so the rule above applies across only (scene.test's "set keeps a label lined up on a window's top or bottom there when an edit draws it taller, and edited back" and "set keeps a label on the design's top or bottom there when a smaller window draws its edit taller"). Its fix was broken 5 ways, among them each earlier rule for the top and bottom put back, and each failed a test.
- **Known, not changed:** a take stopped while it is still starting shows "● REC 0:00" until it ends (a take reports no time before it records); a text whose words are exactly an asset id keeps that file (K10); in an unfitted window that is resized, edges lined up there can drift apart, since layout keeps centres (D14).
- **The real-app run (S2, 2026-11-09, 13:47–14:32 UTC, about 45 minutes, about 35 of them with the app running),** on this machine (Windows 11 Pro 26200, i7-14700K, RTX 4090; the primary screen 3440×1440 and screen 2 1920×1080, both at 100 %), in a debug build of `c9f066b`, step R's commit (its later rounds changed where `set` puts an edited text, after S2; W2's edits ran in a window the size of its design, on boxes inside it, their height unchanged and narrower than the window, for which the later rule stores exactly what `c9f066b` stores, from the code), on screen 2 only, driven by Playwright over WebView2's DevTools port with no key pressed and no tray click, the silent route (the Focusrite in, the Realtek USB speakers out, no monitor device, Monitor off; no CABLE device; nothing played) and the whole of `localStorage` (11 keys) recorded and put back after every launch (the final restore read every key "same"); the main window's position was put back, and the 13 take, clip and stand-in files were deleted by their literal paths after they were analysed. The owner agreed right before it ran, W6's Hide on, its Replay clip and the stand-in included, and chose to skip W8. Three harness bugs, not the app's, were fixed during the run: a `localStorage` restore lost because the harness killed the app before Chromium had written it to disk (the harness now closes the browser over the DevTools protocol first), W9's states measured on different pages (below), and the stand-in's decode deadlocked on ffmpeg's warnings (now drained by a thread). Each run's setup, pass line and figures are in MANUAL_TESTS → Text charms (v1.62.0).
  - **W1, six texts:** passed 111/111: every face in its stored family, weight and colour, inside its row (±1 px), its bound side at least 99.9 % filled, and `data-fit-backstop` 0 on all six; `<b>hi</b>` drawn as those characters. The Latin texts came entirely from the bundled fonts; the other letters from Windows fonts: Segoe UI for the Cyrillic and Greek (Consolas in Mono), Microsoft YaHei for the Chinese and Segoe UI Emoji for 🎮. **Save** → the face drawn in 8 to 30 ms (worst 29.6 ms). The `font: "mono"` mutation failed it.
  - **W2, two edits:** passed: the same id and `z`, `h` and the aligned side (the centre, and the right edge) equal to 1e-6, and the face updated 8.2 and 5.7 ms after **Save**; an overlay drag with the editor open moved the text and stored it, and kept the editor on that text, its field, the draft, the caret and the focus.
  - **W3, snapping:** passed: a drop 5 px off the centre line stored the centre at 0.5 (±1e-6), with a guide; a drop near the circle's left edge put the text's left edge on it (259.98 px both), with a guide; with Ctrl, no guide, stored where dropped. The 9 px mutation didn't snap, so it failed, as it had to.
  - **W4 and W5, the light in takes with Hide off, and the takes decoded:** passed: the light's words within 0.1 ms of each of the 20.6 s take's 24 events (the worst), and gone within 0.1 ms of `idle`; a take stopped 0.70 s after **Record** showed "REC", then "REC 0:00"; a page reopened mid-take read the take's status and drew "REC 0:04" before the next event. Decoded: the dot in 607 of 607 frames, 14 of 14, and 101 of 101 before the overlay's close and 185 of 185 after its reopen, the reopen itself 45 frames (about 1.5 s) without it. Whether "m:ss" is the floor of `elapsedMs` wasn't measurable: no report's fraction of a second reached 0.5 s, where the floor and rounding would differ.
  - **W6, Hide on:** passed: 0 frames with the dot in Divora's 10 s take (312 frames), its take inside Replay (162) and the saved clip (481), against 140 of 140 in the Hide-off control's middle; the overlay window's display affinity read 0x11 (left out of captures) only during the protected take, while Replay was armed and during the take inside it, and 0x0 otherwise. The stand-in, ffmpeg's Desktop Duplication (`ddagrab`) of the light's rectangle at 60 fps, not OBS, proved itself (275 of 275 frames of the control's middle, 0 of 517 in the protected take's), then saw the dot in 0 frames within 1 s of every edge: the 10 s take's start (118 frames) and Stop (121), Replay arming (120), the inside take's start (98) and Stop (120), and Replay off (117). Window Capture and OBS itself weren't tried (Y3), and no take ended by itself.
  - **W7, Replay:** passed: armed "REPLAY", the take inside "REC 0:00" … "REC 0:04", its end "REPLAY", held nothing, started "REPLAY", off nothing; 30 events, each drawn within 0.1 ms, and the take's dot in 137 of 137 frames.
  - **W8, v1.61.0 on the same profile:** not run (the owner chose to skip it). That an older Divora keeps text charms and the light is from the code and the tests: scene.test's rows, and S2's offline self-test of W8's verdict against `c53b312`'s own reducer.
  - **W9, the cost:** passed on its third run. Runs 1 and 2 failed the idle 3D gate (+1.13 and +1.12 points against +0.5) on the harness: they measured the idle circle alone with Settings showing and the idle texts with the Mixer showing (the Replay-armed pair the other way round), whose level meters cost about 0.9 points on their own; a diagnosis run, each state measured on the same page, per process, read ten texts and the light at 0.749 % against the circle alone's 0.885 % with Settings showing, and 1.566 % against 1.826 % with the Mixer showing. Run 3, every state with Settings showing: idle +0.27 points and +4.6 MB of the renderer's private memory; the light drawn, Replay armed, −0.65 points and +7.0 MB; the gates +0.5 points and +20 MB. Run to run, the 3D figures vary by about ±0.3 points.
- **Not measured** (the owner checks Y1–Y7 in MANUAL_TESTS → Text charms (v1.62.0) haven't been run): OBS itself: the light in OBS's captures with Hide on, and the colours keyed with OBS's own presets (W6's stand-in is a Desktop Duplication capture, not OBS; the colours rest on a port of OBS's key distance). Windows 10 and the first Windows 11 release (with R3). A take that ends by itself (Divora's takes have no time limit, and sleep, unplugging a screen, a full disk and a failing encoder were out of bounds), so the light's self-ending edge waits for v1.64.0. Real keys, Alt in Arrange and an IME; a 150 % screen and mixed DPI; a game; a take over an hour; the Windows fonts on another PC; screen readers. In the app: a corner-resize snap, Ctrl changed mid-drag, an Edit in a window smaller than its scene's design, the editor's refusals, Vietnamese text (each rests on unit tests or e2e with Tauri mocked); whether the light's "m:ss" is the floor of `elapsedMs` (no W4 report's fraction of a second reached 0.5 s); an older Divora on a profile with text charms and a light (W8, which the owner chose to skip: from the code and the tests).
- **Follow-ups:**
  - **A Rust ack for K6:** hold Windows' request until the page confirms the light is erased, which would close the end edges where the backend stops asking in the same step that reports the change (Replay turning off, a take that ends by itself). W6's count at Replay off (0 frames with the dot in the 117 within 1 s of it, by a Desktop Duplication capture, not OBS) and Y3 say how much it matters.
  - **`fitTask`'s failure path:** if the place fails, it puts back the whole scene it read before its reframe, discarding every commit that landed while it waited: a picture, a timer face, the circle or (since v1.62.0) a recording light added meanwhile, a remove, an edit from the overlay. Storage keeps them only until the next save. Text saves wait on `placement`, so they're safe. The fix: undo only its own reframe (a reframe from `to` back to `from`, over the scene as it is then), with a controller test that a light or picture added during a failing Fit change is kept.
  - **The Alt finding from Y1** (K12): if Alt let go alone puts Windows into menu mode in Arrange, block it as Alt+Space is.
  - **v1.64.0's `hideApplies` and the self-ending edge:** once R15 makes `capturing` stop meaning the overlay is protected, the light's gate and the Arrange hint read the new key; and a window closing ends a take, so the light's self-ending edge is measured there.
  - **A very wide text in an overlay larger than its design** can still be born under 24 px tall, as a picture can since v1.55.0; a fix needs `add` to grow the design.
  - **The take and Replay state names:** `recLight.test` ties the light's state lists to `api.ts`, and the Rust test ties only "recording" and "armed" to the fixture; nothing ties `api.ts`'s other state names to Rust's.
- **Tests** at `c68237a`, on this PC (`CI=true cargo test --workspace --all-features`): divora-app's lib 482 passed and 1 ignored, divora-capture's 181 and 1, divora-core's 562 and 17, and the integration binaries; Vitest 2,528 tests in 64 files; Playwright 81, with Tauri mocked. Each track's tests were seen failing on the mutations in its table (the design's §10.5), but one that can't be told apart from the real code (guides drawn while Locked: locking already ends the drag); the audit found one more that no test caught (a drag going on after Lock) and one whose named test missed its defect, put back by a move and by a resize, though a table row caught each (a snap leaving the overlay), and each now has a test that fails on it; and step R's on theirs (93 over its five rounds, all but one caught, and that one given a test).

## Decisions

| Question | Choice | Why | Alternatives |
|---|---|---|---|
| Capture and encode stack | Hand-rolled WGC + Media Foundation on `windows = "0.61"` in `divora-capture`, with an encoder ladder; `windows-capture 2.0.1` as the day-5 kill switch | **Unsafe:** a few hundred lines we own and can fix, vs. a crate's unsafe we cannot. **Dependencies:** windows-rs is Microsoft-maintained and already compiled; windows-capture adds a fourth copy and has one maintainer with no changelog. **Timestamps:** we set them on both streams from one QPC base, whereas windows-capture clocks audio by sample count. **Replay:** windows-capture exposes no encoded samples | windows-capture as primary; WebView2 `getDisplayMedia` + MediaRecorder (no monitor pre-select, picker bugs WebView2Feedback #2184/#5173/#1850); ffmpeg sidecar (109 MB GPLv3 by default); openh264 (licence conditions, no AAC); AV1/VP9 (not real-time on CPU) |
| How Media Foundation is linked | Loaded at runtime, never imported | **Claim fixed.** A static import (windows 0.61's raw-dylib) would stop the app launching on Windows N editions. Precedent: `ort`'s `load-dynamic` | `/DELAYLOAD` + delayimp |
| How frames reach H.264 | Always NV12, converted explicitly | **Claim fixed.** Microsoft's sink-writer tutorial encodes RGB32 to *WMV*; nothing documents an RGB→NV12 converter ahead of H.264 | Rely on writer-inserted converters |
| Audio clock | Per-callback `StreamInstant` through a side ring; exact relative timing; one absolute anchor per pipeline; drift-only `ClockFit` | **Claim fixed.** `StreamInstant` is `Copy`. cpal's `playback − callback` is the free write window, not latency. Voice-convert overruns make the sample count a stepped clock | QPC read in the callback; drain-time `Instant` only |
| One mix or separate sources | Separate voice and soundboard rings, each corrected for its own latency | The chain delays the voice but not the soundboard | Tap the pre-mixed WAV signal |
| Desktop audio | Directly after the recorder; process loopback (exclude Divora) first, endpoint loopback as fallback | The owner confirmed game and Discord audio. Endpoint loopback doubles your voice in Divora's own recommended routing | Endpoint loopback only |
| Replay: ring contents | Encoded H.264 access units plus mixed PCM (not AAC) | Compressed video is ~1/80 the size of NV12. PCM lets a clip's audio start on the exact sample of its first keyframe | Raw frames (≈ 5 GB per 30 s); AAC frames (up to 21 ms start offset) |
| Replay: writing a clip | `IMFSinkWriter` passthrough for H.264 (CleanPoint on IDRs, samples interleaved by time) and writer-encoded AAC, from a snapshot on its own thread | Documented remux path; no new crate; same writer as takes | mp4-atom 0.16 (named fallback if passthrough fails the smoke test); mp4 0.14 (unmaintained); muxide (heavy dependencies) |
| Replay + take at once | One capture and one encoder, two consumers; the take uses the ring's settings | A take while armed costs a passthrough mux, not a second encode | Two pipelines (double GPU work, two encoder sessions) |
| Replay: GOP | 2 s, plus a proactive forced IDR at 2 × fps frames; refuse arming on B-frames or on encoders that ignore forced keyframes | Bounds the overshoot at about 2 s and makes the What's-new sentence true. 1 s would double the keyframes and soften takes too | 1 s or 0.5 s GOP |
| Replay: placement | Directly after desktop audio | The encoder refactor is cheapest while v1.52.0 is fresh; timers can then offer "save replay" | After the voice timers (charms 9–13 days sooner; re-measures takes later) |
| Replay: consent | Off at every launch; explainer checked on every arm path; titlebar chip, tray tooltip and the OS border while armed; nothing written until Save | It watches the screen for hours, in a privacy-first app | Arm on launch; a modal dialog |
| Hiding the overlay from recordings | v1.55.0, covers takes and Replay, applied only while Locked; the copy says "may" | **Claim fixed.** `WDA_EXCLUDEFROMCAPTURE` is reliable for non-layered windows only on Windows 11 22H2+ *(corrected by v1.62.0's design, C7: the plan also said "Locked makes the window layered", which Windows 10 was said to need. That doesn't hold: on Windows 10 19045 and Windows 11 22000 the click-through Locked adds is the suspected trigger of Electron #47834 (v1.55.0's As built, D8). Windows 10 is still R3's owner check)* | Ship it in v1.52.0 |
| Recorder, Replay and OBS commands | `async` | Tauri 2 runs sync commands on the main thread | Sync |
| Overlay topology | One overlay window + opt-in Fit to monitor; per-monitor windows in the tail | **Claim fixed:** Fit to monitor *does* ship a full-monitor window, so it is off by default and its GPU cost is measured | Per-monitor from day one |
| Editing under whole-window click-through | Arrange/Locked; Locked set from Rust before the first show; Arrange only after paint; six exits *(corrected by v1.62.0's design, C7: the plan said four, Esc, Settings, the tray and the hotkey; v1.55.0 added the chrome's Lock and the lease, which locks again after three silent 1 s checks)* | Tauri has no per-region click-through (#6164 open) | Cursor-polling thread toggling click-through |
| Where stream UI lives | Mixer-rail cards and Settings sections | The rail has 2.25 px left for a seventh entry | New nav screen |
| Timer clock | Rust thread, pure `TimerCore`, definitions pushed with `sync_timers` | The main window hides to the tray, and WebView2 throttles hidden-page timers | JS timers |
| Default OBS bridge | Overlay element + `.txt`; WebSocket opt-in and last | It must work with no password and no socket | Local HTTP server for a Browser Source |
| OBS client | Hand-written on `tokio-websockets =0.13.3`, about 600–800 lines *(as built in v1.61.0: about 4,500, not counting comments, blank lines and tests)* | obws is archived on GitHub with one owner, gates OBS ≥ 30.2, and has no request timeouts. The protocol has been frozen since 2022. +3 lockfile crates | obws 0.15.0 (about a day faster); tokio-tungstenite 0.30 (+13 crates) |
| OBS password | DPAPI blob in app data, FFI in `divora-capture`'s `native::dpapi`, crypt32.dll loaded at run time (v1.61.0) | No new crates; nothing left behind in Credential Manager after uninstall; testable on CI | keyring 4.2 (three new crates; entry outlives uninstall); localStorage (plaintext) |
| OBS scope | Timer text, BRB scene switch and back, uptime follow. **Changed from the earlier sketch:** no "stop recording at zero", no source show/hide | A misfired timer ending a live stream is the worst failure available. Show/hide is generic automation | StartStream/StopStream ops; a source show/hide op |
| OBS addressing and floor | UUIDs (names for display); obs-websocket 5.4.0 = OBS 30.1; IP literals only *(as built in v1.61.0: the floor is 5.4.0, and the copy names that, since the OBS 30.1 pairing isn't verified here: U9)* | UUIDs survive renames. IP literals mean no DNS query ever leaves the machine | Names back to OBS 28; hostnames |
| Replay and OBS bindings | Hotkeys, tray and timer actions now; MIDI and glyph actions in the bindings tail | The ask was "save on a hotkey"; one place for persisted union additions | Add MIDI/glyph kinds in each release |
| Real mute | Its own release, before timers | It touches the callback and makes an existing action honest | Inside the timers release |
| MSRV | **1.89** in v1.51.3 | **Claim fixed** (twice). `symphonia-adapter-libopus` 0.3.0 already requires 1.89, so the declared 1.80 is false today and the OBS client needs no bump | 1.88 (also false) |
| Naming | "Record screen", "Replay", "Charms", "Hourglass" | **Owner approved** | "Overlay elements", "Timers" |

## Not building, and why

- **Chat, chat-command timers, alerts, goals, subathon add-time, StreamElements/Streamlabs sync, any sign-in.** All of these need accounts, OAuth/EventSub or a cloud.
- **A local HTTP/SSE server for an OBS Browser Source, or OBS → Divora control.** Either would put a listening socket in a product that says "no server".
- **Starting or stopping OBS streams or recordings from Divora.**
- **ffmpeg sidecar, openh264, AV1/VP9, Opus-in-MP4, multi-track MP4, HDR tone-mapping.**
- **DXGI Desktop Duplication.** Revisit only if users ask and the border spike fails.
- **Replay on disk, over 120 s, armed at launch, or uploaded.**
- **Per-region click-through, or hover on a Locked overlay.** Tauri cannot forward mouse events to the window underneath.
- **A Stream Deck plugin, Pomodoro or a scheduler.** Hotkeys already reach everything, and Advanced Scene Switcher and Streamer.bot own generic automation.
- **A cpal 0.15 → 0.18 upgrade** bundled into any of these releases.
- **A pinned WebView2 runtime.**
- **Rotating a charm.** v1.55.0's "Not" named it as not yet, and the tail row dropped it without saying so; the owner chose not to build it (Q19, 2026-11-07).

## Risks and unknowns

- **COM that CI can only partly run.** The encoder and passthrough smoke tests run on CI. WGC, the GPU path, hardware encoders and AAC run only on this machine, so the spike gates and the manual checklist are the only real executions before a tag. Every COM call returns a `Result`, because `panic = "abort"` would otherwise kill the app.
- **Hardware encoders vary by vendor.** Selection, GOP and forced-keyframe compliance, and passthrough output types can each differ, and only one GPU vendor is tested here. Mitigations: the encoder actually loaded is checked, a clean media type is rebuilt for the muxer, and a keyframe watchdog refuses encoders that misbehave.
- **Hours-long timelines.** Replay runs `AudioTimeline` for hours. The CI 2-hour simulation and the manual 2-hour row are the checks; if `ClockFit` saturates, it re-anchors at a GOP boundary.
- **Game frame pacing** under continuous capture is unmeasured. It gets a PresentMon row, and the copy is updated if it shows.
- **Anchor accuracy.** The one-time absolute anchor reads `StreamInstant`'s value off the audio thread. Its `Debug` output is not a semver promise, hence the QPC cross-check, the fallback and the offset control.
- **Process loopback on Windows 10 22H2** rests on community reports; Microsoft documents build 20348+. The endpoint fallback covers the gap.
- **Click-through over WebView2's child window** has no primary-source guarantee. v1.55.0 names its kill switch. Evergreen regressions on transparent windows (tauri #14764) can appear with no app change.
- **Capture exclusion** never covers WebView2 popups (WebView2Feedback #4544), and on Windows 10 it is conditional.
- **OBS client.** We own the protocol code, and the fake server and the client share one author. Mitigations: an independently computed auth vector, and manual runs against five OBS versions. LAN traffic is plaintext (obs-websocket serves only `ws://`), and the copy says so. A DPAPI blob can stop decrypting after a profile reset. *(As built in v1.61.0: the auth vectors computed with Python and Node; the fake server written by a different hand from the client, from the protocol document and obs-websocket's sources; S2 against a third, a Node stand-in. No OBS version was run here, so the five are owner checks, U1–U15 and U9.)*
- **Exclusive-fullscreen games and protected video** *may* hide the overlay or record black.
- **Lip-sync bar.** ≤ 40 ms is a product choice that sits inside EBU R37's end-to-end limits (+40/−60 ms). R37's per-stage target (+5/−15 ms) is not claimed.
- **`std::time::Instant` across sleep** is unspecified. Persisted epochs and the resume signal cover it. *(As built in v1.59.0: each running timer's wall-clock anchor, checked when the resume signal comes; not yet tried across a real sleep (O3); its As built states the two cases it misses: a resume signal during an awake hold of over 500 ms on a clock that doesn't count sleep, and a second sleep of 5 s or less within a minute of one the clock counted.)*
- **Release link time** grows with the Media Foundation, D3D11 and tokio `net` additions under `lto` + `codegen-units = 1`. Measure it once and note it. *(v1.61.0's tokio `net`: T10, 84 s against `1c537fa`'s (v1.60.0's) 79 s on this PC (i7-14700K), same day (+5 s, +6 %); link.exe itself is 0.2 s of it. The whole release rebuild after a change to the app crate (both units, cargo's start-up left out): 154 s against 142 s (+12 s).)*

## Evidence

**Crates.**

| Crate | Version | Licence | Role |
|---|---|---|---|
| windows | 0.61.3 (pin "0.61") | MIT OR Apache-2.0 | WGC, D3D11, Media Foundation, process loopback, DPAPI; already compiled by tauri/tao/wry |
| windows-capture | 2.0.1 (kill switch only) | MIT | WGC + WinRT MediaTranscoder; windows 0.62.2; no encoded-sample access |
| cpal | 0.15.3 (unchanged) | Apache-2.0 | WASAPI streams; endpoint loopback fallback |
| rubato / ringbuf / hound | 0.16.2 / 0.4.8 / 3.5.1 (in lock) | MIT / MIT OR Apache-2.0 / Apache-2.0 | resampling with drift nudging, SPSC rings, WAV stems |
| sha2 / base64 | 0.10.9 (pin "0.10") / 0.22.1 (in lock) | MIT OR Apache-2.0 | asset ids; OBS auth string |
| tokio | 1.52.3 (in lock; gains `net`) | MIT | Tauri's own runtime, for the OBS link |
| tokio-websockets | 0.13.3 (pin "=0.13.3"; client, fastrand, sha1_smol; dev: server) | MIT | OBS WebSocket transport and the fake test server; MSRV 1.89 |
| sha1_smol / simdutf8 | 1.0.1 / 0.1.5 (new, transitive) | BSD-3-Clause / MIT OR Apache-2.0 | WebSocket accept-key check; UTF-8 validation |
| zeroize | 1.9.0 (in lock; new direct dependency) | Apache-2.0 OR MIT | wipes the decrypted OBS password |
| tauri / @tauri-apps/api | 2.11.2 / 2.11.0 | Apache-2.0 OR MIT | `set_content_protected`, `set_ignore_cursor_events`, `protocol-asset`, tray tooltip/menu |
| mp4-atom | 0.16.2 (fallback only, not added) | MIT OR Apache-2.0 | pure-Rust muxer if passthrough fails |
| wasapi (option) | 0.26.0 | MIT | process loopback; windows 0.62.2 |
| obws (rejected) | 0.15.0 | MIT | archived on GitHub; one owner; gates OBS ≥ 30.2 |
| obs-websocket (spoken to, not linked) | 5.4.2–5.7.6 (OBS 30.1–33.0; the OBS pairing not verified here, U9) | GPL-2.0-or-later | the server, over a TCP socket |

**Primary sources.**
- **Microsoft Learn: capture.** `IGraphicsCaptureItemInterop::CreateForMonitor` (1903), `IsCursorCaptureEnabled` (2004), `IsBorderRequired`/`GraphicsCaptureAccess` (20348; client Windows 11), `Direct3D11CaptureFrame.SystemRelativeTime` (QPC), `SetWindowDisplayAffinity`.
- **Microsoft Learn: Media Foundation.** The H.264 encoder (input formats, GOP default, ForceKeyFrame, sequence header), the AAC encoder, the MPEG-4 file sink (one audio stream, no edit lists, B-frame DTS), H.264 remux (clean points), Using the Sink Writer ("compressed input with identical output"), `SetInputMediaType`, `MFSampleExtension_CleanPoint`, the sink-writer attributes.
- **Microsoft Learn: audio, security and platform.** Loopback recording, `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`, `CryptProtectData`, the Media Feature Pack (KB4016817), DXGI flip model, the Windows 10 lifecycle.
- **Source read: Rust crates.**
  - cpal 0.15.3 `StreamInstant` and WASAPI host;
  - tao 0.35.3;
  - tauri 2.11.2 (window, tray, async runtime);
  - windows 0.61.3 / windows-link 0.1.3 (features, raw-dylib, `IMFSinkWriter`);
  - windows-capture 2.0.1 `encoder.rs`;
  - ort 2.0.0-rc.12 and symphonia-adapter-libopus 0.3.0 `Cargo.toml`;
  - global-hotkey 0.8.0;
  - tokio-websockets 0.13.3;
  - obws 0.15.0;
  - robmikh/displayrecorder.
- **Source read: OBS.** obs-websocket protocol.md and its server sources (close codes, event masks, outputs); obs-studio `obs-text.cpp`, plus `obs-ffmpeg-mux.c` as a design reference only (GPL-2.0; no code ported).
- **Issue trackers.**
  - tauri #8308, #6164, #11461, #14189, #14764, #11718, #6784;
  - WebView2Feedback #2184, #5173, #1850, #4176, #4544, #3070;
  - Electron #29085, #31340, #47834.
- **Other.** EBU R37; Raymond Chen on `NonRudeHWND` (2025-05-22); the Chrome timer-throttling post; YouTube's recommended upload settings.
- **Secondary, marked where used.** Consumer clock drift of ±50–100 ppm; NAudio/Chromium behaviour of idle loopback; borderless consent for unpackaged apps; process loopback on Windows 10 22H2.
