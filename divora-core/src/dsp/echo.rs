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
            let read_pos = (self.write_pos + buf_len - delay) % buf_len;
            let delayed = self.buffer[read_pos];
            let wet = delayed * 0.5;
            self.buffer[self.write_pos] = *sample + delayed * self.feedback;
            self.write_pos = (self.write_pos + 1) % buf_len;
            *sample += wet;
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
}
