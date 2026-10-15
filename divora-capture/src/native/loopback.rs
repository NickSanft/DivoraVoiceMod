//! Desktop audio from Windows (v1.53.0): what the PC plays, captured as it
//! plays it.
//!
//! Two ways in, one loop ([`crate::desktop_thread`]):
//!
//! - **Everything except Divora** — *process loopback*: the audio of every
//!   process except this one and the processes it started, whatever device
//!   each plays on. Divora's own voice, soundboard and monitor are never in
//!   it. Tried, not assumed, before it is offered.
//! - **One playback device** — *endpoint loopback*: everything that device
//!   plays, Divora included. Offered where process loopback can't be had.
//!
//! Both ask Windows for the same thing — 48 kHz float stereo, converted from
//! whatever the device runs at.
//!
//! What was measured on Windows 11 (26200) and is relied on here: process
//! loopback sends a packet every 10 ms even when nothing plays, and its
//! timestamps follow the performance counter exactly; endpoint loopback
//! sends nothing while its device is idle; neither may be `Reset()` (a reset
//! process-loopback client never delivers again); and a process-loopback
//! client answers `GetMixFormat`, `GetCurrentPadding` and friends with
//! `E_NOTIMPL`. Activating one needs its parameters in a `PROPVARIANT` that
//! must never be dropped as one (see [`handler::ActivationBlob`]).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{sync_channel, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::core::{s, w, IUnknown, Interface, Owned, HRESULT, HSTRING, PCWSTR, PWSTR};
use windows::Foundation::Metadata::ApiInformation;
use windows::Win32::Foundation::{
    ERROR_TIMEOUT, E_NOINTERFACE, HANDLE, HMODULE, PROPERTYKEY, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IAudioCaptureClient, IAudioClient, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    DEVICE_STATE_ACTIVE, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, BLOB, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentThread, SetThreadPriority, WaitForSingleObject, THREAD_PRIORITY,
    THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_NORMAL,
};
use windows::Win32::System::Variant::{VT_BLOB, VT_LPWSTR};

use super::record::ComScope;
use crate::desktop::{
    qpc_to_100ns, DesktopSource, DesktopTap, OpenedSource, PlaybackDevice, QpcAnchor,
    DESKTOP_CHANNELS, DESKTOP_RATE,
};
use crate::desktop_thread::{self, Clock, LossFlags, OpenError, Opener, PacketSource, RawPacket};
use crate::error::{CaptureError, Context};

/// Set to anything to make this process behave as if process loopback were
/// unavailable — how the playback-device path is checked on a machine that
/// would never need it (`docs/MANUAL_TESTS.md`).
pub const PRETEND_NO_PROCESS_LOOPBACK_ENV: &str = "DIVORA_PRETEND_NO_PROCESS_LOOPBACK";

/// How long process-loopback activation may take before it is given up on.
/// It takes a couple of milliseconds when it works; shorter than
/// `DesktopCapture`'s own wait, so the answer always reaches it.
const ACTIVATE_TIMEOUT: Duration = Duration::from_millis(1_500);

/// What a playback device is asked to buffer: 100 ms of room, so a reader
/// held up by the system loses nothing, yet well inside the timeline's
/// patience for desktop audio (`DESKTOP_WAIT_SECONDS`). Process loopback
/// keeps about 30 ms whatever it is asked for.
const DEVICE_BUFFER_100NS: i64 = 1_000_000;

/// `WAVE_FORMAT_IEEE_FLOAT`. windows 0.61 only declares it, as a `u32`,
/// under a feature this crate does not otherwise need.
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

/// `PKEY_Device_FriendlyName` — the name Windows' Sound settings show, and
/// the one the audio engine picks devices by.
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xa45c_254e_df1c_4efd_8020_67d1_46a8_50e0),
    pid: 14,
};

// ---- Availability --------------------------------------------------------

/// Set once process loopback has been seen to work. A failure is not
/// remembered: it may have been passing (the audio service restarting).
static PROCESS_LOOPBACK_WORKS: OnceLock<()> = OnceLock::new();

