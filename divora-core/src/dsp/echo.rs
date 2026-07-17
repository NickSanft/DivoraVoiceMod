//! Echo / delay. A circular buffer is written every sample; the read
//! head trails by `time_ms`. The delayed signal is summed back into the
//! input at `feedback` strength and mixed with the dry signal at 50/50.

use super::{AudioEffect, EffectKind};

/// Max delay buffer. 2 seconds at 192 kHz covers every realistic
/// (`time_ms` × `sample_rate`) configuration the UI exposes.
const MAX_DELAY_FRAMES: usize = 192_000 * 2;

pub struct Echo {
    enabled: bool,
    time_ms: f32,
    feedback: f32, // 0..0.95
    buffer: Vec<f32>,
    write_pos: usize,
}

impl Echo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            time_ms: 240.0,
            feedback: 0.35,
            buffer: vec![0.0; MAX_DELAY_FRAMES],
            write_pos: 0,
        }
    }
}

impl Default for Echo {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for Echo {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate as f32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let delay = ((self.time_ms / 1000.0) * sr) as usize;
        let delay = delay.clamp(1, self.buffer.len() - 1);
        let buf_len = self.buffer.len();
        for sample in buffer.iter_mut() {
            // Guard the input so a stray NaN/Inf can't latch into the feedback
            // buffer and recirculate forever (feedback runs up to 0.95).
            let x = if sample.is_finite() { *sample } else { 0.0 };
            let read_pos = (self.write_pos + buf_len - delay) % buf_len;
            let delayed = self.buffer[read_pos];
            let wet = delayed * 0.5;
            let fed = x + delayed * self.feedback;
            self.buffer[self.write_pos] = if fed.is_finite() { fed } else { 0.0 };
            self.write_pos = (self.write_pos + 1) % buf_len;
            let out = x + wet;
            *sample = if out.is_finite() { out } else { x };
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        match key {
            "time" => self.time_ms = value.clamp(1.0, 2000.0),
            "fb" => self.feedback = (value / 100.0).clamp(0.0, 0.95),
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Echo
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, Echo};

    #[test]
    fn delayed_impulse_appears_after_delay() {
        let mut e = Echo::new();
        e.set_enabled(true);
        e.set_param("time", 10.0);
        e.set_param("fb", 0.0);
        // 10 ms at 48 kHz = 480 samples.
        let mut buf = vec![0.0_f32; 600];
        buf[0] = 1.0;
        e.process(&mut buf, 48000);
        // Original sample plus a delayed wet copy at index 480.
        assert!((buf[0] - 1.0).abs() < 0.01);
        assert!(buf[480] > 0.4);
    }

    #[test]
    fn feedback_increases_echo_count() {
        let mut e = Echo::new();
        e.set_enabled(true);
        e.set_param("time", 1.0); // 1 ms = 48 samples at 48 kHz
        e.set_param("fb", 80.0);
        let mut buf = vec![0.0_f32; 500];
        buf[0] = 1.0;
        e.process(&mut buf, 48000);
        // Count peaks above 0.05.
        let peaks = buf.iter().filter(|s| s.abs() > 0.05).count();
        assert!(peaks > 3);
    }

    #[test]
    fn survives_nan_without_latching_feedback() {
        let mut e = Echo::new();
        e.set_enabled(true);
        e.set_param("fb", 90.0);
        // A short delay (1 ms = 48 samples) so the read head circles back to
        // the poisoned write positions within the second buffer below — this
        // genuinely closes the feedback loop, not just the input path.
        e.set_param("time", 1.0);
        // Poison the input, then feed clean audio: the NaN/Inf must not latch
        // into the feedback buffer and recirculate forever.
        let mut buf = [f32::NAN, f32::INFINITY, -f32::INFINITY, 0.5, 0.5, 0.5];
        e.process(&mut buf, 48_000);
        for s in buf {
            assert!(s.is_finite(), "output must stay finite, got {s}");
        }
        // A later buffer of clean audio must also stay finite (no latched NaN
        // recirculating through the delay line).
        let mut buf2 = vec![0.2_f32; 2400];
        e.process(&mut buf2, 48_000);
        for s in buf2 {
            assert!(s.is_finite(), "no latched non-finite state, got {s}");
        }
    }
}
