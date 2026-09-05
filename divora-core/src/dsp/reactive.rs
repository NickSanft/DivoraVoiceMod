//! Reactive modulation (v1.46.0) — the **dry** input envelope drives effect
//! parameters, so raising your voice pushes the character of the voice with
//! it. Deliberately *not* emotion recognition: no model, no classification, no
//! label. It measures how loud you are, nothing more.
//!
//! ### Why the source must be the dry signal
//!
//! The envelope is measured on the mic buffer *before* [`super::EffectChain`]
//! runs on it. Two reasons, both load-bearing:
//!
//! 1. **No feedback loop.** Following the chain's *output* while modulating
//!    the chain means louder → more drive → louder. The pre-chain buffer
//!    cannot form that loop.
//! 2. **Not the AGC'd signal.** `audio::loudness` runs *after* the chain and
//!    its entire job is removing level variation — following it would mean
//!    following a signal something else has already flattened.
//!
//! ### Detection
//!
//! Mean-square (not peak): on voiced speech a rectified peak follower ripples
//! at the fundamental (85–255 Hz), so it would need a release long enough to
//! hide its own ripple. Mean-square integrates that away and tracks perceived
//! loudness. Accumulated per sample; converted to dB and smoothed once per
//! block, which is the rate the modulation is applied at anyway.
//!
//! Smoothing and range-mapping both happen in **dB**, because loudness
//! perception is logarithmic: a normal speaking voice at −30 dBFS and a
//! committed shout at −14 dBFS are 0.032 and 0.20 in linear amplitude, so a
//! linear map would waste ~90% of its travel and only wake up on a scream.
//!
//! Release is ~25× attack on purpose. The modulation spectrum of speech peaks
//! at 4–5 Hz (syllable rate); a release fast enough to resolve syllables makes
//! the parameter flutter at exactly that rate, which reads as *something is
//! wrong with my voice*. A long release resolves once per phrase instead.

use serde::{Deserialize, Serialize};

use super::envelope::EnvelopeFollower;
use super::{EffectChain, EffectKind};

/// Detector high-pass corner (Hz), detector path only — rejects desk thumps,
/// plosive rumble, and handling noise that would otherwise trigger on a
/// consonant.
const DETECTOR_HP_HZ: f32 = 110.0;
/// Floor of the response window (dBFS): below this, no modulation at all.
pub const DEFAULT_FLOOR_DB: f32 = -42.0;
/// Top of the response window (dBFS): a committed shout.
pub const DEFAULT_CEIL_DB: f32 = -14.0;
/// How far below the floor the envelope may sit. Bounding it stops a long
/// silence winding the envelope down to −∞, which would otherwise cost a full
/// attack ramp to climb back from ("anti-windup").
const WINDUP_MARGIN_DB: f32 = 6.0;
/// Mean-square floor, so digital silence yields a finite dB value.
const MS_FLOOR: f32 = 1e-12;
/// Detector input bound (linear), ≈ +18 dBFS — far above the response
/// ceiling, so clamping here cannot affect any level the window can express.
///
/// This is a *detector* clamp, not a limiter: the audio itself is never
/// touched. It exists because `is_finite()` alone is not enough here. A huge
/// but perfectly finite sample (say `1e30`, which every other effect handles
/// fine because they track `|x|` linearly) squares to `1e60` and overflows an
/// `f32` accumulator to `+inf` — which then latches the envelope at full
/// depth forever, since the release can never bring infinity back down.
const DETECTOR_CEIL: f32 = 8.0;

const DEFAULT_ATTACK_MS: f32 = 12.0;
const DEFAULT_HOLD_MS: f32 = 60.0;
const DEFAULT_RELEASE_MS: f32 = 300.0;

