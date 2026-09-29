//! Polyphonic voice pool. Owned by the audio output callback; the UI
//! pushes `SoundboardCommand`s through an SPSC channel.

// DSP code inherently casts between integer and float representations
// (sample indexing, sample rate ratios, position counters). Mute the
// noise so the real warnings stand out.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Maximum simultaneous soundboard voices. Picking a fresh voice past
/// this cap steals the oldest playing voice so the latest play always
/// wins.
pub const MAX_VOICES: usize = 16;

/// Commands the UI sends to the mixer. Voice payload uses `Arc<Vec<f32>>`
/// so the decoded buffer is shared without copy and the audio thread
/// keeps it alive only as long as a voice references it.
#[derive(Debug, Clone)]
pub enum SoundboardCommand {
    Play {
        clip_id: String,
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        /// Phase 15: per-tile linear gain (1.0 = unchanged). Applied
        /// per voice on top of the master gain.
        gain: f32,
    },
    /// v1.18.0: like `Play`, but the voice is routed to the **monitor only**
    /// (the local "hear yourself" path) and excluded from the main send — so
    /// e.g. TTS can be previewed locally without the call hearing it. With no
    /// separate monitor device it rides the single output (so it's still
    /// audible). Same fields as `Play`.
    PlayMonitorOnly {
        clip_id: String,
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        gain: f32,
    },
    Stop {
        clip_id: String,
    },
    StopAll,
    /// Phase 15: set the master soundboard gain (linear, 1.0 = unity).
    SetMasterGain(f32),
}

/// Owned audio the audio callback took out of the mixer and must not free.
///
/// Playing a clip replaces whatever occupied the voice slot, and stopping one
/// arrives carrying an owned name. Both used to be dropped on the spot. The
/// counts were small — one free for a `Stop`, one for a `Play` into a fresh
/// slot — but the third case is the one that matters: when the callback holds
/// the last reference to a decoded clip, stealing that slot frees the whole
/// buffer there. Measured at 3 frees for a 1.9 MB clip, one of which is the
/// 1.9 MB itself.
///
/// So it leaves instead, by the same route as a displaced effect chain: out
/// through a preallocated ring to the `divora-graveyard` thread.
pub struct RetiredAudio {
    /// A displaced voice's clip name, or a `Stop`'s own owned name.
    pub clip_id: String,
    /// The decoded clip a displaced voice held, if it had one. `None` for a
    /// `Stop`, which displaces nothing.
    pub samples: Option<Arc<Vec<f32>>>,
}

/// Wire-format snapshot of a playing voice for the UI's progress UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayingClipSnapshot {
    pub clip_id: String,
    pub position_secs: f32,
    pub duration_secs: f32,
}

#[derive(Clone)]
struct Voice {
    clip_id: String,
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    /// Per-tile linear gain captured at play time.
    gain: f32,
    /// v1.18.0: when true this voice is mixed only into the monitor path
    /// (`mix_monitor_into`), never the main send (`mix_into`).
    monitor_only: bool,
    /// Float position into `samples` (so the mixer can interpolate
    /// across engine-rate vs. clip-rate mismatches).
    position: f64,
    active: bool,
    /// Monotonic counter — newer voices have higher values; used to
    /// pick the oldest voice when stealing.
    started_at: u64,
}

impl Voice {
    fn idle() -> Self {
        Self {
            clip_id: String::new(),
            samples: Arc::new(Vec::new()),
            sample_rate: 48_000,
            gain: 1.0,
            monitor_only: false,
            position: 0.0,
            active: false,
            started_at: 0,
        }
    }
}

pub struct SoundboardMixer {
    voices: [Voice; MAX_VOICES],
    counter: u64,
    /// Master gain applied to every voice (linear, 1.0 = unity).
    master_gain: f32,
}

