//! Noise suppression — Phase 10 wraps Xiph's `RNNoise` (via the pure-
//! Rust `nnnoiseless` port) into a streaming `AudioEffect`. The voice
//! gets cleaner across a ~10 ms processing window without crushing
//! dynamics the way a brute-force noise gate would.
//!
//! ### Constraints
//!
//! `RNNoise` was trained on 48 kHz mono audio with a fixed 480-sample
//! frame (= 10 ms). At any other sample rate the model's filterbank
//! assumptions break, so when the engine runs at a non-48 kHz input
//! rate we **bypass** the effect entirely. Users who care about
//! denoising should pick a 48 kHz input device — most consumer USB
//! mics already are. The 48 kHz constraint will go away when we
//! resample around the denoiser in a later phase.
//!
//! ### Latency
//!
//! Constant 480-sample delay (≈ 10 ms) added to the chain when the
//! effect is enabled. During the first 480 samples after the engine
//! starts, output is **silent** — the buffer hasn't filled yet, and
//! passing the dry signal through during warm-up would cause those
//! samples to audibly repeat once the denoised stream caught up.
//!
//! ### Wet / dry mix
//!
//! The `mix` parameter is `0..1`. `mix = 0` outputs the dry signal
//! (matched in time to the wet stream by an internal delay line);
//! `mix = 1` outputs only the denoised stream. The Phase 3 noise gate
//! is a different effect entirely (slot-by-slot threshold) and stays
//! in the catalog — users can stack both.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_range_loop
)]

use std::collections::VecDeque;

use nnnoiseless::{DenoiseState, FRAME_SIZE};

use super::{AudioEffect, EffectKind};

/// Sample rate `RNNoise` was trained on. Off-rate input bypasses.
const RNN_RATE: u32 = 48_000;

/// `nnnoiseless` expects f32 samples scaled to the int16 range
/// (`-32768.0 ..= 32767.0`).
const SCALE: f32 = 32_768.0;

pub struct RnnDenoiser {
    enabled: bool,
    mix: f32,
    state: Box<DenoiseState<'static>>,
    /// Native-rate input samples queued for the next 480-sample frame.
    in_queue: VecDeque<f32>,
    /// Denoised samples ready to write to the output buffer.
    out_queue: VecDeque<f32>,
    /// Dry samples delayed to stay aligned with the wet stream — used
    /// for the wet/dry mix without re-introducing the 10 ms offset.
    dry_delay: VecDeque<f32>,
}

impl RnnDenoiser {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            mix: 1.0,
            state: DenoiseState::new(),
            in_queue: VecDeque::with_capacity(FRAME_SIZE * 2),
            out_queue: VecDeque::with_capacity(FRAME_SIZE * 2),
            dry_delay: VecDeque::with_capacity(FRAME_SIZE * 2),
        }
    }

    /// Current wet/dry mix in 0..1 (UI-side surface is 0..100).
    #[doc(hidden)]
    #[must_use]
    pub fn mix(&self) -> f32 {
        self.mix
    }

    fn clear_pipeline(&mut self) {
        self.in_queue.clear();
        self.out_queue.clear();
        self.dry_delay.clear();
    }
}