/// One modulation route — which parameter moves, and how far.
///
/// Targets are addressed by **kind + occurrence**, never by chain index: a
/// drag-reorder in the preset editor must not silently re-point the
/// modulation at a different effect.
#[derive(Debug, Clone, PartialEq)]
pub struct ModRoute {
    pub kind: EffectKind,
    /// Which occurrence of `kind` in the chain (0 = the first).
    pub nth: u8,
    pub key: &'static str,
    /// The preset-authored value the modulation offsets *from*.
    ///
    /// The modulator has to own this: [`super::AudioEffect`] exposes no
    /// getter, and `SetChain` rebuilds every effect from its spec, so there is
    /// nothing on the audio thread to read a parameter's authored value back
    /// from.
    pub base: f32,
    /// Signed. Negative moves the parameter DOWN as the voice rises.
    pub depth: f32,
    /// Clamp bounds, from the effect catalog for this key.
    pub min: f32,
    pub max: f32,
}

impl ModRoute {
    /// The value to write for a shaped envelope in 0..=1.
    #[must_use]
    pub fn value_at(&self, shaped: f32) -> f32 {
        self.depth
            .mul_add(shaped, self.base)
            .clamp(self.min, self.max)
    }
}

/// The dry-signal envelope, mapped to a 0..=1 modulation depth.
pub struct ReactiveSource {
    floor_db: f32,
    ceil_db: f32,
    attack_ms: f32,
    hold_ms: f32,
    release_ms: f32,
    // --- state ---
    env_db: EnvelopeFollower,
    /// One-pole low-pass state for the detector high-pass.
    hp_lp: f32,
    /// Remaining hold, in samples.
    hold_left: f32,
}

impl ReactiveSource {
    #[must_use]
    pub fn new() -> Self {
        let rest = DEFAULT_FLOOR_DB - WINDUP_MARGIN_DB;
        Self {
            floor_db: DEFAULT_FLOOR_DB,
            ceil_db: DEFAULT_CEIL_DB,
            attack_ms: DEFAULT_ATTACK_MS,
            hold_ms: DEFAULT_HOLD_MS,
            release_ms: DEFAULT_RELEASE_MS,
            env_db: EnvelopeFollower::new(rest),
            hp_lp: 0.0,
            hold_left: 0.0,
        }
    }

    /// Response window in dBFS. `floor` is where modulation starts, `ceil`
    /// where it saturates; a collapsed or inverted window is rejected so the
    /// normalisation can never divide by zero.
    pub fn set_window(&mut self, floor_db: f32, ceil_db: f32) {
        let floor = floor_db.clamp(-90.0, -3.0);
        let ceil = ceil_db.clamp(-90.0, 0.0);
        if ceil - floor >= 1.0 {
            self.floor_db = floor;
            self.ceil_db = ceil;
        }
    }

    pub fn set_timing(&mut self, attack_ms: f32, hold_ms: f32, release_ms: f32) {
        self.attack_ms = attack_ms.clamp(1.0, 200.0);
        self.hold_ms = hold_ms.clamp(0.0, 1000.0);
        self.release_ms = release_ms.clamp(20.0, 2000.0);
    }

    /// Reset to rest. Call on enable, on engine start/stop, and on a device
    /// change — a follower carrying a stale envelope would otherwise slam the
    /// modulation on at the first buffer after a rebuild.
    pub fn reset(&mut self) {
        self.env_db.reset(self.floor_db - WINDUP_MARGIN_DB);
        self.hp_lp = 0.0;
        self.hold_left = 0.0;
    }

