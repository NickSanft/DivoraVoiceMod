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

1. **Local-first, MIT, no account, no telemetry.** No GPL is linked into the app (espeak-ng's arm's-length subprocess is the precedent). Large optional components are only opt-in, explained downloads. Nothing touches the network without explicit consent.
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
  - **Modules:** `probe`, `monitors`, `wgc`, `encoder`, `mux`, `replay`, `loopback` and `ffi::dpapi`. The pure, CI-tested pieces are `FramePacer`, `ClockFit`, `AudioTimeline` and `ReplayRing`.
  - **`CapturePipeline`:** fans encoded packets out to `PacketSink` consumers. These are the take's muxer and the replay ring.
  - **Runtime-only Media Foundation:** `mfplat.dll`/`mfreadwrite.dll` are loaded with `LoadLibraryExW(…SYSTEM32)` + `GetProcAddress` for every MF free function used. COM methods are vtable calls and import nothing. A static import would stop `divora-app.exe` from launching at all on Windows N editions, where it launches today.
- **`src-tauri`** owns:
  - `AppState.screen_recorder`, which outlives engine sessions and holds the pipeline and the replay state;
  - `timers.rs`, `overlay_assets.rs` and `overlay_windows.rs`;
  - `obs/`: `proto.rs` and `machine.rs`, both pure, plus `link.rs`.

  Recorder, replay and OBS commands are `async`. Tauri 2 runs sync commands on the main thread, which every window shares.

**Threads.** The cpal callbacks keep their rules. Per buffer, the new work is two ring pushes, one side-ring push, one `fetch_add`, one atomic load and (v1.56.0) setting FTZ and DAZ in MXCSR, plus (v1.58.0) one more atomic load and push-to-modulate's dry line (a ring write and a delayed read every buffer; the voice itself is touched only while the effects are off or switching), all lock-free and allocation-free.
- **`divora-av-tap` (5 ms):** forwards chunks over a bounded channel.
- **`divora-encoder` (MTA COM):** owns the D3D11 device, frame pool, video processor, H.264 MFT and fan-out. It lives one per take in v1.52.0 and becomes one per pipeline in v1.54.0. WGC's FrameArrived only signals it.
- **`divora-replay-save`:** short-lived, one per save. It muxes a snapshot so capture never waits on disk.
- **`divora-loopback`:** owns the desktop-audio stream.
- **`divora-timers` (100 ms):** owns timer deadlines.
- **The OBS link:** two tasks on Tauri's existing tokio runtime, with no new OS thread and no second runtime. They exist only while the user has OBS control switched on.

```
 main (WebView2, hides to tray)                  overlay (WebView2, transparent, shadow:false,
 ┌───────────────────────────────┐              topmost; Locked by default, Arrange on request)
 │ store · Settings rows ·       │ overlay:state ┌───────────────────────────────┐
 │ Mixer cards · charm/timer defs│──────────────▶│ scene: spell circle, charms,  │
 │                               │◀──────────────│ timer face; emits on pointerup│
 └───────────────┬───────────────┘ ready / edit  └───────────────────────────────┘
                 │ invoke (recorder/replay/OBS commands async)  ▲ timer-state (broadcast)
 ════════════════╪════════════════ Tauri IPC ═══════════════════╪═════════════════════
 src-tauri                                                       │
 ├─ commands: *_screen_recording · *_replay_buffer · save_replay · open_overlay ·
 │            set_overlay_mode · import_overlay_asset · *_timer · set_input_muted · set_ptm_* · obs_*
 ├─ AppState.screen_recorder (outlives engine sessions; owns the CapturePipeline + replay state)
 ├─ thread divora-timers (100 ms; timers/<id>.txt on whole seconds) ──text/ops──┐
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
 ├─ ffi::dpapi (OBS password, v1.61.0)
 └─ FrameSink trait (kill switch: windows-capture 2.0.1 behind it, a code branch, never a feature)
```

## Releases

