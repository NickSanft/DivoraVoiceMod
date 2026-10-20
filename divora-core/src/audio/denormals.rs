//! v1.56.0: subnormal floats count as zero on the audio thread (risk K16).
//!
//! While muted, the chain hears exact zeros for as long as the mute lasts,
//! and its recursive states (reverb combs, echo feedback, biquads, the
//! loudness EMA, the STFT magnitudes) decay into subnormal floats. On x86
//! each operation on one can cost a microcode assist. Without a flush, 21
//! of the 24 bundled presets cost 2-16 × max(speech, pause) per buffer three
//! minutes into a mute (`a_long_mute_costs_at_most_twice_speech_or_a_pause`, a release
//! build on the development PC's i7-14700K). A first fix fed the starved
//! chain ±1e-18, a normal float, instead of zeros. It didn't help: the noise
//! gate that 23 of the 24 bundled presets start with took that down 60 dB to
//! about 1e-21, still normal, and loudness's `x * x` and the STFT magnitudes
//! squared it below the normal range. With FTZ and DAZ set the CPU treats
//! subnormals as zero, and on the same PC every preset's mute cost
//! 0.68-0.99 × max(speech, pause) three minutes in.

/// MXCSR's flush-to-zero bit: a subnormal result is written as zero.
#[cfg(target_arch = "x86_64")]
const FTZ: u32 = 0x8000;

/// MXCSR's denormals-are-zero bit: a subnormal input is read as zero.
#[cfg(target_arch = "x86_64")]
const DAZ: u32 = 0x0040;

/// Sets FTZ and DAZ on the calling thread, for every SSE float op it runs
/// from then on; other threads keep IEEE gradual underflow. cpal owns the
/// output stream's thread, so its callback calls this first, every buffer.
/// A no-op on other targets.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)] // the crate's one production `unsafe`; see SAFETY
#[inline]
pub(crate) fn flush_to_zero() {
    let mut csr = 0_u32;
    // SAFETY: the asm touches only `csr`, a local it is given a pointer to,
    // and MXCSR, whose other bits (rounding, exception masks and flags) it
    // writes back as it read them; it uses no stack and can't unwind. What it
    // changes is how this thread's float ops treat subnormals. Rust assumes
    // gradual underflow (std's `_mm_setcsr` docs call changing DAZ undefined
    // behaviour for that reason), so an op the compiler folds at build time
    // can still give a subnormal that the same op at run time would flush.
    // Nothing on the audio thread gets memory safety from a float, so that
    // is a difference under 1.2e-38 in a sample, which is the point.
    unsafe {
        core::arch::asm!(
            "stmxcsr [{csr}]",
            "or dword ptr [{csr}], {bits}",
            "ldmxcsr [{csr}]",
            csr = in(reg) &raw mut csr,
            bits = const FTZ | DAZ,
            options(nostack),
        );
    }
}

/// See the x86-64 version: nothing to set here.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub(crate) fn flush_to_zero() {}

#[cfg(test)]
#[cfg(target_arch = "x86_64")]
mod tests {
    use super::flush_to_zero;
    use std::hint::black_box;

    /// Half the smallest normal: a subnormal result from normal inputs.
    fn underflow() -> f32 {
        black_box(f32::MIN_POSITIVE) * black_box(0.5)
    }

    /// The smallest subnormal times 2^30: a normal result from a subnormal
    /// input (about 1.5e-36).
    fn from_subnormal() -> f32 {
        black_box(f32::from_bits(1)) * black_box(1_073_741_824.0)
    }

    /// FTZ zeroes a subnormal result and DAZ reads a subnormal input as zero,
    /// on the thread that called it and no other. Its own thread, so the
    /// flags can't leak into another test.
    #[test]
    fn flush_to_zero_flushes_this_thread_only() {
        std::thread::spawn(|| {
            // A new thread starts with gradual underflow.
            assert!(underflow().is_subnormal());
            assert!(from_subnormal().is_normal());
            flush_to_zero();
            assert_eq!(underflow().to_bits(), 0, "FTZ: a subnormal result");
            assert_eq!(from_subnormal().to_bits(), 0, "DAZ: a subnormal input");
            // Twice is the same as once.
            flush_to_zero();
            assert_eq!(underflow().to_bits(), 0);
        })
        .join()
        .unwrap();
        assert!(underflow().is_subnormal(), "it leaked to another thread");
        assert!(from_subnormal().is_normal(), "it leaked to another thread");
    }
}