    /// Advance over one block of **dry, pre-effects** mono and return the
    /// shaped modulation depth in 0..=1.
    ///
    /// RT-safe: no allocation, no locks. Roughly 4 flops per sample plus one
    /// `log10` and one `exp` pair per block.
    pub fn observe(&mut self, dry: &[f32], sample_rate: u32) -> f32 {
        if dry.is_empty() {
            return self.shaped();
        }
        #[allow(clippy::cast_precision_loss)]
        let sr = sample_rate.max(1) as f32;
        #[allow(clippy::cast_precision_loss)]
        let n = dry.len() as f32;

        // Per sample: high-pass the detector path, accumulate mean-square.
        let hp_c = (-std::f32::consts::TAU * DETECTOR_HP_HZ / sr).exp();
        let mut sum_sq = 0.0_f32;
        for &s in dry {
            let x = if s.is_finite() {
                s.clamp(-DETECTOR_CEIL, DETECTOR_CEIL)
            } else {
                0.0
            };
            self.hp_lp = hp_c.mul_add(self.hp_lp, (1.0 - hp_c) * x);
            let hp = x - self.hp_lp;
            sum_sq = hp.mul_add(hp, sum_sq);
        }

        let target_db = 10.0 * (sum_sq / n).max(MS_FLOOR).log10();
        let rest_db = self.floor_db - WINDUP_MARGIN_DB;

        // Block-rate coefficients: exp(-block_seconds / tau). Deriving them
        // from the block length rather than a fixed count keeps the timing
        // identical across buffer sizes and sample rates.
        let block_secs = n / sr;
        let atk = (-block_secs / (self.attack_ms / 1000.0)).exp();
        let rel = (-block_secs / (self.release_ms / 1000.0)).exp();

        let rising = target_db > self.env_db.value();
        if rising {
            // Refresh the hold whenever the signal pushes the envelope up.
            self.hold_left = self.hold_ms / 1000.0 * sr;
            self.env_db.attack_on_rise(target_db, atk, rel);
        } else if self.hold_left > 0.0 {
            // Hold: bridge stop closures, plosive gaps and inter-word pauses
            // so the modulation doesn't drop out on every hard consonant.
            self.hold_left -= n;
        } else {
            self.env_db.attack_on_rise(target_db.max(rest_db), atk, rel);
        }

        self.shaped()
    }

    /// Normalise the envelope across the dB window and shape it.
    ///
    /// Smoothstep is used rather than a linear ramp because its zero
    /// derivative at both ends gives a natural dead zone at the bottom (room
    /// noise never chatters the effect on) and a settled top (shouting harder
    /// stops escalating instead of banging into the clamp).
    fn shaped(&self) -> f32 {
        let span = self.ceil_db - self.floor_db;
        let u = ((self.env_db.value() - self.floor_db) / span).clamp(0.0, 1.0);
        u * u * 2.0_f32.mul_add(-u, 3.0)
    }
}

impl Default for ReactiveSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Parameters reactive modulation is allowed to drive.
///
/// The rule: a parameter is safe only if it **scales a signal**. Anything that
/// changes where a sample is read from (`echo.time` recomputes an integer read
/// offset per buffer), how many samples exist, or where a filter pole sits
/// (`eq` / `radio_bandpass` frequencies rebuild stateful biquads) will click or
/// ring when stepped at block rate — so those are absent and cannot be routed.
/// Every entry here is a memoryless waveshaper gain, a dry/wet crossfade, or an
/// amplitude scalar.
///
/// This table is also what turns a wire-format `String` key into the
/// `&'static str` the realtime path needs, so on the IPC path — the only one a
/// frontend can reach — an unlisted key simply fails to resolve and the
/// whitelist needs no separate check. Note this is *not* a type-level
/// guarantee: [`ModRoute`]'s fields are public, so in-crate Rust code could
/// still build one by hand.
///
/// Tuples are `(kind, key, min, max)`; the bounds are authoritative here rather
/// than trusted from the UI.
const MOD_TARGETS: &[(EffectKind, &str, f32, f32)] = &[
    (EffectKind::Distortion, "drive", 0.0, 100.0),
    (EffectKind::Distortion, "warmth", 0.0, 100.0),
    (EffectKind::Reverb, "mix", 0.0, 100.0),
    (EffectKind::Chorus, "mix", 0.0, 100.0),
    (EffectKind::Harmonizer, "mix", 0.0, 100.0),
    (EffectKind::Robot, "mix", 0.0, 100.0),
    (EffectKind::Warble, "mix", 0.0, 100.0),
    (EffectKind::Breath, "amount", 0.0, 100.0),
    (EffectKind::Tremolo, "depth", 0.0, 100.0),
];