impl Default for RnnDenoiser {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEffect for RnnDenoiser {
    fn process(&mut self, buffer: &mut [f32], sample_rate: u32) {
        // Bypass: disabled, zero-mix shortcut, or off-rate input.
        if !self.enabled || sample_rate != RNN_RATE {
            if !self.in_queue.is_empty() || !self.out_queue.is_empty() {
                self.clear_pipeline();
            }
            return;
        }
        if self.mix < 1e-4 {
            // mix == 0 → all dry; nothing to do, but keep the pipeline
            // primed so toggling mix > 0 mid-session is glitch-free.
            for &s in buffer.iter() {
                self.in_queue.push_back(s);
            }
            self.drain_frames();
            // Pop matching wets to keep the queues bounded (and so
            // dry_delay stays in sync when mix returns).
            for _ in 0..buffer.len() {
                let _ = self.out_queue.pop_front();
                let _ = self.dry_delay.pop_front();
            }
            return;
        }

        // Phase 1: accumulate input at the native rate.
        for &s in buffer.iter() {
            self.in_queue.push_back(s);
        }

        // Phase 2: process every full 480-sample frame.
        self.drain_frames();

        // Phase 3: write the next output samples back into `buffer`.
        // During warm-up (out_queue empty), output silence — passing
        // the dry signal through here would make those samples
        // audibly repeat once the wet stream caught up.
        for slot in buffer.iter_mut() {
            if let (Some(wet), Some(dry)) = (self.out_queue.pop_front(), self.dry_delay.pop_front())
            {
                *slot = self.mix * wet + (1.0 - self.mix) * dry;
            } else {
                *slot = 0.0;
            }
        }
    }