/// Whether "everything except Divora" can be recorded here: Windows 10 2004
/// or later (`UniversalApiContract` 10), and an activation that actually
/// succeeds.
pub fn process_loopback_available() -> bool {
    if std::env::var_os(PRETEND_NO_PROCESS_LOOPBACK_ENV).is_some() {
        return false;
    }
    if PROCESS_LOOPBACK_WORKS.get().is_some() {
        return true;
    }
    let contract = ApiInformation::IsApiContractPresentByMajor(
        &HSTRING::from("Windows.Foundation.UniversalApiContract"),
        10,
    )
    .unwrap_or(false);
    if !contract {
        return false;
    }
    let _com = ComScope::enter();
    let works = activate_process_loopback().and_then(|client| initialize(&client, false));
    match works {
        Ok(()) => {
            let _ = PROCESS_LOOPBACK_WORKS.set(());
            true
        }
        Err(e) => {
            tracing::info!(error = %e, "process loopback unavailable");
            false
        }
    }
}

/// The playback devices that are plugged in and enabled, the Windows default
/// marked.
pub fn list_playback_devices() -> Result<Vec<PlaybackDevice>, CaptureError> {
    let _com = ComScope::enter();
    let en = enumerator()?;
    // SAFETY: a live enumerator; there may legitimately be no default.
    let default_id = unsafe { en.GetDefaultAudioEndpoint(eRender, eConsole) }
        .ok()
        .and_then(|d| device_id(&d).ok());
    // SAFETY: a live enumerator.
    let all = unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        .context("EnumAudioEndpoints")?;
    // SAFETY: a live collection.
    let count = unsafe { all.GetCount() }.context("IMMDeviceCollection::GetCount")?;
    let mut out = Vec::new();
    for i in 0..count {
        // SAFETY: `i` is below the collection's count.
        let Ok(dev) = (unsafe { all.Item(i) }) else {
            continue;
        };
        let Ok(id) = device_id(&dev) else {
            continue;
        };
        let name = friendly_name(&dev).unwrap_or_else(|| "Playback device".to_string());
        out.push(PlaybackDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name,
        });
    }
    Ok(out)
}

// ---- The capture thread --------------------------------------------------

/// The body of `divora-loopback` (see [`desktop_thread::run`]).
pub fn capture_thread(
    source: &DesktopSource,
    mut tap: DesktopTap,
    stop: &AtomicBool,
    (lost, gave_up): (&AtomicBool, &AtomicBool),
    ready: &Sender<Result<OpenedSource, CaptureError>>,
) {
    let _com = ComScope::enter();
    let flags = LossFlags { lost, gave_up };
    desktop_thread::run(
        &mut ComOpener,
        &RealClock,
        source,
        &mut tap,
        stop,
        &flags,
        ready,
    );
}

struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, d: Duration, stop: &AtomicBool) -> bool {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if stop.load(std::sync::atomic::Ordering::Acquire) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        stop.load(std::sync::atomic::Ordering::Acquire)
    }
}

fn set_priority(p: THREAD_PRIORITY) {
    // SAFETY: the pseudo-handle of the calling thread.
    if let Err(e) = unsafe { SetThreadPriority(GetCurrentThread(), p) } {
        tracing::debug!(error = %e, "desktop audio thread priority unchanged");
    }
}

/// `AvSetMmThreadCharacteristicsW` and `AvRevertMmThreadCharacteristics`
/// (avrt.dll), exactly as windows 0.61.3 declares them
/// (Win32/System/Threading/mod.rs) — resolved at run time, so the exe
/// gains no load-time import of avrt.dll.
type AvSetFn = unsafe extern "system" fn(PCWSTR, *mut u32) -> HANDLE;
type AvRevertFn = unsafe extern "system" fn(HANDLE) -> windows::core::BOOL;