/// Whether `kind`/`key` may be modulated, and the bounds if so.
#[must_use]
pub fn mod_target_range(kind: EffectKind, key: &str) -> Option<(&'static str, f32, f32)> {
    MOD_TARGETS
        .iter()
        .find(|(k, name, _, _)| *k == kind && *name == key)
        .map(|(_, name, min, max)| (*name, *min, *max))
}

/// One route as it crosses the IPC boundary. `key` is an owned `String` here
/// and a `&'static str` once resolved — see [`MOD_TARGETS`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactiveRouteSpec {
    pub kind: EffectKind,
    #[serde(default)]
    pub nth: u8,
    pub key: String,
    /// The preset-authored value to offset from.
    pub base: f32,
    /// Signed; negative moves the parameter down as the voice rises.
    pub depth: f32,
}

/// The whole reactive configuration, sent as one message so the audio thread
/// can never observe a half-applied state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactiveConfig {
    pub enabled: bool,
    /// Master scale over every route depth, 0..=1.
    pub intensity: f32,
    pub floor_db: f32,
    pub ceil_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    pub routes: Vec<ReactiveRouteSpec>,
}

impl Default for ReactiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 0.7,
            floor_db: DEFAULT_FLOOR_DB,
            ceil_db: DEFAULT_CEIL_DB,
            attack_ms: DEFAULT_ATTACK_MS,
            hold_ms: DEFAULT_HOLD_MS,
            release_ms: DEFAULT_RELEASE_MS,
            routes: Vec::new(),
        }
    }
}

impl ReactiveConfig {
    /// Resolve every route against [`MOD_TARGETS`], dropping any the whitelist
    /// does not permit.
    ///
    /// Allocates, so this runs on the **engine thread** — see
    /// [`ResolvedReactive`]. Values are hardened here rather than trusted:
    /// `base` is clamped into the target's range, and a non-finite or absurd
    /// `depth` is neutralised. A raw `depth` of `1e39` arrives from JSON as
    /// `f32::INFINITY`, and `value_at(0.0)` would then be
    /// `inf.mul_add(0.0, base)` = **NaN** — at rest, i.e. immediately — which
    /// would poison every sample downstream to the device.
    #[must_use]
    pub fn resolve_routes(&self) -> Vec<ModRoute> {
        self.routes
            .iter()
            .filter_map(|spec| {
                let (key, min, max) = mod_target_range(spec.kind, &spec.key)?;
                let span = max - min;
                let depth = if spec.depth.is_finite() {
                    spec.depth.clamp(-span, span)
                } else {
                    0.0
                };
                let base = if spec.base.is_finite() {
                    spec.base.clamp(min, max)
                } else {
                    min
                };
                Some(ModRoute {
                    kind: spec.kind,
                    nth: spec.nth,
                    key,
                    base,
                    depth,
                    min,
                    max,
                })
            })
            .collect()
    }

    /// Resolve the whole config into the form the audio thread consumes.
    #[must_use]
    pub fn resolve(&self) -> ResolvedReactive {
        ResolvedReactive {
            enabled: self.enabled,
            intensity: self.intensity,
            floor_db: self.floor_db,
            ceil_db: self.ceil_db,
            attack_ms: self.attack_ms,
            hold_ms: self.hold_ms,
            release_ms: self.release_ms,
            routes: self.resolve_routes(),
        }
    }
}

/// A [`ReactiveConfig`] with its routes already resolved against the
/// whitelist.
///
/// Resolution happens on the engine thread so the audio callback never runs
/// the `filter_map`/`collect`, nor drops the incoming config's `String`s.
/// Handing the callback this type instead leaves it a single `Vec` free when
/// it swaps the route table.
#[derive(Debug, Clone)]
pub struct ResolvedReactive {
    pub enabled: bool,
    pub intensity: f32,
    pub floor_db: f32,
    pub ceil_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    pub routes: Vec<ModRoute>,
}

