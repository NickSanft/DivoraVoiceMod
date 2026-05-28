//! Audio level metering: short-window RMS plus peak with decay.
//!
//! The RMS is per-buffer, smoothed with a one-pole low-pass so the meter
//! doesn't twitch. The peak holds the loudest sample seen recently and
//! decays by a fixed factor each buffer; any new buffer peak that
//! exceeds the held peak immediately replaces it.

/// Tracks RMS and peak for a single audio channel.
#[derive(Debug, Clone)]
pub struct LevelMeter {
    rms: f32,
    peak: f32,
    /// Fraction of new buffer RMS folded into the smoothed RMS each call.
    /// 1.0 = no smoothing; smaller values = slower attack.
    rms_smoothing: f32,
    /// Multiplier applied to the held peak each buffer (0..1).
    /// 1.0 = peak never falls; smaller = faster fall.
    peak_decay: f32,
}

impl LevelMeter {
    /// Create a new meter with reasonable defaults for a UI meter
    /// (~30 % RMS attack, ~5 % peak decay per buffer).
    #[must_use]
    pub fn new() -> Self {
        Self {
            rms: 0.0,
            peak: 0.0,
            rms_smoothing: 0.3,
            peak_decay: 0.95,
        }
    }

    /// Override the RMS smoothing and peak decay factors.
    #[must_use]
    pub fn with_response(rms_smoothing: f32, peak_decay: f32) -> Self {
        let mut m = Self::new();
        m.rms_smoothing = rms_smoothing.clamp(0.0, 1.0);
        m.peak_decay = peak_decay.clamp(0.0, 1.0);
        m
    }

    /// Fold a buffer of samples into the meter. Designed to be called
    /// from the audio callback — no allocations, no syscalls.
    pub fn process(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            // Still decay the peak even if no audio arrived.
            self.peak *= self.peak_decay;
            return;
        }
        let mut sum_sq: f32 = 0.0;
        let mut buf_peak: f32 = 0.0;
        for &s in samples {
            sum_sq += s * s;
            let abs = s.abs();
            if abs > buf_peak {
                buf_peak = abs;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let buf_rms = (sum_sq / samples.len() as f32).sqrt();
        // One-pole low-pass for RMS.
        self.rms = self.rms * (1.0 - self.rms_smoothing) + buf_rms * self.rms_smoothing;
        // Peak with decay; replace on overshoot.
        self.peak *= self.peak_decay;
        if buf_peak > self.peak {
            self.peak = buf_peak;
        }
    }

    /// Current smoothed RMS, normalized in 0..1 (assumes input is in [-1, 1]).
    #[must_use]
    pub fn rms(&self) -> f32 {
        self.rms
    }

    /// Current held peak, normalized in 0..1.
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// Reset the meter to silence.
    pub fn reset(&mut self) {
        self.rms = 0.0;
        self.peak = 0.0;
    }
}

impl Default for LevelMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::LevelMeter;

    #[test]
    fn silent_buffer_yields_zero_rms() {
        let mut m = LevelMeter::new();
        m.process(&[0.0; 256]);
        assert!(m.rms() < 1e-6);
        assert!(m.peak() < 1e-6);
    }

    #[test]
    fn unit_buffer_yields_unit_rms_after_settle() {
        let mut m = LevelMeter::with_response(1.0, 0.0);
        m.process(&[1.0; 256]);
        // With full smoothing the smoothed RMS equals the buffer RMS.
        assert!((m.rms() - 1.0).abs() < 1e-6);
        // Peak should be exactly 1.0.
        assert!((m.peak() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn peak_decays_when_idle() {
        let mut m = LevelMeter::with_response(0.5, 0.5);
        m.process(&[1.0; 32]);
        let initial_peak = m.peak();
        m.process(&[]);
        m.process(&[]);
        assert!(m.peak() < initial_peak);
    }

    #[test]
    fn peak_replaces_on_overshoot() {
        let mut m = LevelMeter::with_response(0.5, 0.5);
        m.process(&[0.5; 32]);
        let lower = m.peak();
        m.process(&[0.9; 32]);
        assert!(m.peak() > lower);
    }

    #[test]
    fn smoothing_dampens_response() {
        let mut fast = LevelMeter::with_response(1.0, 0.95);
        let mut slow = LevelMeter::with_response(0.1, 0.95);
        let signal = [0.5_f32; 64];
        fast.process(&signal);
        slow.process(&signal);
        assert!(fast.rms() > slow.rms());
    }

    #[test]
    #[allow(clippy::float_cmp)] // reset stores literal 0.0
    fn reset_returns_to_zero() {
        let mut m = LevelMeter::new();
        m.process(&[0.7; 32]);
        assert!(m.rms() > 0.0);
        m.reset();
        assert_eq!(m.rms(), 0.0);
        assert_eq!(m.peak(), 0.0);
    }
}