impl SoundboardMixer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            voices: std::array::from_fn(|_| Voice::idle()),
            counter: 0,
            master_gain: 1.0,
        }
    }

    /// Apply a single command, handing back any owned audio it retired.
    ///
    /// Called from the audio callback after draining the SPSC channel, so the
    /// return value is the whole point: this cannot see the engine's graveyard
    /// ring, so what it displaces leaves through the caller. Dropping the
    /// return value instead of routing it does exactly what this change fixed.
    #[must_use]
    pub fn apply(&mut self, cmd: SoundboardCommand) -> Option<RetiredAudio> {
        match cmd {
            SoundboardCommand::Play {
                clip_id,
                samples,
                sample_rate,
                gain,
            } => self.play(clip_id, samples, sample_rate, gain, false),
            SoundboardCommand::PlayMonitorOnly {
                clip_id,
                samples,
                sample_rate,
                gain,
            } => self.play(clip_id, samples, sample_rate, gain, true),
            // The command owns this string and the mixer has no home for it, so
            // it has to leave rather than be dropped here.
            SoundboardCommand::Stop { clip_id } => {
                self.stop(&clip_id);
                Some(RetiredAudio {
                    clip_id,
                    samples: None,
                })
            }
            // Neither of these owns anything: `stop_all` only clears flags, and
            // a gain is a float.
            SoundboardCommand::StopAll => {
                self.stop_all();
                None
            }
            SoundboardCommand::SetMasterGain(g) => {
                self.master_gain = g.clamp(0.0, 4.0);
                None
            }
        }
    }

    /// Start a voice, returning whatever occupied the slot it took.
    ///
    /// `mem::replace` rather than assignment: `*v = Voice { .. }` drops the
    /// previous occupant right here, and an idle slot still owns an `Arc`, a
    /// stopped slot still owns its decoded clip, and both own a name. Handing
    /// the occupant back is what keeps those frees off the audio thread.
    fn play(
        &mut self,
        clip_id: String,
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        gain: f32,
        monitor_only: bool,
    ) -> Option<RetiredAudio> {
        self.counter = self.counter.wrapping_add(1);
        let started_at = self.counter;
        let gain = gain.clamp(0.0, 4.0);
        let fresh = Voice {
            clip_id,
            samples,
            sample_rate,
            gain,
            monitor_only,
            position: 0.0,
            active: true,
            started_at,
        };
        // Prefer an idle slot; otherwise steal the oldest. Found by index and
        // then taken mutably once, because searching mutably and falling back
        // mutably would be two overlapping borrows of the same array.
        let idle = self.voices.iter().position(|v| !v.active);
        let index = match idle {
            Some(i) => i,
            None => self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.started_at)
                .map(|(i, _)| i)?,
        };
        let displaced = std::mem::replace(&mut self.voices[index], fresh);
        Some(RetiredAudio {
            clip_id: displaced.clip_id,
            samples: Some(displaced.samples),
        })
    }

    fn stop(&mut self, clip_id: &str) {
        for v in &mut self.voices {
            if v.active && v.clip_id == clip_id {
                v.active = false;
            }
        }
    }

    fn stop_all(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
    }

    /// Number of currently playing voices.
    #[must_use]
    pub fn active_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }

    /// Mix every active **main-send** voice into `output` (additive). Voices
    /// that run off the end of their buffer self-deactivate. Monitor-only
    /// voices are skipped (see [`SoundboardMixer::mix_monitor_into`]).
    pub fn mix_into(&mut self, output: &mut [f32], engine_rate: u32) {
        self.mix_filtered(output, engine_rate, false);
    }

    /// v1.18.0: mix every active **monitor-only** voice into `output`
    /// (additive). The engine calls this on the monitor signal (or, with no
    /// separate monitor device, the single output) exactly once per callback,
    /// so these voices advance. Main-send voices are skipped here.
    pub fn mix_monitor_into(&mut self, output: &mut [f32], engine_rate: u32) {
        self.mix_filtered(output, engine_rate, true);
    }

    fn mix_filtered(&mut self, output: &mut [f32], engine_rate: u32, monitor_only: bool) {
        if output.is_empty() {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let engine_rate_f = f64::from(engine_rate);
        let master = self.master_gain;
        for v in &mut self.voices {
            if !v.active || v.monitor_only != monitor_only {
                continue;
            }
            let step = f64::from(v.sample_rate) / engine_rate_f;
            let voice_gain = v.gain * master;
            let buf = v.samples.as_ref();
            let len = buf.len();
            if len < 2 {
                v.active = false;
                continue;
            }
            for slot in output.iter_mut() {
                let idx = v.position as usize;
                if idx + 1 >= len {
                    v.active = false;
                    break;
                }
                #[allow(clippy::cast_possible_truncation)]
                let frac = (v.position - idx as f64) as f32;
                let sample = buf[idx] * (1.0 - frac) + buf[idx + 1] * frac;
                *slot += sample * voice_gain;
                v.position += step;
            }
        }
    }

    /// Snapshot of currently-playing voices for UI progress events.
    #[must_use]
    pub fn snapshot(&self, engine_rate: u32) -> Vec<PlayingClipSnapshot> {
        let _ = engine_rate; // reserved for future per-voice details
        self.voices
            .iter()
            .filter(|v| v.active)
            .map(|v| {
                #[allow(clippy::cast_precision_loss)]
                let position_secs = (v.position as f32) / v.sample_rate as f32;
                #[allow(clippy::cast_precision_loss)]
                let duration_secs = v.samples.len() as f32 / v.sample_rate as f32;
                PlayingClipSnapshot {
                    clip_id: v.clip_id.clone(),
                    position_secs,
                    duration_secs,
                }
            })
            .collect()
    }
}