fn avrt() -> Option<(AvSetFn, AvRevertFn)> {
    static AVRT: OnceLock<Option<(AvSetFn, AvRevertFn)>> = OnceLock::new();
    *AVRT.get_or_init(|| {
        // SAFETY: a static name; System32 only; never freed.
        let module: HMODULE =
            unsafe { LoadLibraryExW(w!("avrt.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
        // SAFETY: a loaded module and static C strings.
        let set = unsafe { GetProcAddress(module, s!("AvSetMmThreadCharacteristicsW")) }?;
        // SAFETY: as above.
        let revert = unsafe { GetProcAddress(module, s!("AvRevertMmThreadCharacteristics")) }?;
        // SAFETY: the exports' real signatures; FARPROC and these function
        // pointers have the same size and ABI.
        unsafe {
            Some((
                std::mem::transmute_copy::<_, AvSetFn>(&set),
                std::mem::transmute_copy::<_, AvRevertFn>(&revert),
            ))
        }
    })
}

/// Put the calling thread under the multimedia scheduler's "Audio" task —
/// how Windows keeps audio threads on time under load — or, where that
/// can't be had, at the highest normal priority. A process-loopback reader
/// that falls 30 ms behind loses audio without being told (Windows queues
/// about that much). The work per packet is microseconds, so the voice
/// engine's threads lose nothing to it.
///
/// Not every hole is the reader's: process loopback itself skips about
/// 15 ms and then sends 30 ms of zeros whenever another app opens or closes
/// a stream on the same device (measured with a tone playing throughout; the
/// device's own loopback had no hole). Those arrive as a jump in the packets' times and
/// are counted as gaps, the audio after them on time.
fn make_urgent() -> Option<HANDLE> {
    let registered = avrt().and_then(|(set, _)| {
        let mut task = 0u32;
        // SAFETY: a static task name and a valid out-pointer.
        let h = unsafe { set(w!("Audio"), &raw mut task) };
        (!h.is_invalid() && !h.0.is_null()).then_some(h)
    });
    if registered.is_none() {
        set_priority(THREAD_PRIORITY_HIGHEST);
    }
    registered
}

/// Undo [`make_urgent`].
fn make_ordinary(mmcss: Option<HANDLE>) {
    if let (Some(h), Some((_, revert))) = (mmcss, avrt()) {
        // SAFETY: the handle AvSetMmThreadCharacteristicsW gave this thread.
        let _ = unsafe { revert(h) };
    }
    set_priority(THREAD_PRIORITY_NORMAL);
}

/// Opens Windows capture clients on the capture thread.
struct ComOpener;

impl Opener for ComOpener {
    type Stream = ComStream;

    fn open(&mut self, source: &DesktopSource) -> Result<(ComStream, OpenedSource), OpenError> {
        // Opening, retrying and the waits between tries happen at normal
        // priority; it is raised only while a stream is open (dropping one
        // lowers it again). Only reading packets is urgent.
        set_priority(THREAD_PRIORITY_NORMAL);
        let (mut stream, opened) = open_exactly(source)?;
        stream.mmcss = make_urgent();
        Ok((stream, opened))
    }
}

fn open_exactly(source: &DesktopSource) -> Result<(ComStream, OpenedSource), OpenError> {
    match source {
        DesktopSource::Everything => {
            if std::env::var_os(PRETEND_NO_PROCESS_LOOPBACK_ENV).is_some() {
                return Err(OpenError::Failed(CaptureError::Unavailable(format!(
                    "{PRETEND_NO_PROCESS_LOOPBACK_ENV} is set"
                ))));
            }
            let client = match activate_process_loopback() {
                Ok(c) => c,
                Err(e @ CaptureError::Windows { .. }) if is_timeout(&e) => {
                    return Err(OpenError::TimedOut(e));
                }
                Err(e) => return Err(OpenError::Failed(e)),
            };
            let stream = ComStream::start(client, false).map_err(OpenError::Failed)?;
            Ok((stream, OpenedSource::Everything))
        }
        DesktopSource::Device(id) => {
            let opened = (|| {
                let en = enumerator()?;
                // SAFETY: a live enumerator; the id is passed as a wide string.
                let dev = unsafe { en.GetDevice(&HSTRING::from(id.as_str())) }
                    .context("IMMDeviceEnumerator::GetDevice")?;
                let name = friendly_name(&dev).unwrap_or_else(|| "Playback device".to_string());
                // SAFETY: a live device; no activation parameters.
                let client: IAudioClient =
                    unsafe { dev.Activate(CLSCTX_ALL, None) }.context("IMMDevice::Activate")?;
                let stream = ComStream::start(client, true)?;
                Ok::<_, CaptureError>((
                    stream,
                    OpenedSource::Device {
                        id: id.clone(),
                        name,
                    },
                ))
            })();
            opened.map_err(OpenError::Failed)
        }
    }
}

fn is_timeout(e: &CaptureError) -> bool {
    matches!(e, CaptureError::Windows { source, .. } if source.code() == ERROR_TIMEOUT.to_hresult())
}

/// An open, started capture client. It stays on the capture thread: COM
/// objects in windows 0.61 are not `Send`, and need not be.
struct ComStream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: Owned<HANDLE>,
    anchor: QpcAnchor,
    device: bool,
    /// This thread's multimedia-scheduler registration while the stream is
    /// open (see [`make_urgent`]).
    mmcss: Option<HANDLE>,
}

impl ComStream {
    fn start(client: IAudioClient, device: bool) -> Result<Self, CaptureError> {
        initialize(&client, device)?;
        // SAFETY: an auto-reset, unnamed, initially unsignalled event.
        let event = unsafe {
            Owned::new(CreateEventW(None, false, false, PCWSTR::null()).context("CreateEventW")?)
        };
        // SAFETY: an initialized event-driven client and a live event; it
        // must be set before Start.
        unsafe { client.SetEventHandle(*event) }.context("SetEventHandle")?;
        // SAFETY: an initialized client.
        let capture: IAudioCaptureClient =
            unsafe { client.GetService() }.context("GetService(IAudioCaptureClient)")?;
        let anchor = read_anchor()?;
        // SAFETY: an initialized client with its event set.
        unsafe { client.Start() }.context("IAudioClient::Start")?;
        Ok(Self {
            client,
            capture,
            event,
            anchor,
            device,
            mmcss: None,
        })
    }
}

impl PacketSource for ComStream {
    fn wait(&self, ms: u32) -> Result<bool, CaptureError> {
        // SAFETY: a live event handle.
        let woke = unsafe { WaitForSingleObject(*self.event, ms) };
        if woke == WAIT_FAILED {
            return Err(CaptureError::Windows {
                context: "WaitForSingleObject",
                source: windows::core::Error::from_win32(),
            });
        }
        Ok(woke == WAIT_OBJECT_0)
    }

    fn next(&self) -> Result<Option<RawPacket<'_>>, CaptureError> {
        // SAFETY: a started client.
        let size = unsafe { self.capture.GetNextPacketSize() }.context("GetNextPacketSize")?;
        if size == 0 {
            return Ok(None);
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        let mut qpc = 0u64;
        // SAFETY: valid out-pointers; the device position is not wanted.
        unsafe {
            self.capture.GetBuffer(
                &raw mut data,
                &raw mut frames,
                &raw mut flags,
                None,
                Some(&raw mut qpc),
            )
        }
        .context("GetBuffer")?;
        if frames == 0 {
            // AUDCLNT_S_BUFFER_EMPTY: nothing handed out, nothing to release.
            return Ok(None);
        }
        let flag = |f: i32| flags & f.cast_unsigned() != 0;
        let silent = flag(AUDCLNT_BUFFERFLAGS_SILENT.0);
        let readable =
            !silent && !data.is_null() && data.align_offset(std::mem::align_of::<f32>()) == 0;
        let samples = readable.then(|| {
            // SAFETY: GetBuffer hands out `frames` frames of the format the
            // client was initialized with — two f32s each — valid until
            // ReleaseBuffer; non-null and aligned, checked above. The slice
            // borrows `self`, so it cannot outlive the stream.
            #[allow(clippy::cast_ptr_alignment)] // checked just above
            unsafe {
                std::slice::from_raw_parts(data.cast::<f32>(), frames as usize * DESKTOP_CHANNELS)
            }
        });
        Ok(Some(RawPacket {
            samples,
            frames,
            qpc_100ns: qpc,
            silent,
            discontinuity: flag(AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0),
            timestamp_error: flag(AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0),
        }))
    }

    fn release(&self, frames: u32) -> Result<(), CaptureError> {
        // SAFETY: releases exactly the packet GetBuffer handed out.
        unsafe { self.capture.ReleaseBuffer(frames) }.context("ReleaseBuffer")
    }

    fn alive(&self) -> Result<(), CaptureError> {
        // A process client has no padding to ask about (E_NOTIMPL); the
        // loop only asks playback devices.
        // SAFETY: a started client.
        unsafe { self.client.GetCurrentPadding() }
            .map(|_| ())
            .context("GetCurrentPadding")
    }

    fn anchor(&self) -> QpcAnchor {
        self.anchor
    }

    fn is_device(&self) -> bool {
        self.device
    }
}

impl Drop for ComStream {
    fn drop(&mut self) {
        // Stop only. Never Reset: a reset process-loopback client never
        // delivers again (measured; GStreamer says the same).
        // SAFETY: a live client.
        let _ = unsafe { self.client.Stop() };
        // Whatever comes next — a wait before trying again, or the end —
        // is not urgent.
        make_ordinary(self.mmcss.take());
    }
}

/// Ask for 48 kHz float stereo, event-driven, converted by Windows from
/// whatever the source runs at.
fn initialize(client: &IAudioClient, device: bool) -> Result<(), CaptureError> {
    let channels = u16::try_from(DESKTOP_CHANNELS).unwrap_or(2);
    let block_align = channels * 4;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: channels,
        nSamplesPerSec: DESKTOP_RATE,
        nAvgBytesPerSec: DESKTOP_RATE * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    let buffer = if device { DEVICE_BUFFER_100NS } else { 0 };
    // SAFETY: a fresh client; `format` lives across the call.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            flags,
            buffer,
            0,
            &raw const format,
            None,
        )
    }
    .context("IAudioClient::Initialize")
}