Version numbers are planning labels; a patch landing in between shifts the rest. Two minors landed after v1.56.0: v1.57.0 (Voice Convert keeps up, outside this plan) and v1.58.0 (push-to-modulate, from v1.56.0's follow-ups). So Hourglass, the voice timers and Talk to OBS, planned as v1.57.0, v1.58.0 and v1.59.0, are now v1.59.0, v1.60.0 and v1.61.0, and the tail v1.62.0 to v1.64.0.

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

**As built** (`[S3: ship date]`).
- **The switch** sits inside `mix_voice_and_soundboard`, after the chain and before loudness and the mute: `feed` → `modulator.apply` → the chain's reported latency, read once → `ptm.capture` → `chain.process` → `ptm.blend` → `loudness.process` → `run.mute` → the soundboard. Everything downstream gets the switched voice: the call, the headphone monitor, the WAV, screen takes, Replay and Voice reading's after-effects half. The soundboard, Speak and Critter Chatter never pass through it, and the mute still wins (D1, D6).
- **The chain keeps running while the effects are off** (D2), so a press has no gap and no stale tail, and an echo of what you said with them off is heard after a press (K10). With them off the call hears your voice with every effect off, the noise gate and the noise remover included; loudness, if on, works on it as on the effects (D7, owner Q4).
- **The dry voice is delayed by the chain's reported latency** through a per-session ring, the power of two above the rate (65,536 samples at 48 kHz, so up to 1.37 s; a longer latency is clamped), so the effects and your own voice carry the same moment of speech. The screen recorder's tap is given the latency the chain reported, which the line moves to, so takes place the voice right with the effects on, and with them off once the line has settled within its clamp (D3; from the code, not checked in a take). A latency change moves the line's read point: a shrink crossfades the two taps over L, skipping the difference; a growth fades out, stays silent for the difference, then fades in, so nothing is heard twice (D4; a 2,048-sample growth is silent for 2,049 samples, the fade-out's last zero-gain sample and the 2,048). A new session starts at delay 0, so the preset's delay arrives as a growth, almost always during start-up silence. With the effects off, your own voice keeps the preset's delay: 21 to 71 ms on the bundled presets that delay you at all, and a Voice Convert voice that isn't streaming adds about a quarter-second (K21).
- **The crossfade** (D5): linear, one position step per sample, L twice the mute's: 512 samples at 44.1 and 48 kHz (11.6 and 10.7 ms), at least 10 and under 20 ms at every rate from 12.8 to 409.6 kHz. A toggle mid-fade turns round. Settled on the effects the buffer isn't touched, so with push-to-modulate off the stage sends what it did before (`the_applied_stage_matches_the_chain_alone`, and the 48 stage fingerprints below).
- **Where a switch lands in what you said** (D8b, owner Q9): an edge toward the effects starts crossing in the first buffer that sees it; an edge away from them keeps the effects for the chain's latency first (taken at the edge; a flip back meanwhile cancels it), so the effects cover everything said while they were meant to be on. The word, the ticks and the pill change at the edge, so the pill leads the sound by that delay when the effects turn off. What remains is the input's own buffering less how late the key reached Divora: up to about 20 ms said just before letting go can reach the call in your own voice (K18).
- **The state** (D10): one engine-lifetime `AtomicU64`, `seq << 16 | mode << 8 | held`, a held bit per source (0 the global key, 1 the in-app key, 2 the Mixer's button, 3 MIDI; 4–7 reserved), so one source letting go never lets go of another. Modes `always` (push-to-modulate off, the launch default), `apply` and `bypass`. Hand-written compare-and-swap setters, from any thread; a set to the value it has changes nothing, `seq` included. The callback reads it once per buffer, the level emitter once per tick (`LevelUpdate.ptm`), and teardown never touches it.
- **Two sync commands** (D23): `set_ptm_mode {mode, restore?}` and `set_ptm_held {source, held}` (`"hotkey"` refused: only the backend holds that source), sync so a press can't overtake its release (`ptm_commands_stay_sync`). A press made while another sync command holds the main thread (device enumeration just after the window gains the focus, a Start opening a device) waits for it (K8). Without `restore` (the Mixer card) the mode set lets go of every hold, the global key's included, which is the way out of a stuck hold (D13). The page's boot sends `restore: true` with the saved mode: it lets go of the page's holds and keeps a global key still held (D14).
- **The global key** is acted on in Rust, press and release (`ptm_on_hotkey` in `global_shortcut_handler`; the release comes from global-hotkey's poller thread, which checks every 50 ms whether the key is still down); the page ignores the event (D15). Binding `ptm` to another key, to a key Divora can't read, clearing it or clearing every key lets go of its hold at the map update. Re-registering the same key while it is still registered only checks that it is (no unregister, no register), so a reload's re-sync keeps a held key held. `ptm:` is reserved. **The lock order** (D16; built in S1 rather than track B): the three shortcut commands call the plugin only after `AppState.shortcuts` is unlocked, which removes a deadlock found by reading the code (a key's release on the poller thread during a rebind), never seen.
- **The page's sources** (D17–D19). The in-app key (Space, or the global key's accelerator) holds on a keydown on the page with exactly the bound modifiers, ignoring autorepeat. It lets go on the trailing key's keyup anywhere, or on the keyup of the physical key that started the hold: that second rule came from the review, for a character that changes with Shift or AltGr (Shift+1 binds as "!", and letting go of Shift first makes the keyup say "1"). The Mixer's **Hold to switch** (was **Hold to test**) holds while the pointer is down on it, or while Enter or Space is down with it focused; it lets go on pointerup, pointerleave, pointercancel, its own blur and the card unmounting, and a mouse press hands the keyboard back to the page. The page lets go of both on window `blur`, on `visibilitychange` to hidden and on Tauri's `onFocusChanged(false)`; whichever comes second sends nothing. WebView2 keeps a window hidden to the tray `visible` (v1.54.0, and P3 again), so a hide lets go through the focus it takes away, not through `visibilitychange`. MIDI: the store keeps the set of `ptm` mappings that are down and sends only when it turns empty or non-empty; a CC holds at 64 or more and lets go at 59 or less (the mute's dead band); an input pick or MIDI-learn on any mapping forgets them all, editing or removing a mapping forgets that one, and the card's set forgets the pads but keeps each CC's side, so a control left up must come down and go back up.
- **The status** (D21, owner Q5): Modulated when an effect other than the noise gate is on and push-to-modulate isn't taking the effects off. `ui.pressed` and `ui.ptmMode` are gone: the pill reads the store's mirror of the word. With push-to-modulate off by default (owner Q2), a preset with effects reads Modulated, so the titlebar, the Mixer and the overlay's spell circle say Modulated and animate where they said Clean (K2); Clean Passthrough, the noise gate only, reads Clean.
- **Saved** (D14, owner Q3): `divora.ptm = {on, hold}`, written only from an engine answer, from the setting as it stands when the answer lands. Holds are never saved. `ptmKnown` (the boot restore's answer or failure, or 500 ms; a tick doesn't count) holds the overlay's pushes beside the mute's.
- **The Mixer card** (U2): a switch, a line under it, Hold to apply / Hold to bypass and Hold to switch, the last two disabled while it is off. With no global key the line says the key works only in Divora's window and where to set one; with only a learned `ptm` MIDI mapping it ends "…in Divora's window, or with your MIDI control." Settings → Hotkeys' row and the Control surfaces intro say what the key and a mapping do; the intro also says the card's set lets go.
- **Voice reading** (D9, U6): the microphone half is now the dry voice after the mute's `feed`, delayed by the chain's latency, pushed just before the after-effects half, so the two halves are compared at the same moment, and with the effects off the after-effects half is the same samples as the microphone half (scaled, if loudness is on), so it reads as a passthrough whatever the preset. A crossing the stage reports drops the after-effects window. The "passing the signal through" flag goes with the after-effects numbers it describes and changes only when they refresh, so it turns about 0.45 to 0.75 s after a switch (the worker test saw 0.53 to 0.72 s), and a held reading keeps its window's verdict. The cost (K3): effects that change nothing but still report latency (pitch or formant at exactly 0, the noise remover at mix 0) no longer read as a passthrough.
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
  - **U2, from the review:** with only a learned MIDI mapping the card's line names the MIDI control; it had said the key works in a call. A mapping that hasn't learned a control doesn't count.
  - **D9 / U6, from the review:** the "passing through" flag travels with the after-effects numbers. The design's version could pair the disguised numbers from just before a release with "passing the signal through", and hold that for 30 s in a pause. It now turns 0.45 to 0.75 s after a switch, not within 500 ms.
  - **Copy (§9 said unchanged):** Voice reading's after-effects line now reads "What the call hears: your voice after your effects, or as it is while Push to modulate has them off.", and its note "No measured difference from the microphone half — your effects are off, or they are passing the signal through." (it said the chain passes the signal through). Settings' MIDI sentence gained "Turning Push to modulate on or off on the Mixer, or picking its hold, lets go: …" (review).
  - **D14, from the review:** the saved setting is built when the answer lands, keeping a hold picked just before. An off sent while a hold pick was in flight had saved the old hold.
  - **K1:** the noise remover reports 480 samples and delays 480 at 480-frame buffers, 928 at 512 and 864 at 1056 (`the_denoisers_real_delay_is_pinned`; the design said 928–957). So with the noise remover on, a switch skips or repeats about 9 ms at 512-frame buffers and 8 ms at 1056. The other causes are as designed: Voice Convert's model about 5 ms past what it reports; Warble, which wavers by up to 8 ms; a pitch or formant shift set to exactly 0, 21 ms. Together, up to about 14 ms (Deep Narrator at 512-frame buffers, worked out from these figures). Not in the design: the harmonizer reports 0, and its harmony voices lag one vocoder window (21 ms) while its dry root doesn't, so at a switch the harmony, not your voice, is off by that much.
  - **The review** (step R): six confirmed findings, none refuted, all fixed before S2: the reading flag, the in-app key's release, the card's MIDI line, the reading's note and the saved setting above, and the Settings MIDI sentence.