impl Default for SoundboardMixer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{SoundboardCommand, SoundboardMixer, MAX_VOICES};

    /// Apply a command and drop what it retired, which is what the graveyard
    /// thread does with it — just not on the audio thread.
    fn apply(m: &mut SoundboardMixer, cmd: SoundboardCommand) {
        drop(m.apply(cmd));
    }

    use std::sync::Arc;

    fn ramp(samples: usize) -> Arc<Vec<f32>> {
        Arc::new(
            (0..samples)
                .map(|i| (i as f32) / (samples as f32))
                .collect(),
        )
    }

    #[test]
    fn empty_mixer_does_not_touch_output() {
        let mut m = SoundboardMixer::new();
        let mut out = [0.5_f32; 64];
        m.mix_into(&mut out, 48_000);
        for s in out {
            assert!((s - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn play_adds_into_output() {
        let mut m = SoundboardMixer::new();
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "a".into(),
                samples: Arc::new(vec![0.5_f32; 128]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        let mut out = vec![0.0_f32; 64];
        m.mix_into(&mut out, 48_000);
        for s in &out {
            assert!((s - 0.5).abs() < 1e-3, "expected ~0.5, got {s}");
        }
    }

    #[test]
    fn voice_deactivates_at_end_of_buffer() {
        let mut m = SoundboardMixer::new();
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "a".into(),
                samples: Arc::new(vec![0.5_f32; 32]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        assert_eq!(m.active_voice_count(), 1);
        let mut out = vec![0.0_f32; 128];
        m.mix_into(&mut out, 48_000);
        assert_eq!(m.active_voice_count(), 0);
    }

    #[test]
    fn stop_deactivates_matching_voice() {
        let mut m = SoundboardMixer::new();
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "keep".into(),
                samples: Arc::new(vec![0.5_f32; 1024]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "kill".into(),
                samples: Arc::new(vec![0.5_f32; 1024]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        assert_eq!(m.active_voice_count(), 2);
        apply(
            &mut m,
            SoundboardCommand::Stop {
                clip_id: "kill".into(),
            },
        );
        assert_eq!(m.active_voice_count(), 1);
    }

    #[test]
    fn stop_all_clears_every_voice() {
        let mut m = SoundboardMixer::new();
        for i in 0..4 {
            apply(
                &mut m,
                SoundboardCommand::Play {
                    clip_id: format!("v{i}"),
                    samples: Arc::new(vec![0.5_f32; 1024]),
                    sample_rate: 48_000,
                    gain: 1.0,
                },
            );
        }
        assert_eq!(m.active_voice_count(), 4);
        apply(&mut m, SoundboardCommand::StopAll);
        assert_eq!(m.active_voice_count(), 0);
    }

    #[test]
    fn sample_rate_mismatch_is_interpolated_not_pitched_up() {
        // 24 kHz clip played on a 48 kHz engine should take twice as
        // long to consume — i.e. after N output samples, the read
        // position should be ~N/2 source samples.
        let mut m = SoundboardMixer::new();
        let clip = ramp(2_000);
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "slow".into(),
                samples: clip.clone(),
                sample_rate: 24_000,
                gain: 1.0,
            },
        );
        let mut out = vec![0.0_f32; 1_000];
        m.mix_into(&mut out, 48_000);
        // After 1000 output samples at 24/48 ratio = 500 source samples.
        // The clip ran out at index ~2000, so we should still be active
        // if we consumed less than that.
        assert_eq!(m.active_voice_count(), 1);
    }

    #[test]
    fn polyphony_sums_voices() {
        let mut m = SoundboardMixer::new();
        for _ in 0..3 {
            apply(
                &mut m,
                SoundboardCommand::Play {
                    clip_id: "x".into(),
                    samples: Arc::new(vec![0.3_f32; 1024]),
                    sample_rate: 48_000,
                    gain: 1.0,
                },
            );
        }
        let mut out = vec![0.0_f32; 64];
        m.mix_into(&mut out, 48_000);
        for s in &out {
            assert!(
                (s - 0.9).abs() < 1e-3,
                "expected ~0.9 (3 voices at 0.3), got {s}"
            );
        }
    }

    #[test]
    fn max_voices_steals_oldest() {
        let mut m = SoundboardMixer::new();
        for i in 0..MAX_VOICES {
            apply(
                &mut m,
                SoundboardCommand::Play {
                    clip_id: format!("voice-{i}"),
                    samples: Arc::new(vec![0.1_f32; 4096]),
                    sample_rate: 48_000,
                    gain: 1.0,
                },
            );
        }
        assert_eq!(m.active_voice_count(), MAX_VOICES);
        // Add a 17th — should bump the oldest.
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "voice-new".into(),
                samples: Arc::new(vec![0.1_f32; 4096]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        assert_eq!(m.active_voice_count(), MAX_VOICES);
        let snap = m.snapshot(48_000);
        assert!(snap.iter().any(|s| s.clip_id == "voice-new"));
        // The very first voice ("voice-0") should be gone.
        assert!(!snap.iter().any(|s| s.clip_id == "voice-0"));
    }

    #[test]
    fn per_voice_gain_scales_output() {
        let mut m = SoundboardMixer::new();
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "g".into(),
                samples: Arc::new(vec![0.5_f32; 128]),
                sample_rate: 48_000,
                gain: 0.5,
            },
        );
        let mut out = vec![0.0_f32; 64];
        m.mix_into(&mut out, 48_000);
        for s in &out {
            assert!(
                (s - 0.25).abs() < 1e-3,
                "0.5 sample × 0.5 gain = 0.25, got {s}"
            );
        }
    }

    #[test]
    fn master_gain_scales_every_voice() {
        let mut m = SoundboardMixer::new();
        apply(&mut m, SoundboardCommand::SetMasterGain(0.5));
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "g".into(),
                samples: Arc::new(vec![0.4_f32; 128]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        let mut out = vec![0.0_f32; 64];
        m.mix_into(&mut out, 48_000);
        for s in &out {
            assert!((s - 0.2).abs() < 1e-3, "0.4 × master 0.5 = 0.2, got {s}");
        }
    }

    #[test]
    fn gains_clamp_to_safe_range() {
        let mut m = SoundboardMixer::new();
        apply(&mut m, SoundboardCommand::SetMasterGain(99.0)); // clamps to 4.0
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "g".into(),
                samples: Arc::new(vec![0.1_f32; 128]),
                sample_rate: 48_000,
                gain: 99.0, // clamps to 4.0
            },
        );
        let mut out = vec![0.0_f32; 64];
        m.mix_into(&mut out, 48_000);
        // 0.1 × 4.0 (gain) × 4.0 (master) = 1.6 — clamped, not exploded.
        for s in &out {
            assert!((s - 1.6).abs() < 1e-3, "clamped gains → 1.6, got {s}");
        }
    }

    #[test]
    fn snapshot_reports_duration_and_progress() {
        let mut m = SoundboardMixer::new();
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "progress".into(),
                samples: Arc::new(vec![0.1_f32; 9_600]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        let mut out = vec![0.0_f32; 480]; // 10 ms at 48 kHz
        m.mix_into(&mut out, 48_000);
        let snap = m.snapshot(48_000);
        let v = snap.iter().find(|s| s.clip_id == "progress").unwrap();
        assert!((v.duration_secs - 0.2).abs() < 1e-3);
        assert!(v.position_secs > 0.009);
        assert!(v.position_secs < 0.012);
    }

    #[test]
    fn monitor_only_routes_to_monitor_path_not_main_send() {
        let mut m = SoundboardMixer::new();
        apply(
            &mut m,
            SoundboardCommand::Play {
                clip_id: "main".into(),
                samples: Arc::new(vec![0.5_f32; 1024]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        apply(
            &mut m,
            SoundboardCommand::PlayMonitorOnly {
                clip_id: "preview".into(),
                samples: Arc::new(vec![0.5_f32; 1024]),
                sample_rate: 48_000,
                gain: 1.0,
            },
        );
        // Main send carries ONLY the normal clip (~0.5); if the monitor-only
        // clip leaked in it'd sum to ~1.0.
        let mut main = vec![0.0_f32; 64];
        m.mix_into(&mut main, 48_000);
        assert!(
            (main[0] - 0.5).abs() < 1e-3,
            "main[0] = {} (expected ~0.5)",
            main[0]
        );
        // Monitor path carries ONLY the monitor-only clip (~0.5), not the
        // main-send clip.
        let mut mon = vec![0.0_f32; 64];
        m.mix_monitor_into(&mut mon, 48_000);
        assert!(
            (mon[0] - 0.5).abs() < 1e-3,
            "mon[0] = {} (expected ~0.5)",
            mon[0]
        );
    }

    // ---- the drain frees nothing on the audio thread ------------------------
    //
    // `apply` runs in the output callback. Until v1.51.2 it dropped what it
    // displaced right there: a `Play` replaced a voice slot with `*v = Voice`,
    // and a `Stop` arrived owning a name that had nowhere to go. Measured on
    // this mixer before the change — 1 free for a `Stop`, 1 for a `Play` into a
    // never-used slot, and **3 for a `Play` that steals a slot holding the last
    // reference to a 1.9 MB decoded clip**, one of those three being the 1.9 MB
    // itself. That last one is why this is worth fixing: not the count, the size.
    //
    // The counting allocator is per-thread, so these are safe alongside the
    // rest of the suite — see `crate::alloc_probe`.

    use crate::alloc_probe::{arm, disarm};

    /// A clip nothing else owns, so retiring it is a real deallocation.
    fn lone_clip(len: usize) -> Arc<Vec<f32>> {
        Arc::new(vec![0.2_f32; len])
    }

    fn play(clip_id: &str, samples: Arc<Vec<f32>>) -> SoundboardCommand {
        SoundboardCommand::Play {
            clip_id: clip_id.to_string(),
            samples,
            sample_rate: 48_000,
            gain: 1.0,
        }
    }

    /// Measure one `apply`, retiring what it hands back the way the engine does
    /// — outside the window, standing in for the graveyard thread.
    fn measure(m: &mut SoundboardMixer, cmd: SoundboardCommand) -> (usize, usize) {
        arm();
        let retired = m.apply(cmd);
        let counts = disarm();
        drop(retired);
        counts
    }

    #[test]
    fn a_play_into_a_fresh_slot_frees_nothing() {
        let mut m = SoundboardMixer::new();
        let clip = lone_clip(1024);
        // Warm: the first `to_string` and `vec!` are the caller's, not ours.
        let cmd = play("first", clip);
        assert_eq!(
            measure(&mut m, cmd),
            (0, 0),
            "a never-used slot still owns an `Arc`, and dropping it here is a \
             free on the audio thread"
        );
    }

    #[test]
    fn a_stop_frees_nothing_even_though_it_owns_its_name() {
        let mut m = SoundboardMixer::new();
        drop(m.apply(play("a", lone_clip(64))));
        let cmd = SoundboardCommand::Stop {
            clip_id: "a".to_string(),
        };
        assert_eq!(
            measure(&mut m, cmd),
            (0, 0),
            "the `Stop`'s own owned name has no home in the mixer, so it has to \
             leave rather than be dropped in the callback"
        );
    }

    /// The case that matters: every slot holds a clip nothing else owns, so
    /// stealing one used to free the whole decoded buffer in the callback.
    #[test]
    fn stealing_a_slot_does_not_free_the_clip_it_held() {
        let mut m = SoundboardMixer::new();
        for i in 0..MAX_VOICES {
            drop(m.apply(play(&format!("lone{i}"), lone_clip(480_000))));
        }
        let cmd = play("next", lone_clip(64));
        let (allocs, frees) = measure(&mut m, cmd);
        assert_eq!(
            (allocs, frees),
            (0, 0),
            "stealing a slot cost {allocs} allocations and {frees} frees — one \
             of those frees is a 1.9 MB buffer, inside a callback with a few \
             milliseconds of budget"
        );
    }

    #[test]
    fn stop_all_and_a_gain_change_retire_nothing_at_all() {
        let mut m = SoundboardMixer::new();
        for i in 0..4 {
            drop(m.apply(play(&format!("c{i}"), lone_clip(128))));
        }
        // `stop_all` only clears flags and a gain is a float, so there is
        // nothing owned to hand back — and nothing to free either.
        arm();
        let a = m.apply(SoundboardCommand::StopAll);
        let b = m.apply(SoundboardCommand::SetMasterGain(0.5));
        let counts = disarm();
        assert!(a.is_none() && b.is_none(), "neither retires anything");
        assert_eq!(counts, (0, 0));
    }

    /// A long run, because a single measured call can miss a cost that only
    /// appears once a slot has been reused.
    #[test]
    fn a_thousand_plays_and_stops_free_nothing() {
        let mut m = SoundboardMixer::new();
        let shared = lone_clip(4096);
        let mut total = (0_usize, 0_usize);
        for i in 0..1000 {
            let cmd = play(&format!("c{i}"), Arc::clone(&shared));
            let (a1, f1) = measure(&mut m, cmd);
            let stop = SoundboardCommand::Stop {
                clip_id: format!("c{i}"),
            };
            let (a2, f2) = measure(&mut m, stop);
            total.0 += a1 + a2;
            total.1 += f1 + f2;
        }
        assert_eq!(
            total,
            (0, 0),
            "{} allocations and {} frees across 1000 play/stop pairs",
            total.0,
            total.1
        );
    }

    /// Retiring the occupant must not change what the mixer does. A slot that
    /// was playing is replaced, an idle slot is filled, and the retired value
    /// carries exactly what left.
    #[test]
    fn what_comes_back_is_what_the_slot_was_holding() {
        let mut m = SoundboardMixer::new();
        // A fresh slot hands back its placeholder: no name, an empty clip.
        let first = m
            .apply(play("one", lone_clip(32)))
            .expect("something retired");
        assert!(first.clip_id.is_empty(), "a never-used slot has no name");
        assert_eq!(
            first.samples.as_deref().map(Vec::len),
            Some(0),
            "a never-used slot holds an empty clip"
        );
        assert_eq!(m.active_voice_count(), 1);

        // Fill the rest, then steal: the retired value is the oldest voice.
        for i in 1..MAX_VOICES {
            drop(m.apply(play(&format!("v{i}"), lone_clip(32))));
        }
        assert_eq!(m.active_voice_count(), MAX_VOICES);
        let stolen = m
            .apply(play("late", lone_clip(32)))
            .expect("something retired");
        assert_eq!(stolen.clip_id, "one", "the oldest voice should be stolen");
        assert_eq!(stolen.samples.as_deref().map(Vec::len), Some(32));
        assert_eq!(m.active_voice_count(), MAX_VOICES, "still full");

        // A `Stop` retires only its own name.
        let stopped = m
            .apply(SoundboardCommand::Stop {
                clip_id: "v1".to_string(),
            })
            .expect("the name has to leave");
        assert_eq!(stopped.clip_id, "v1");
        assert!(stopped.samples.is_none(), "a Stop displaces no clip");
        assert_eq!(m.active_voice_count(), MAX_VOICES - 1);
    }
}