/// The performance counter, in the 100-ns units `GetBuffer` stamps packets in.
fn qpc_now() -> Result<u64, CaptureError> {
    let mut t = 0i64;
    let mut f = 0i64;
    // SAFETY: valid out-pointers.
    unsafe {
        QueryPerformanceCounter(&raw mut t).context("QueryPerformanceCounter")?;
        QueryPerformanceFrequency(&raw mut f).context("QueryPerformanceFrequency")?;
    }
    Ok(qpc_to_100ns(t, f))
}

/// The (`Instant`, counter) pair a stream's timestamps are read by: the
/// counter read between two `Instant`s, taken only when they are close —
/// a thread preempted between the reads would shift a whole stream.
fn read_anchor() -> Result<QpcAnchor, CaptureError> {
    let mut best: Option<(Duration, QpcAnchor)> = None;
    for _ in 0..8 {
        let before = Instant::now();
        let qpc = qpc_now()?;
        let after = Instant::now();
        let width = after.saturating_duration_since(before);
        let anchor = QpcAnchor {
            at: before + width / 2,
            qpc_100ns: qpc,
        };
        if width <= Duration::from_micros(20) {
            return Ok(anchor);
        }
        if best.is_none_or(|(w, _)| width < w) {
            best = Some((width, anchor));
        }
    }
    let (width, anchor) = best.ok_or_else(|| {
        CaptureError::Unavailable("couldn't read the performance counter".to_string())
    })?;
    tracing::debug!(?width, "desktop audio anchored loosely");
    Ok(anchor)
}