- **Owner decisions** (the design's Q1–Q9, every default taken except Q8's tone): v1.57.0 shipped first (Q1); off by default (Q2); remembered (Q3); "effects off" is the whole chain, cut within the crossfade, with the chain running and your own voice keeping the preset's delay (Q4); a gate-only chain reads Clean (Q5); a latch later, `ptm:` reserved (Q6); the hotkey lock-order fix in this release (Q7); S2's eight `SendInput` holds yes, the test tone no (Q8); a switch lands where you spoke (Q9).
- **Done when** rests on N1 (a global key in a real Discord call: the preset while held, your voice after, no click, nothing doubled or lost mid-word), not run yet.
- **Not measured** (S2's gaps; the owner checks N1–N11 in MANUAL_TESTS → Push-to-modulate (v1.58.0) haven't been run yet): any sound in the app — the switch on the call, in the headphones, in a WAV, a take or Replay, its alignment and clicks (unit tests only; the fingerprints cover only the stage settled on the effects); the far end of a call (N1); the headphone monitor (N2); MIDI with a device (N3); a Stream Deck **Hotkey** action, expected to blip (N4); a key held with Shift (N5); an elevated window (N6); Voice reading in the app (N7); Voice Convert (N8); Discord's own push-to-talk on the same key (N9); the lock screen (N10); sleep and UAC; a hide to the tray with a key held; a preset switch with the effects off (N11); the card's line, Hold to switch and the in-app key's modifiers in the app (unit and e2e tests; P5 only clicked the card's switch); a relaunch; any rate but 48 kHz; the device signal after the resampler; Windows 10; B1 on other CPUs.
- **Follow-ups:**
  - **Report actual delays** (K1): the noise remover at other buffer sizes, Voice Convert's model, the harmonizer's pad. The switch, the latency readout and the take's placement would then all use the real figure.
  - **A latch** under `ptm:` (Q6), for a glyph, or a Stream Deck's stock **Hotkey** action, which is expected to send a tap (N4).
  - **Keep cleanup effects while the effects are off** (Q4): the noise gate and the noise remover; needs the chain split.
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