/// Owns the source and the route table, and writes the routed parameters.
///
/// Deliberately lives **beside** [`super::EffectChain`] rather than inside it:
/// `SetChain` replaces the chain wholesale on every preset switch, which would
/// wipe (or worse, silently invalidate) any routing stored within it.
pub struct ReactiveModulator {
    source: ReactiveSource,
    routes: Vec<ModRoute>,
    enabled: bool,
    /// Master scale over every route's depth, 0..=1 (the single "Intensity"
    /// control the UI exposes).
    intensity: f32,
    /// Last depth written, published for the UI meter.
    last_depth: f32,
    /// Set while modulation is live, so switching the feature off restores
    /// every routed parameter to its authored value exactly once instead of
    /// leaving it frozen wherever the modulation happened to stop.
    needs_restore: bool,
}

impl ReactiveModulator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            source: ReactiveSource::new(),
            routes: Vec::new(),
            enabled: false,
            intensity: 0.7,
            last_depth: 0.0,
            needs_restore: false,
        }
    }

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.clamp(0.0, 1.0);
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Enabling resets the source, so the modulation fades in from rest
    /// instead of slamming to whatever the first buffer happens to contain.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.source.reset();
            self.last_depth = 0.0;
        }
        self.enabled = enabled;
    }

    /// Apply an already-resolved configuration.
    ///
    /// Takes `chain` because **the outgoing routes must be restored before the
    /// table is replaced**. `restore_bases` can only iterate the routes it
    /// currently holds, so swapping first would strand any route that the new
    /// config drops or re-points — it would keep whatever value the modulation
    /// last wrote, for the life of the chain. That is reachable two ways a UI
    /// will hit routinely: sending `{enabled: false, routes: []}` to turn the
    /// panel off, and editing a route to target a different parameter.
    ///
    /// Called from the command drain at the top of the output callback (the
    /// same place `DspCommand::SetChain` is applied). It frees the previous
    /// route `Vec`; resolution itself already happened on the engine thread.
    pub fn configure(&mut self, cfg: &ResolvedReactive, chain: &mut EffectChain) {
        if self.needs_restore {
            self.restore_bases(chain);
            self.needs_restore = false;
        }
        self.set_intensity(cfg.intensity);
        self.source.set_window(cfg.floor_db, cfg.ceil_db);
        self.source
            .set_timing(cfg.attack_ms, cfg.hold_ms, cfg.release_ms);
        self.routes.clear();
        self.routes.extend_from_slice(&cfg.routes);
        self.set_enabled(cfg.enabled);
    }

    /// The most recent modulation depth, 0..=1, for the UI meter.
    #[must_use]
    pub const fn depth(&self) -> f32 {
        self.last_depth
    }

    /// Observe the **dry, pre-effects** buffer and write every routed
    /// parameter into `chain`. Call immediately before `chain.process`.
    ///
    /// When disabled this restores each route's base value once and then does
    /// nothing, so turning the feature off leaves the chain at its authored
    /// values rather than frozen wherever the modulation happened to stop.
    pub fn apply(&mut self, dry: &[f32], chain: &mut EffectChain, sample_rate: u32) {
        if !self.enabled {
            if self.needs_restore {
                self.restore_bases(chain);
                self.needs_restore = false;
            }
            return;
        }
        self.needs_restore = true;
        let depth = self.source.observe(dry, sample_rate) * self.intensity;
        self.last_depth = depth;
        for route in &self.routes {
            if let Some(index) = chain.index_of_kind(route.kind, route.nth) {
                chain.set_param_at(index, route.key, route.value_at(depth));
            }
        }
    }

    /// Write every route back to its authored base value. Used when the
    /// feature is switched off so a modulated parameter does not stay stuck.
    pub fn restore_bases(&mut self, chain: &mut EffectChain) {
        for route in &self.routes {
            if let Some(index) = chain.index_of_kind(route.kind, route.nth) {
                chain.set_param_at(index, route.key, route.base);
            }
        }
        self.last_depth = 0.0;
    }
}