fn enumerator() -> Result<IMMDeviceEnumerator, CaptureError> {
    // SAFETY: COM is initialized on this thread (ComScope).
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .context("CoCreateInstance(MMDeviceEnumerator)")
}

/// A device's endpoint id: stable for the device, whatever it is renamed.
fn device_id(dev: &IMMDevice) -> Result<String, CaptureError> {
    // SAFETY: a live device; the string is CoTaskMemAlloc'd for us.
    let p: PWSTR = unsafe { dev.GetId() }.context("IMMDevice::GetId")?;
    // SAFETY: a NUL-terminated wide string from GetId.
    let id = unsafe { p.to_string() }.unwrap_or_default();
    // SAFETY: allocated by GetId, freed once.
    unsafe { CoTaskMemFree(Some(p.0.cast_const().cast::<c_void>())) };
    Ok(id)
}

/// The name Windows shows for a device, when it has one.
fn friendly_name(dev: &IMMDevice) -> Option<String> {
    // SAFETY: a live device.
    let store = unsafe { dev.OpenPropertyStore(STGM_READ) }.ok()?;
    let key = PKEY_DEVICE_FRIENDLY_NAME;
    // SAFETY: a live store and a valid key. The PROPVARIANT clears itself
    // (PropVariantClear) when dropped — this one is Windows' own.
    let value = unsafe { store.GetValue(&raw const key) }.ok()?;
    // SAFETY: the union is read as the member its `vt` tag names.
    unsafe {
        let inner = &value.Anonymous.Anonymous;
        if inner.vt != VT_LPWSTR || inner.Anonymous.pwszVal.is_null() {
            return None;
        }
        inner.Anonymous.pwszVal.to_string().ok()
    }
}

// ---- Process-loopback activation -----------------------------------------