### v1.60.0 — Timers that change your voice (4–6 days)

**Goal.** The slice nothing else offers: a timer that switches preset for its duration and reverts, fires a tile, speaks a line, saves a replay, panics, or mutes for a break.

**Deliverables.**
1. **The `TimerAction` union.** It is the glyph actions plus:
   - `revertPreset` and `randomPreset`;
   - `speak {text, voiceId}`;
   - **`saveReplay`**, from v1.54.0;
   - an **idempotent `setMuted {muted}`**. The glyph mute toggles, which would *unmute* an already-muted voice on BRB.
2. **Hooks and dispatch:** timers gain `onStart`/`onFire`, all dispatched by one generalised `dispatchAction`.
3. **Where actions run.**
   - **Rust timer thread:** mute, unmute, panic and saveReplay. Rust-side panic emits `soundboard:stopped` so the store clears its playing clips.
   - **JS, on the `timer:fired` event:** everything else. Events reach a hidden page promptly; it is JS *timers* that get throttled.
4. **`speak` limits:** Kokoro and Critter Chatter voices only.
5. **"Break (BRB)" template.** It mutes on start, then unmutes and optionally speaks a line at zero.
6. **End now.** `finish_timer {id}`, plus the hotkey id `tm:<id>:finish`, runs a countdown's at-zero actions immediately. Ending a break early therefore unmutes, while Reset still cancels without firing.
7. **Defined behaviour** for a deleted preset and A/B compare open. Push-to-modulate keeps switching your effects under a timer's preset (since v1.58.0 it switches them in the engine), and the status says which the call hears.

