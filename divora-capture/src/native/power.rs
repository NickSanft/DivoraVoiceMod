//! Telling that the PC slept (v1.54.0).
//!
//! Two signals, made one event by [`crate::sleep::Discontinuities`]:
//! Windows' resume notification, registered once per process with a
//! callback (no window needed — the app's windows belong to Tauri, which
//! never sees `WM_POWERBROADCAST`), and the backstop [`SleepDetector`]: how
//! much further the performance counter moved than the interrupt time that
//! leaves sleep out.
//!
//! Only user32 and kernel32 are imported, both already the app's.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Power::{
    RegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTime;
use windows::Win32::UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC};

/// Resumes since the process began watching.
static RESUMES: AtomicU64 = AtomicU64::new(0);

/// Runs on a thread Windows picks, as the PC sleeps or wakes. It must not
/// block: every process shares about two seconds on the way down.
unsafe extern "system" fn on_power(
    _context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    // Every resume sends RESUMEAUTOMATIC; RESUMESUSPEND follows only when
    // a person woke it, so it alone is counted.
    if kind == PBT_APMRESUMEAUTOMATIC {
        RESUMES.fetch_add(1, Ordering::AcqRel);
    }
    0
}

struct Params(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS);
// SAFETY: the context is null and the callback a plain function; Windows
// only reads the struct, and it lives for the whole process.
unsafe impl Sync for Params {}
static PARAMS: Params = Params(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
    Callback: Some(on_power),
    Context: std::ptr::null_mut(),
});

/// Windows' resume count. The first call registers for it; the
/// registration is kept for the life of the process (the callback only
/// counts). If Windows refuses, the backstop alone tells.
pub fn resume_count() -> u64 {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        // SAFETY: `PARAMS` is a 'static struct of the layout the callback
        // form asks for. The handle is deliberately never unregistered (and
        // never wrapped in `Owned`, whose `Free` would call the wrong
        // unregister function).
        let params = std::ptr::from_ref(&PARAMS.0).cast::<c_void>().cast_mut();
        match unsafe { RegisterSuspendResumeNotification(HANDLE(params), DEVICE_NOTIFY_CALLBACK) } {
            Ok(_) => true,
            Err(e) => {
                tracing::info!(error = %e, "no resume notifications; watching the clocks only");
                false
            }
        }
    });
    RESUMES.load(Ordering::Acquire)
}

/// How far the performance counter moved beyond the interrupt time between
/// two readings: the time the PC spent asleep.
pub struct SleepDetector {
    at: Instant,
    unbiased: u64,
}

impl SleepDetector {
    pub fn new() -> Self {
        Self {
            at: Instant::now(),
            unbiased: unbiased(),
        }
    }

    /// Sleep since the last call.
    pub fn slept(&mut self) -> Duration {
        let (at, u) = (Instant::now(), unbiased());
        let slept = crate::sleep::slept(at - self.at, u.saturating_sub(self.unbiased));
        self.at = at;
        self.unbiased = u;
        slept
    }
}

impl Default for SleepDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Both signals, for a loop timed by one `Instant`: checked first thing in
/// each iteration, with the one reading of that clock the iteration uses
/// (see [`crate::sleep::LoopSleep`]).
pub struct SleepWatch {
    watch: crate::sleep::LoopSleep,
    detector: SleepDetector,
    /// Its own clock for telling one sleep from the next: the loop's
    /// restarts after a rebuild, and two sleeps would then look like one.
    origin: Instant,
}

impl SleepWatch {
    pub fn new() -> Self {
        Self {
            watch: crate::sleep::LoopSleep::new(resume_count()),
            detector: SleepDetector::new(),
            origin: Instant::now(),
        }
    }

    /// `Some(when)` if the PC slept since the last check — `when` being the
    /// loop's clock that check was handed, read before the sleep. `clock` is
    /// this iteration's one reading of the loop's clock, taken just before.
    pub fn jumped(&mut self, clock: Duration) -> Option<Duration> {
        self.watch.check(
            clock,
            self.origin.elapsed(),
            resume_count(),
            self.detector.slept(),
        )
    }

    /// The loop's clock starts again at `clock`.
    pub fn restart(&mut self, clock: Duration) {
        self.watch.restart(clock);
    }
}

impl Default for SleepWatch {
    fn default() -> Self {
        Self::new()
    }
}

/// `QueryUnbiasedInterruptTime`, in 100-ns units: time awake.
fn unbiased() -> u64 {
    let mut t = 0u64;
    // SAFETY: a valid out-pointer; the call cannot fail on Windows 7 and up.
    let _ = unsafe { QueryUnbiasedInterruptTime(&raw mut t) };
    t
}

#[cfg(test)]
mod tests {
    use super::{resume_count, SleepDetector};

    /// Registering works from a plain thread with no window, and an awake
    /// loop reads no sleep.
    #[test]
    fn registration_works_and_an_awake_pc_slept_nothing() {
        let before = resume_count();
        assert_eq!(resume_count(), before, "no resume while the test runs");
        let mut d = SleepDetector::new();
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            d.slept() < crate::sleep::Discontinuities::THRESHOLD,
            "awake time is not sleep"
        );
    }
}