/// `ActivateAudioInterfaceAsync`, exactly as windows 0.61.3 declares it
/// (Win32/Media/Audio/mod.rs) — resolved at run time, like Media
/// Foundation, so the exe gains no load-time import of mmdevapi.dll.
type ActivateFn = unsafe extern "system" fn(
    PCWSTR,
    *const windows::core::GUID,
    *const PROPVARIANT,
    *mut c_void,
    *mut *mut c_void,
) -> HRESULT;

fn activate_fn() -> Option<ActivateFn> {
    static ACTIVATE: OnceLock<Option<ActivateFn>> = OnceLock::new();
    *ACTIVATE.get_or_init(|| {
        // SAFETY: a static name; the search is restricted to System32. The
        // module is never freed: a completion that arrives after our timeout
        // still runs code in it.
        let module: HMODULE =
            unsafe { LoadLibraryExW(w!("mmdevapi.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .ok()?;
        // SAFETY: a loaded module and a static C string.
        let f = unsafe { GetProcAddress(module, s!("ActivateAudioInterfaceAsync")) }?;
        // SAFETY: `ActivateFn` is the export's real signature; a FARPROC and
        // an `extern "system" fn` pointer have the same size and ABI.
        Some(unsafe { std::mem::transmute_copy::<_, ActivateFn>(&f) })
    })
}

/// The completion handler, in a module of its own: `#[implement]` expands
/// to impls that trip two pedantic lints in this crate, which an `allow` on
/// the struct does not reach.
mod handler {
    #![allow(clippy::inline_always, clippy::ref_as_ptr)]

    use std::mem::ManuallyDrop;
    use std::sync::mpsc::SyncSender;

    use windows::core::{implement, Ref, Result};
    use windows::Win32::Media::Audio::{
        IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
        IActivateAudioInterfaceCompletionHandler_Impl, AUDIOCLIENT_ACTIVATION_PARAMS,
    };
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

    /// What Windows is given to read: the parameters, and the PROPVARIANT
    /// pointing at them. On the heap and owned by the handler, so it lives
    /// as long as Windows holds the handler — even past our own timeout.
    #[repr(C)]
    pub(super) struct ActivationBlob {
        pub(super) params: AUDIOCLIENT_ACTIVATION_PARAMS,
        /// Never dropped as a PROPVARIANT: windows 0.61's `Drop` for it runs
        /// `PropVariantClear`, which would hand our own memory to
        /// `CoTaskMemFree` and corrupt the heap.
        pub(super) prop: ManuallyDrop<PROPVARIANT>,
    }

    /// Agile (`#[implement]`'s default), as Windows requires: it is called
    /// on a worker thread. It only signals; nothing in it can panic.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    pub(super) struct Activated {
        pub(super) done: SyncSender<()>,
        pub(super) blob: *mut ActivationBlob,
    }

    impl Drop for Activated {
        fn drop(&mut self) {
            if !self.blob.is_null() {
                // SAFETY: from Box::into_raw, freed exactly once, here, when
                // the last reference to the handler goes.
                drop(unsafe { Box::from_raw(self.blob) });
            }
        }
    }

    impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
        fn ActivateCompleted(
            &self,
            _op: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
        ) -> Result<()> {
            let _ = self.done.try_send(());
            Ok(())
        }
    }
}

/// A process-loopback client for everything but this process and the ones
/// it started, not yet initialized. Must run with COM initialized.
fn activate_process_loopback() -> Result<IAudioClient, CaptureError> {
    let Some(activate) = activate_fn() else {
        return Err(CaptureError::Unavailable(
            "ActivateAudioInterfaceAsync is not available".to_string(),
        ));
    };
    let blob = Box::into_raw(Box::new(handler::ActivationBlob {
        params: AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: std::process::id(),
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                },
            },
        },
        prop: ManuallyDrop::new(PROPVARIANT::default()),
    }));
    let size = u32::try_from(std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>()).unwrap_or(0);
    // SAFETY: `blob` is a unique, live allocation, written only through this
    // pointer until the handler owns it; the PROPVARIANT points into it.
    let prop: *const PROPVARIANT = unsafe {
        let params = (&raw mut (*blob).params).cast::<u8>();
        (*blob).prop = ManuallyDrop::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        blob: BLOB {
                            cbSize: size,
                            pBlobData: params,
                        },
                    },
                }),
            },
        });
        // ManuallyDrop is repr(transparent).
        (&raw const (*blob).prop).cast::<PROPVARIANT>()
    };
    let (tx, rx) = sync_channel::<()>(1);
    let handler: IActivateAudioInterfaceCompletionHandler =
        handler::Activated { done: tx, blob }.into();
    let mut op_raw: *mut c_void = std::ptr::null_mut();
    // SAFETY: the export's real signature; the handler is lent (Windows
    // takes its own reference) and the parameters live as long as it does.
    unsafe {
        activate(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            prop,
            handler.as_raw(),
            &raw mut op_raw,
        )
    }
    .ok()
    .context("ActivateAudioInterfaceAsync")?;
    if op_raw.is_null() {
        return Err(CaptureError::Unavailable(
            "process loopback gave no operation".to_string(),
        ));
    }
    // SAFETY: on success the out-parameter is an owned operation.
    let op = unsafe { IActivateAudioInterfaceAsyncOperation::from_raw(op_raw) };
    if rx.recv_timeout(ACTIVATE_TIMEOUT).is_err() {
        return Err(CaptureError::Windows {
            context: "ActivateAudioInterfaceAsync (no answer)",
            source: windows::core::Error::from_hresult(ERROR_TIMEOUT.to_hresult()),
        });
    }
    let mut activated = HRESULT(0);
    let mut unknown: Option<IUnknown> = None;
    // SAFETY: valid out-pointers; the operation has completed.
    unsafe { op.GetActivateResult(&raw mut activated, &raw mut unknown) }
        .context("GetActivateResult")?;
    // Released now: on Windows 10 a lingering operation has been seen to
    // break the next activation.
    drop(op);
    activated.ok().context("process loopback activation")?;
    unknown
        .ok_or_else(|| windows::core::Error::from_hresult(E_NOINTERFACE))
        .and_then(|u| u.cast::<IAudioClient>())
        .context("process loopback IAudioClient")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{activate_fn, handler, read_anchor};
    use std::mem::ManuallyDrop;
    use std::sync::mpsc::sync_channel;
    use windows::Win32::Media::Audio::{
        IActivateAudioInterfaceCompletionHandler, AUDIOCLIENT_ACTIVATION_PARAMS,
    };
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

    /// mmdevapi.dll is there and exports the function under the name used —
    /// on CI too, which has no audio devices but has the DLL.
    #[test]
    fn the_activation_export_resolves() {
        assert!(activate_fn().is_some());
    }

    /// The handler signals and, when its last reference goes, frees the
    /// parameters it owns — without touching them as a PROPVARIANT (which
    /// would hand Rust memory to `CoTaskMemFree` and corrupt the heap).
    #[test]
    fn the_completion_handler_signals_and_frees_its_parameters() {
        for _ in 0..100 {
            let blob = Box::into_raw(Box::new(handler::ActivationBlob {
                params: AUDIOCLIENT_ACTIVATION_PARAMS::default(),
                prop: ManuallyDrop::new(PROPVARIANT::default()),
            }));
            let (tx, rx) = sync_channel(1);
            let h: IActivateAudioInterfaceCompletionHandler =
                handler::Activated { done: tx, blob }.into();
            // SAFETY: a live handler; a null operation is allowed by the
            // signature and ignored by ours.
            unsafe { h.ActivateCompleted(None) }.unwrap();
            assert!(rx.try_recv().is_ok(), "no signal");
            // A second completion (Windows never sends one) can't block or
            // panic on the full channel.
            // SAFETY: as above.
            unsafe { h.ActivateCompleted(None) }.unwrap();
            drop(h);
        }
    }

    /// The multimedia scheduler can be had and given back. (A server image
    /// may not run it; there the thread falls back to a high priority.)
    #[test]
    fn the_capture_thread_can_be_made_urgent() {
        std::thread::spawn(|| {
            let h = super::make_urgent();
            if std::env::var_os("CI").is_none() {
                assert!(h.is_some(), "no MMCSS registration");
            }
            super::make_ordinary(h);
        })
        .join()
        .unwrap();
    }

    /// The anchor pairs the counter with an `Instant` read around it.
    #[test]
    fn the_anchor_is_read_tightly() {
        let before = std::time::Instant::now();
        let a = read_anchor().unwrap();
        let after = std::time::Instant::now();
        assert!(a.at >= before && a.at <= after);
        assert!(a.qpc_100ns > 0);
    }
}