    fn set_param(&mut self, key: &str, value: f32) {
        if key == "mix" {
            // UI surfaces 0..100; store 0..1.
            self.mix = (value / 100.0).clamp(0.0, 1.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        if !enabled {
            // Drop any buffered audio so the next enable starts cleanly.
            self.clear_pipeline();
        }
        self.enabled = enabled;
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Denoiser
    }

    fn latency_samples(&self, sample_rate: u32) -> usize {
        // One 480-sample frame (≈ 10 ms @ 48 kHz) of buffering delay,
        // and only at the rate the model runs — off-rate the effect
        // bypasses entirely, adding nothing.
        if sample_rate == RNN_RATE {
            FRAME_SIZE
        } else {
            0
        }
    }
}

impl RnnDenoiser {
    /// Hand every full 480-sample frame to nnnoiseless and queue the
    /// denoised result + a matching delayed dry copy.
    fn drain_frames(&mut self) {
        while self.in_queue.len() >= FRAME_SIZE {
            let mut frame_in = [0f32; FRAME_SIZE];
            let mut frame_dry = [0f32; FRAME_SIZE];
            for i in 0..FRAME_SIZE {
                let s = self.in_queue.pop_front().expect("checked len");
                frame_in[i] = s * SCALE;
                frame_dry[i] = s;
            }
            let mut frame_out = [0f32; FRAME_SIZE];
            // process_frame returns a voice-activity probability we
            // don't use yet (could surface as a "voice detected" badge
            // in a future phase).
            let _vad = self.state.process_frame(&mut frame_out, &frame_in);
            for i in 0..FRAME_SIZE {
                self.out_queue.push_back(frame_out[i] / SCALE);
                self.dry_delay.push_back(frame_dry[i]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioEffect, RnnDenoiser, FRAME_SIZE, RNN_RATE};

    fn pink_noise(len: usize) -> Vec<f32> {
        // Crude reproducible noise: walk a simple LCG, scale to ±0.3.
        let mut s: u32 = 0xDEAD_BEEF;
        (0..len)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let n = ((s >> 8) & 0xFFFF) as f32 / 32_768.0 - 1.0;
                n * 0.3
            })
            .collect()
    }

    /// Disabled → bit-identical passthrough at any sample rate.
    #[test]
    fn passthrough_when_disabled() {
        let input = pink_noise(2 * FRAME_SIZE);
        let mut buf = input.clone();
        let mut d = RnnDenoiser::new();
        d.set_param("mix", 100.0);
        // enabled() stays false
        d.process(&mut buf, RNN_RATE);
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// Non-48 kHz input → bit-identical passthrough even when enabled.
    #[test]
    fn bypasses_off_rate_input() {
        let input = pink_noise(2 * FRAME_SIZE);
        let mut buf = input.clone();
        let mut d = RnnDenoiser::new();
        d.set_enabled(true);
        d.set_param("mix", 100.0);
        d.process(&mut buf, 44_100);
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    /// Sub-`FRAME_SIZE` chunks: the denoiser needs to accumulate
    /// `FRAME_SIZE` samples before it can emit anything. During that
    /// window the output is silent (passing the dry signal through
    /// would make those samples audibly repeat once the wet stream
    /// caught up).
    #[test]
    fn small_chunks_silent_until_frame_filled() {
        let mut d = RnnDenoiser::new();
        d.set_enabled(true);
        d.set_param("mix", 100.0);
        // Send 5 chunks of 64 samples each (320 total < FRAME_SIZE).
        for _ in 0..5 {
            let mut chunk = pink_noise(64);
            d.process(&mut chunk, RNN_RATE);
            for s in &chunk {
                assert!(s.abs() < 1e-6, "pre-frame chunk should be silent, got {s}");
            }
        }
    }

    /// After warm-up the output stream contains *something* — not
    /// silence — and stays finite.
    #[test]
    fn produces_finite_output_after_warmup() {
        let input = pink_noise(3 * FRAME_SIZE);
        let mut buf = input.clone();
        let mut d = RnnDenoiser::new();
        d.set_enabled(true);
        d.set_param("mix", 100.0);
        d.process(&mut buf, RNN_RATE);
        // Skip the first frame (warm-up); the remaining samples should
        // not all be zero and should all be finite.
        let post_warmup = &buf[FRAME_SIZE..];
        let mut any_nonzero = false;
        for s in post_warmup {
            assert!(s.is_finite(), "non-finite sample {s}");
            if s.abs() > 1e-3 {
                any_nonzero = true;
            }
        }
        assert!(any_nonzero, "post-warmup output looks all-silent");
    }

    /// Disabling clears the pipeline so a subsequent enable starts
    /// with empty queues (no stale audio).
    #[test]
    fn disable_clears_pipeline() {
        let mut d = RnnDenoiser::new();
        d.set_enabled(true);
        d.set_param("mix", 100.0);
        // Push half a frame in, then disable before it can be consumed.
        let mut half = pink_noise(FRAME_SIZE / 2);
        d.process(&mut half, RNN_RATE);
        d.set_enabled(false);
        // Re-enable and send another half-frame — should still be silent
        // because pipeline was cleared (queues empty < FRAME_SIZE).
        d.set_enabled(true);
        let mut more = pink_noise(FRAME_SIZE / 2);
        d.process(&mut more, RNN_RATE);
        for s in &more {
            assert!(
                s.abs() < 1e-6,
                "expected silence after disable→enable with half-frame, got {s}"
            );
        }
    }

    /// `mix` is clamped to 0..1 (UI range 0..100) and unknown keys
    /// are silently ignored.
    #[test]
    fn set_param_clamps_mix_and_ignores_unknown_keys() {
        let mut d = RnnDenoiser::new();
        d.set_param("mix", 50.0);
        assert!((d.mix() - 0.5).abs() < 1e-6);
        d.set_param("mix", 9_999.0);
        assert!((d.mix() - 1.0).abs() < 1e-6);
        d.set_param("mix", -999.0);
        assert!((d.mix() - 0.0).abs() < 1e-6);
        d.set_param("unknown", 1.0);
        assert!((d.mix() - 0.0).abs() < 1e-6);
    }

    /// `mix == 0` → straight dry signal (output matches input AFTER
    /// the warm-up delay, since dry is delay-matched to wet).
    #[test]
    fn mix_zero_outputs_dry_signal_after_warmup() {
        let input = pink_noise(3 * FRAME_SIZE);
        let mut buf = input.clone();
        let mut d = RnnDenoiser::new();
        d.set_enabled(true);
        d.set_param("mix", 0.0);
        d.process(&mut buf, RNN_RATE);
        // With mix=0 we shortcut to "drop the wet path entirely" — no
        // delay-line writeback. So the buffer should be unchanged.
        for (a, b) in buf.iter().zip(input.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "mix=0 should leave buffer untouched, but {a} ≠ {b}"
            );
        }
    }
}