impl Default for ReactiveModulator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{ModRoute, ReactiveSource, DEFAULT_CEIL_DB, DEFAULT_FLOOR_DB};
    use crate::dsp::EffectKind;

    const SR: u32 = 48_000;

    /// A block of steady tone at a given dBFS, long enough that the detector
    /// high-pass has settled.
    fn tone_at(db: f32, frames: usize) -> Vec<f32> {
        let amp = 10_f32.powf(db / 20.0) * std::f32::consts::SQRT_2;
        (0..frames)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / SR as f32;
                amp * (std::f32::consts::TAU * 440.0 * t).sin()
            })
            .collect()
    }

    /// Drive the source with `db` for `blocks` blocks and return the settled
    /// modulation depth.
    fn settle(src: &mut ReactiveSource, db: f32, blocks: usize) -> f32 {
        let mut out = 0.0;
        for _ in 0..blocks {
            let buf = tone_at(db, 480); // 10 ms blocks
            out = src.observe(&buf, SR);
        }
        out
    }

    #[test]
    fn silence_produces_no_modulation() {
        let mut src = ReactiveSource::new();
        let quiet = vec![0.0_f32; 480];
        for _ in 0..100 {
            src.observe(&quiet, SR);
        }
        assert!(
            src.shaped() < 1e-6,
            "silence must not modulate, got {}",
            src.shaped()
        );
    }

    #[test]
    fn below_the_floor_is_a_dead_zone() {
        let mut src = ReactiveSource::new();
        let depth = settle(&mut src, DEFAULT_FLOOR_DB - 10.0, 200);
        assert!(depth < 1e-3, "below floor must stay dead, got {depth}");
    }

    #[test]
    fn a_shout_saturates_the_window() {
        let mut src = ReactiveSource::new();
        let depth = settle(&mut src, DEFAULT_CEIL_DB + 6.0, 200);
        assert!(depth > 0.99, "above ceiling must saturate, got {depth}");
    }

    #[test]
    fn depth_increases_monotonically_with_level() {
        let mut last = -1.0_f32;
        for step in 0..7 {
            #[allow(clippy::cast_precision_loss)]
            let db = DEFAULT_FLOOR_DB + (step as f32) * 5.0;
            let mut src = ReactiveSource::new();
            let depth = settle(&mut src, db, 200);
            assert!(
                depth >= last - 1e-6,
                "depth must not fall as level rises ({db} dBFS gave {depth}, previous {last})"
            );
            last = depth;
        }
    }

    #[test]
    fn output_is_always_within_zero_and_one() {
        let mut src = ReactiveSource::new();
        for db in [-90.0, -60.0, -42.0, -24.0, -14.0, -3.0, 0.0] {
            let depth = settle(&mut src, db, 60);
            assert!(
                (0.0..=1.0).contains(&depth),
                "{db} dBFS produced out-of-range {depth}"
            );
        }
    }

    /// Attack is fast, release is slow — the asymmetry that stops the
    /// modulation fluttering at syllable rate.
    #[test]
    fn release_is_much_slower_than_attack() {
        let mut src = ReactiveSource::new();
        // Rise: a couple of blocks should already move a long way.
        let after_rise = settle(&mut src, -16.0, 6);
        assert!(after_rise > 0.5, "attack should be quick, got {after_rise}");

        // Fall: the same number of silent blocks should barely move it.
        let quiet = vec![0.0_f32; 480];
        for _ in 0..6 {
            src.observe(&quiet, SR);
        }
        let after_fall = src.shaped();
        assert!(
            after_fall > after_rise * 0.5,
            "release should be slow (was {after_rise}, now {after_fall})"
        );
    }

    /// Timing must not depend on the device's buffer size.
    #[test]
    fn timing_is_independent_of_block_size() {
        let depth_for = |frames: usize| {
            let mut src = ReactiveSource::new();
            let total = 48_000; // 1 second of audio either way
            let blocks = total / frames;
            let mut out = 0.0;
            for _ in 0..blocks {
                out = src.observe(&tone_at(-20.0, frames), SR);
            }
            out
        };
        let small = depth_for(128);
        let large = depth_for(2048);
        assert!(
            (small - large).abs() < 0.05,
            "buffer size must not change the response ({small} vs {large})"
        );
    }

    #[test]
    fn non_finite_input_cannot_latch_the_follower() {
        let mut src = ReactiveSource::new();
        let mut poisoned = vec![f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30];
        poisoned.extend(std::iter::repeat_n(0.2_f32, 475));
        for _ in 0..20 {
            let depth = src.observe(&poisoned, SR);
            assert!(depth.is_finite(), "depth must stay finite, got {depth}");
            assert!((0.0..=1.0).contains(&depth), "depth out of range: {depth}");
        }
        // And it recovers to silence afterwards rather than latching high.
        let quiet = vec![0.0_f32; 480];
        for _ in 0..400 {
            src.observe(&quiet, SR);
        }
        assert!(src.shaped() < 1e-3, "should fall back to rest");
    }

    #[test]
    fn reset_returns_to_rest() {
        let mut src = ReactiveSource::new();
        assert!(settle(&mut src, -10.0, 100) > 0.9);
        src.reset();
        assert!(src.shaped() < 1e-6, "reset must return to no modulation");
    }

    #[test]
    fn an_empty_block_holds_the_last_value() {
        let mut src = ReactiveSource::new();
        let before = settle(&mut src, -20.0, 100);
        let after = src.observe(&[], SR);
        assert!(
            (before - after).abs() < 1e-9,
            "empty block must not disturb"
        );
    }

    #[test]
    fn a_collapsed_window_is_rejected() {
        let mut src = ReactiveSource::new();
        src.set_window(-20.0, -20.0); // zero span — must be ignored
        let depth = settle(&mut src, -28.0, 100);
        assert!(
            depth.is_finite() && (0.0..=1.0).contains(&depth),
            "collapsed window must not divide by zero, got {depth}"
        );
    }

    // ---- routes ----

    #[test]
    fn a_route_offsets_from_its_base_and_clamps() {
        let route = ModRoute {
            kind: EffectKind::Distortion,
            nth: 0,
            key: "drive",
            base: 18.0,
            depth: 45.0,
            min: 0.0,
            max: 70.0,
        };
        assert!((route.value_at(0.0) - 18.0).abs() < 1e-6, "rest = base");
        assert!((route.value_at(0.5) - 40.5).abs() < 1e-6, "half depth");
        assert!(
            (route.value_at(1.0) - 63.0).abs() < 1e-6,
            "full depth, under the clamp"
        );
    }

    #[test]
    fn a_route_clamps_to_the_catalog_range() {
        let route = ModRoute {
            kind: EffectKind::Distortion,
            nth: 0,
            key: "drive",
            base: 60.0,
            depth: 45.0, // would reach 105
            min: 0.0,
            max: 70.0,
        };
        assert!(
            (route.value_at(1.0) - 70.0).abs() < 1e-6,
            "must clamp to max"
        );
    }

    /// Negative depth is legal: a parameter may fall as the voice rises.
    #[test]
    fn a_negative_depth_moves_the_parameter_down() {
        let route = ModRoute {
            kind: EffectKind::Reverb,
            nth: 0,
            key: "mix",
            base: 40.0,
            depth: -30.0,
            min: 0.0,
            max: 100.0,
        };
        assert!((route.value_at(0.0) - 40.0).abs() < 1e-6);
        assert!((route.value_at(1.0) - 10.0).abs() < 1e-6);
    }
}