**Done when.**
- A 5:00 "Villager" timer switches preset at start and reverts at zero.
- A BRB timer really silences the send, and End now brings the voice back early.

### v1.61.0 — Talk to OBS: timer text, break scenes, live uptime (opt-in; 6–9 days)

**Goal.** For people who stream with OBS, let the Hourglass and the Break template reach into OBS:
- a timer's time appears in an OBS text source as soon as it changes;
- a break switches OBS to a chosen scene and back;
- an uptime clock starts when OBS goes live.

It stays **off until switched on**, and nothing connects anywhere while it is off. The `.txt` bridge and the overlay timer are unchanged and stay the default.

**Why a small client of our own, not obws.** obws 0.15.0 is MIT and it works, but four things weigh against it.
- **One maintainer, and an archived repo:** a single crates.io owner, and the GitHub repo is archived (last push 2026-05-03) with a pointer to a self-hosted forge.
- **Size:** 9,673 lines to reach the 12 requests Divora sends.
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
     - **What it sends:** the timer thread pushes the same string as the `.txt` bridge, only when it changes, as `SetInputSettings {inputUuid, inputSettings:{text}, overlay:true}`.
     - **Which sources:** text sources are found by `unversionedInputKind` ∈ {`text_gdiplus`, `text_ft2_source`} from an unfiltered GetInputList, re-fetched every time a picker opens.
     - **What it leaves alone:** a source set to read from a file is labelled *"reads a file — it's using the .txt bridge"* and never switched over.
     - **Reconnect:** the current text is pushed at once.
   - **Scenes.** `ObsOp` = `{op:"scene", sceneUuid, sceneName}` | `{op:"sceneBack"}`.
     - **`sceneBack`** returns to the replaced scene **only if the program is still the scene Divora set**. If you moved on during the break, Divora leaves OBS alone.
     - **Not persisted:** the return slot does not survive a Divora restart.
     - **Timers:** `TimerAction` gains `{kind:"obs", op}`, run on the Rust timer thread.
   - **Uptime.** A count-up timer gains an optional `followObs: "stream"|"record"`. It is driven by `outputState` (STARTED, STOPPED, PAUSED, RESUMED); RECONNECTING is ignored. When Divora connects to an OBS that is already live, the clock is seeded from `outputDuration`, which counts frames, so the copy says "about".
7. **The Break template gains** *"Switch OBS to [scene] for the break"* (shown only when OBS is on). onStart runs `scene`, and onFire/End now run `sceneBack`.
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
   - **Commands, async:** `obs_configure {config}`, `obs_set_password {password}`, `obs_forget_password`, `obs_status`, `obs_inventory`, `obs_run {op}`.
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

### Tail releases (optional; each its own minor; the plan can stop before any of them)

| Release | Scope | Gate |
|---|---|---|
| v1.62.0 Charms everywhere | One overlay window per monitor (`overlay*` capability glob, monitor stored by name, physical-pixel placement); text and video charms; snapping; record a single window (`CreateForWindow`); a small "recording" charm | Mixed-DPI and hot-plug history (tauri #11718, #6784); one WebView2 per monitor |
| v1.63.0 Remove the yellow border (Windows 11) | Spike: test `IsBorderRequired(false)` with *and* without `RequestAccessAsync(Borderless)`, and ship only if the border **visibly disappears** for the unpackaged app. Applies to takes and Replay. Client availability is Windows 11 22000+, gated by `IsPropertyPresent` | A drop outcome is acceptable |
| v1.64.0 More timers and bindings | Laps, interval ("voice roulette"), wall-clock targets with clock-jump detection, the titlebar timer pill; MIDI and glyph actions for timers, Replay save (`replay`) and OBS scenes (`obs`), with router edge detection so a fader cannot fire a save repeatedly | — |

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
| Hiding the overlay from recordings | v1.55.0, covers takes and Replay, applied only while Locked; the copy says "may" | **Claim fixed.** `WDA_EXCLUDEFROMCAPTURE` is reliable for non-layered windows only on Windows 11 22H2+; Locked makes the window layered | Ship it in v1.52.0 |
| Recorder, Replay and OBS commands | `async` | Tauri 2 runs sync commands on the main thread | Sync |
| Overlay topology | One overlay window + opt-in Fit to monitor; per-monitor windows in the tail | **Claim fixed:** Fit to monitor *does* ship a full-monitor window, so it is off by default and its GPU cost is measured | Per-monitor from day one |
| Editing under whole-window click-through | Arrange/Locked; Locked set from Rust before the first show; Arrange only after paint; four exits | Tauri has no per-region click-through (#6164 open) | Cursor-polling thread toggling click-through |
| Where stream UI lives | Mixer-rail cards and Settings sections | The rail has 2.25 px left for a seventh entry | New nav screen |
| Timer clock | Rust thread, pure `TimerCore`, definitions pushed with `sync_timers` | The main window hides to the tray, and WebView2 throttles hidden-page timers | JS timers |
| Default OBS bridge | Overlay element + `.txt`; WebSocket opt-in and last | It must work with no password and no socket | Local HTTP server for a Browser Source |
| OBS client | Hand-written on `tokio-websockets =0.13.3`, about 600–800 lines | obws is archived on GitHub with one owner, gates OBS ≥ 30.2, and has no request timeouts. The protocol has been frozen since 2022. +3 lockfile crates | obws 0.15.0 (about a day faster); tokio-tungstenite 0.30 (+13 crates) |
| OBS password | DPAPI blob in app data, FFI in `divora-capture` | No new crates; nothing left behind in Credential Manager after uninstall; testable on CI | keyring 4.2 (three new crates; entry outlives uninstall); localStorage (plaintext) |
| OBS scope | Timer text, BRB scene switch and back, uptime follow. **Changed from the earlier sketch:** no "stop recording at zero", no source show/hide | A misfired timer ending a live stream is the worst failure available. Show/hide is generic automation | StartStream/StopStream ops; a source show/hide op |
| OBS addressing and floor | UUIDs (names for display); obs-websocket 5.4.0 = OBS 30.1; IP literals only | UUIDs survive renames. IP literals mean no DNS query ever leaves the machine | Names back to OBS 28; hostnames |
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

## Risks and unknowns

- **COM that CI can only partly run.** The encoder and passthrough smoke tests run on CI. WGC, the GPU path, hardware encoders and AAC run only on this machine, so the spike gates and the manual checklist are the only real executions before a tag. Every COM call returns a `Result`, because `panic = "abort"` would otherwise kill the app.
- **Hardware encoders vary by vendor.** Selection, GOP and forced-keyframe compliance, and passthrough output types can each differ, and only one GPU vendor is tested here. Mitigations: the encoder actually loaded is checked, a clean media type is rebuilt for the muxer, and a keyframe watchdog refuses encoders that misbehave.
- **Hours-long timelines.** Replay runs `AudioTimeline` for hours. The CI 2-hour simulation and the manual 2-hour row are the checks; if `ClockFit` saturates, it re-anchors at a GOP boundary.
- **Game frame pacing** under continuous capture is unmeasured. It gets a PresentMon row, and the copy is updated if it shows.
- **Anchor accuracy.** The one-time absolute anchor reads `StreamInstant`'s value off the audio thread. Its `Debug` output is not a semver promise, hence the QPC cross-check, the fallback and the offset control.
- **Process loopback on Windows 10 22H2** rests on community reports; Microsoft documents build 20348+. The endpoint fallback covers the gap.
- **Click-through over WebView2's child window** has no primary-source guarantee. v1.55.0 names its kill switch. Evergreen regressions on transparent windows (tauri #14764) can appear with no app change.
- **Capture exclusion** never covers WebView2 popups (WebView2Feedback #4544), and on Windows 10 it is conditional.
- **OBS client.** We own the protocol code, and the fake server and the client share one author. Mitigations: an independently computed auth vector, and manual runs against five OBS versions. LAN traffic is plaintext (obs-websocket serves only `ws://`), and the copy says so. A DPAPI blob can stop decrypting after a profile reset.
- **Exclusive-fullscreen games and protected video** *may* hide the overlay or record black.
- **Lip-sync bar.** ≤ 40 ms is a product choice that sits inside EBU R37's end-to-end limits (+40/−60 ms). R37's per-stage target (+5/−15 ms) is not claimed.
- **`std::time::Instant` across sleep** is unspecified. Persisted epochs and the resume signal cover it.
- **Release link time** grows with the Media Foundation, D3D11 and tokio `net` additions under `lto` + `codegen-units = 1`. Measure it once and note it.

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
| obs-websocket (spoken to, not linked) | 5.4.2–5.7.6 (OBS 30.1–33.0) | GPL-2.0-or-later | the server, over a TCP socket |

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
