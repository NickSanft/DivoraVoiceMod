//! Period estimation for [`super::Analyzer`] — a YIN-style detector with an
//! explicit octave guard.
//!
//! ### Why YIN and not bare autocorrelation
//!
//! The raw autocorrelation peak of voiced speech is routinely at *half* the
//! true period, because the second harmonic is often stronger than the
//! fundamental. YIN's cumulative-mean-normalised difference function plus its
//! absolute-threshold rule fixes that by construction: it takes the *first*
//! lag that dips below the threshold, i.e. the longest period that explains
//! the waveform, which is the definition of the fundamental.
//!
//! ### Why it runs at ~12 kHz
//!
//! The difference function costs `window × lags` per frame, and both scale
//! with the sample rate, so the cost is quadratic in it. Nothing above ~1 kHz
//! carries period information we need, so the detector decimates to a target
//! of [`PITCH_RATE_HZ`] first — a 16× saving at 96 kHz. The integer factor is
//! derived from the sample rate, and every bound below is in Hz or seconds, so
//! 44.1 / 48 / 96 kHz all land on the same answer. Lag quantisation at 12 kHz
//! is 0.57 semitones at the top of the search range, which would be far too
//! coarse for a semitone-denominated pitch *range*; parabolic interpolation of
//! the difference minimum (YIN step 5) takes it back under 0.05 st.
//!
//! ### The octave guard (two parts, both needed)
//!
//! Octave errors are the failure users actually see: a pitch-range readout
//! that intermittently doubles is worse than no readout at all, and a gaming
//! headset behind a gate and a denoiser can thin the fundamental until a
//! period-based tracker latches onto the second harmonic.
//!
//! * **In-frame** ([`OCTAVE_REL`]): after the threshold pick, the sub-octave
//!   lag wins only if it is *relatively* far more periodic. A pure tone is
//!   genuinely periodic at twice its period too, so an absolute comparison
//!   would halve every sine; the [`OCTAVE_EPS`] floor says differences below
//!   2 % aperiodicity are not evidence, which leaves pure tones alone while
//!   still catching a frame whose fundamental has been filtered out.
//! * **Across frames** ([`OCTAVE_TOL_ST`]): a candidate sitting ~an octave
//!   from the running median of recent accepted values is re-tested at the
//!   corrected lag and only kept if its confidence clearly beats the
//!   correction's. A *real* octave move sustains, so the lock releases itself
//!   after [`OCTAVE_LOCK_MAX`] consecutive corrections rather than pinning the
//!   tracker to a register the speaker has left.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

/// Bottom of the search range, Hz. Below the ~85 Hz floor of adult male modal
/// speech with headroom for creak, and above mains hum and desk rumble.
pub const F0_MIN_HZ: f32 = 70.0;
/// Top of the search range, Hz.
///
/// Well above the ~255 Hz ceiling usually quoted for adult female modal
/// speech, because the **after-effects tap is the point**: this app ships
/// presets that pitch a voice up an octave, and at a 400 Hz ceiling a 500 Hz
/// wet signal read as exactly 250 — the panel told the user a +12 semitone
/// preset had changed nothing, on the one comparison that half of the card
/// exists for. Review measured it. A higher ceiling costs a slightly longer
/// scan and nothing else: YIN takes the first dip under threshold, so a
/// normal voice is still found at its own period.
pub const F0_MAX_HZ: f32 = 1_000.0;
/// Rate the detector wants to work at. The integer decimation factor is
/// `round(sample_rate / PITCH_RATE_HZ)`, so 44.1 kHz lands at 11.025 kHz and
/// both 48 and 96 kHz land exactly here.
pub const PITCH_RATE_HZ: f32 = 12_000.0;
/// Anti-alias corner for the decimator, Hz. Three cascaded one-poles put
/// ~36 dB on content at 6 kHz, and keeping only the first several harmonics is
/// what the difference function wants anyway.
const DECIM_LP_HZ: f32 = 1_500.0;
/// Frame the difference function integrates over. 60 ms leaves ~45 ms of
/// integration after the longest lag, i.e. three periods at [`F0_MIN_HZ`].
pub const PITCH_FRAME_S: f32 = 0.060;
/// YIN's absolute threshold on the normalised difference.
const YIN_THRESHOLD: f32 = 0.15;
/// Worst normalised difference still called voiced. White noise's minimum
/// hovers near 1.0, so this rejects it with a wide margin while accepting the
/// breathy and gated frames real speech is full of.
const VOICED_MAX_D: f32 = 0.45;
/// In-frame sub-octave test: the longer period must cut the normalised
/// difference to this fraction of the shorter one's.
const OCTAVE_REL: f32 = 0.6;
/// Floor added to both sides of that ratio, in normalised-difference units.
/// Without it the test compares two numbers near zero and coin-flips on pure
/// tones.
const OCTAVE_EPS: f32 = 0.02;
/// How close to a whole octave a candidate has to be for the cross-frame guard
/// to look at it, in semitones.
const OCTAVE_TOL_ST: f32 = 3.0;
/// How much better the candidate's confidence must be than the correction's to
/// survive the cross-frame guard.
const OCTAVE_CONF_MARGIN: f32 = 0.10;
/// Accepted values kept for the running median.
const REF_LEN: usize = 24;
/// Values needed before the cross-frame guard turns on.
const REF_MIN: usize = 8;
/// Consecutive corrections after which the guard concedes the speaker really
/// did change register and re-references. Half a second: longer than any
/// transient thinning of the fundamental, short enough to follow a genuine
/// move of register within a phrase.
const OCTAVE_LOCK_MAX: usize = 25;

/// One frame's verdict.
#[derive(Debug, Clone, Copy)]
pub struct Pitch {
    pub hz: f32,
    /// `1 - d'(tau)`, clamped to 0..=1. Periodicity, not loudness.
    pub confidence: f32,
}

/// Cascaded one-pole low-pass plus integer decimation.
pub struct Decimator {
    lp: [f32; 3],
    coeff: f32,
    factor: usize,
    phase: usize,
}

impl Decimator {
    pub fn new(sample_rate: f32) -> Self {
        let factor = (sample_rate / PITCH_RATE_HZ).round().max(1.0) as usize;
        // Time constant in Hz against the *input* rate, so the corner sits at
        // DECIM_LP_HZ whatever the device is running at.
        let coeff = 1.0 - (-std::f32::consts::TAU * DECIM_LP_HZ / sample_rate).exp();
        Self {
            lp: [0.0; 3],
            coeff: coeff.clamp(0.0, 1.0),
            factor,
            phase: 0,
        }
    }

    /// Decimated rate this produces.
    pub fn rate(&self, sample_rate: f32) -> f32 {
        sample_rate / self.factor as f32
    }

    /// Feed one input sample; yields a decimated sample every `factor` calls.
    pub fn push(&mut self, x: f32) -> Option<f32> {
        let mut v = x;
        for stage in &mut self.lp {
            *stage += self.coeff * (v - *stage);
            v = *stage;
        }
        self.phase += 1;
        if self.phase >= self.factor {
            self.phase = 0;
            Some(v)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.lp = [0.0; 3];
        self.phase = 0;
    }
}

/// YIN-style detector. Every buffer is allocated in [`PitchDetector::new`];
/// [`PitchDetector::estimate`] allocates nothing.
pub struct PitchDetector {
    rate: f32,
    tau_min: usize,
    tau_max: usize,
    /// Difference function, indexed by lag.
    diff: Vec<f32>,
    /// Cumulative-mean-normalised difference, indexed by lag.
    cmnd: Vec<f32>,
    /// Recent accepted values, for the cross-frame guard.
    hist: Vec<f32>,
    hist_pos: usize,
    hist_len: usize,
    /// Sort scratch for the running median.
    sorted: Vec<f32>,
    lock_run: usize,
}

impl PitchDetector {
    pub fn new(decimated_rate: f32) -> Self {
        let tau_min = (decimated_rate / F0_MAX_HZ).floor().max(2.0) as usize;
        let tau_max = (decimated_rate / F0_MIN_HZ).ceil() as usize;
        Self {
            rate: decimated_rate,
            tau_min,
            tau_max,
            diff: vec![0.0; tau_max + 1],
            cmnd: vec![1.0; tau_max + 1],
            hist: vec![0.0; REF_LEN],
            hist_pos: 0,
            hist_len: 0,
            sorted: Vec::with_capacity(REF_LEN),
            lock_run: 0,
        }
    }

    /// Samples one frame must hold for the search range to be reachable.
    pub fn min_frame_len(&self) -> usize {
        self.tau_max * 2
    }

    pub fn reset(&mut self) {
        self.hist_len = 0;
        self.hist_pos = 0;
        self.lock_run = 0;
    }

    /// Estimate the frame's fundamental. `frame` is decimated, oldest first.
    pub fn estimate(&mut self, frame: &[f32]) -> Option<Pitch> {
        if frame.len() <= self.tau_max + self.tau_min {
            return None;
        }
        self.difference(frame);
        self.normalise();
        let tau = self.pick_lag()?;
        if self.above_ceiling(tau) {
            return None;
        }
        let tau = self.in_frame_octave_check(tau);
        let (tau_f, d) = self.refine(tau);
        let hz = self.rate / tau_f;
        if !(F0_MIN_HZ..=F0_MAX_HZ).contains(&hz) {
            return None;
        }
        let confidence = (1.0 - d).clamp(0.0, 1.0);
        if d > VOICED_MAX_D {
            return None;
        }
        let (hz, confidence) = self.cross_frame_octave_check(tau, hz, confidence);
        self.remember(hz);
        Some(Pitch { hz, confidence })
    }

    /// YIN step 2: `d(tau) = sum_j (x[j] - x[j+tau])^2`.
    fn difference(&mut self, frame: &[f32]) {
        let w = frame.len() - self.tau_max;
        self.diff[0] = 0.0;
        for tau in 1..=self.tau_max {
            let mut acc = 0.0f32;
            for j in 0..w {
                let d = frame[j] - frame[j + tau];
                acc = d.mul_add(d, acc);
            }
            self.diff[tau] = acc;
        }
    }

    /// YIN step 3: divide by the running mean of `d` up to this lag. This is
    /// what removes the "shorter lag always looks better" bias that makes bare
    /// autocorrelation pick the second harmonic.
    fn normalise(&mut self) {
        self.cmnd[0] = 1.0;
        let mut running = 0.0f32;
        for tau in 1..=self.tau_max {
            running += self.diff[tau];
            self.cmnd[tau] = if running > f32::MIN_POSITIVE {
                self.diff[tau] * tau as f32 / running
            } else {
                1.0
            };
        }
    }

    /// YIN step 4: the first lag under the threshold, descended to its local
    /// minimum. Falls back to the global minimum so a breathy frame still
    /// produces a candidate for the voicing test to reject.
    fn pick_lag(&self) -> Option<usize> {
        if self.tau_min >= self.tau_max {
            return None;
        }
        let mut tau = self.tau_min;
        while tau <= self.tau_max {
            if self.cmnd[tau] < YIN_THRESHOLD {
                return Some(self.descend(tau));
            }
            tau += 1;
        }
        let mut best = self.tau_min;
        for tau in self.tau_min..=self.tau_max {
            if self.cmnd[tau] < self.cmnd[best] {
                best = tau;
            }
        }
        Some(best)
    }

    /// Walk downhill from `tau` to the bottom of its dip.
    fn descend(&self, mut tau: usize) -> usize {
        while tau < self.tau_max && self.cmnd[tau + 1] < self.cmnd[tau] {
            tau += 1;
        }
        tau
    }

    /// Whether the real period is shorter than anything the scan can see.
    ///
    /// [`Self::pick_lag`] scans upward from `tau_min`, so a signal above the
    /// ceiling has its first reachable dip at a *multiple* of its true period
    /// and reads as an exact sub-multiple — silently, at full confidence.
    /// The difference function is computed from lag 1, so the evidence is
    /// already in hand: a dip at half the chosen lag, below the scan floor,
    /// means the scan started too late. Returning nothing is the honest
    /// answer; halving the pitch is not.
    ///
    /// Only consulted below `tau_min`. At or above it the ordinary first-dip
    /// rule has already had its say, and overriding it there would undo the
    /// deliberate preference for the lower octave that
    /// [`Self::in_frame_octave_check`] exists to express.
    fn above_ceiling(&self, tau: usize) -> bool {
        let half = tau / 2;
        half >= 1 && half < self.tau_min && self.cmnd[half] < YIN_THRESHOLD
    }

    /// The in-frame half of the octave guard. See the module docs for why the
    /// comparison is relative and why [`OCTAVE_EPS`] is there.
    fn in_frame_octave_check(&self, tau: usize) -> usize {
        let sub = tau * 2;
        if sub > self.tau_max {
            return tau;
        }
        let sub = self.descend(sub);
        if self.cmnd[sub] + OCTAVE_EPS < OCTAVE_REL * (self.cmnd[tau] + OCTAVE_EPS) {
            sub
        } else {
            tau
        }
    }

    /// YIN step 5: parabolic interpolation through the minimum's neighbours.
    /// Returns the fractional lag and the interpolated difference there.
    fn refine(&self, tau: usize) -> (f32, f32) {
        if tau == 0 || tau + 1 > self.tau_max {
            return (tau as f32, self.cmnd[tau]);
        }
        let (a, b, c) = (self.cmnd[tau - 1], self.cmnd[tau], self.cmnd[tau + 1]);
        // Vertex of the parabola through (-1, a), (0, b), (+1, c). The
        // denominator is positive at a minimum; a half-lag at 12 kHz is a
        // third of a semitone at the top of the range, so getting this sign
        // wrong is worth a test of its own.
        let denom = (a + c) - 2.0 * b;
        if denom <= f32::MIN_POSITIVE {
            return (tau as f32, b);
        }
        let shift = (0.5 * (a - c) / denom).clamp(-1.0, 1.0);
        let refined = 0.25f32.mul_add(-((a - c) * shift), b);
        (tau as f32 + shift, refined.max(0.0))
    }

    /// The cross-frame half of the octave guard.
    fn cross_frame_octave_check(&mut self, tau: usize, hz: f32, confidence: f32) -> (f32, f32) {
        let Some(reference) = self.reference() else {
            self.lock_run = 0;
            return (hz, confidence);
        };
        let st = 12.0 * (hz / reference).log2();
        let alt = if (st - 12.0).abs() <= OCTAVE_TOL_ST {
            // Candidate looks an octave high: test twice the period.
            Some(tau * 2)
        } else if (st + 12.0).abs() <= OCTAVE_TOL_ST {
            // ... an octave low: test half of it.
            Some(tau / 2)
        } else {
            None
        };
        let Some(alt) = alt else {
            self.lock_run = 0;
            return (hz, confidence);
        };
        if alt < self.tau_min || alt > self.tau_max {
            self.lock_run = 0;
            return (hz, confidence);
        }
        let (alt_tau, alt_d) = self.refine(self.descend(alt));
        let alt_conf = (1.0 - alt_d).clamp(0.0, 1.0);
        if confidence > alt_conf + OCTAVE_CONF_MARGIN {
            self.lock_run = 0;
            return (hz, confidence);
        }
        self.lock_run += 1;
        if self.lock_run > OCTAVE_LOCK_MAX {
            // Sustained: the speaker moved, not the tracker. Drop the
            // reference so the next frames re-establish one there.
            self.hist_len = 0;
            self.hist_pos = 0;
            self.lock_run = 0;
            return (hz, confidence);
        }
        let alt_hz = self.rate / alt_tau;
        if (F0_MIN_HZ..=F0_MAX_HZ).contains(&alt_hz) {
            (alt_hz, alt_conf)
        } else {
            (hz, confidence)
        }
    }

    /// Median of the recent accepted values, once there are enough.
    fn reference(&mut self) -> Option<f32> {
        if self.hist_len < REF_MIN {
            return None;
        }
        self.sorted.clear();
        self.sorted.extend_from_slice(&self.hist[..self.hist_len]);
        self.sorted
            .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(self.sorted[self.sorted.len() / 2])
    }

    fn remember(&mut self, hz: f32) {
        self.hist[self.hist_pos] = hz;
        self.hist_pos = (self.hist_pos + 1) % REF_LEN;
        self.hist_len = (self.hist_len + 1).min(REF_LEN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: f32, hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f32::consts::TAU * hz * i as f32 / rate).sin())
            .collect()
    }

    /// Decimate `signal`, then run the detector on 60 ms frames every 20 ms,
    /// which is the rate [`super::Analyzer`] uses. Returns the voiced frames'
    /// frequencies and the total number of frames tried.
    fn run(rate: f32, signal: &[f32]) -> (Vec<f32>, usize) {
        let mut decim = Decimator::new(rate);
        let drate = decim.rate(rate);
        let mut det = PitchDetector::new(drate);
        let frame_len = (PITCH_FRAME_S * drate) as usize;
        let hop = (0.020 * drate) as usize;
        let mut buf: Vec<f32> = Vec::new();
        for &x in signal {
            if let Some(d) = decim.push(x) {
                buf.push(d);
            }
        }
        let mut out = Vec::new();
        let mut frames = 0;
        let mut start = 0;
        while start + frame_len <= buf.len() {
            frames += 1;
            if let Some(p) = det.estimate(&buf[start..start + frame_len]) {
                out.push(p.hz);
            }
            start += hop;
        }
        (out, frames)
    }

    fn median(mut v: Vec<f32>) -> f32 {
        v.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }

    #[test]
    fn a_pure_sine_reads_its_own_frequency() {
        for &hz in &[90.0f32, 140.0, 220.0, 330.0] {
            let sig = sine(48_000.0, hz, 24_000);
            let (got, _) = run(48_000.0, &sig);
            assert!(!got.is_empty(), "{hz} Hz produced no voiced frames");
            let read = median(got);
            assert!((read - hz).abs() < hz * 0.01, "{hz} Hz read as {read} Hz");
        }
    }

    #[test]
    fn a_pitched_up_voice_reads_its_own_pitch_not_half_of_it() {
        // The after-effects tap's whole job. At the old 400 Hz ceiling the
        // scan could not reach these periods, so it locked onto the first
        // multiple it could see and reported exactly half at full confidence:
        // a +12 semitone preset on a 250 Hz voice read as 250 Hz, i.e. "the
        // effect did nothing", which is the one thing that half of the card
        // exists to show.
        for &hz in &[450.0f32, 500.0, 600.0, 700.0, 880.0] {
            let sig = sine(48_000.0, hz, 24_000);
            let (got, _) = run(48_000.0, &sig);
            assert!(!got.is_empty(), "{hz} Hz produced no voiced frames");
            let read = median(got);
            assert!(
                (read - hz).abs() < hz * 0.02,
                "{hz} Hz read as {read} Hz (half would be {})",
                hz / 2.0
            );
        }
    }

    #[test]
    fn a_pitch_above_the_ceiling_reads_nothing_rather_than_a_sub_multiple() {
        // Past the ceiling the honest answer is "cannot read it". Reporting
        // half would be indistinguishable, to the user, from a real reading.
        for &hz in &[1_400.0f32, 2_000.0] {
            let sig = sine(48_000.0, hz, 24_000);
            let (got, _) = run(48_000.0, &sig);
            let halves = got
                .iter()
                .filter(|&&read| (read - hz / 2.0).abs() < hz * 0.05)
                .count();
            assert_eq!(
                halves, 0,
                "{hz} Hz reported as its own half in {halves} frames"
            );
        }
    }

    #[test]
    fn interpolation_beats_the_lag_grid_it_sits_on() {
        // 12 kHz quantises 330 Hz to lags 36 (333.3 Hz) and 37 (324.3 Hz),
        // i.e. 0.48 semitones apart. Landing inside 0.1 st of the true value
        // is only possible if the parabolic step is right, sign included.
        let sig = sine(48_000.0, 330.0, 24_000);
        let (got, _) = run(48_000.0, &sig);
        let read = median(got);
        let err_st = 12.0 * (read / 330.0).log2();
        assert!(
            err_st.abs() < 0.1,
            "330 Hz read as {read} Hz, {err_st} semitones out"
        );
    }

    #[test]
    fn a_dominant_second_harmonic_is_not_read_an_octave_high() {
        // Fundamental present but weaker than its own second harmonic — the
        // case bare autocorrelation gets wrong every time.
        let f0 = 120.0f32;
        let rate = 48_000.0f32;
        let sig: Vec<f32> = (0..24_000)
            .map(|i| {
                let t = std::f32::consts::TAU * f0 * i as f32 / rate;
                0.5f32.mul_add(t.sin(), (2.0 * t).sin())
                    + 0.5 * (3.0 * t).sin()
                    + 0.25 * (4.0 * t).sin()
            })
            .collect();
        let (got, _) = run(rate, &sig);
        assert!(!got.is_empty());
        for hz in got {
            assert!(
                (hz - f0).abs() < 6.0,
                "read {hz} Hz for a {f0} Hz buzz (an octave would be {} or {})",
                f0 * 2.0,
                f0 / 2.0
            );
        }
    }

    #[test]
    fn a_thinned_fundamental_is_still_not_read_an_octave_high() {
        // What a gate plus a denoiser plus a gaming headset's low-end roll-off
        // leaves: the fundamental at a seventh of the second harmonic, and
        // the odd harmonics that would otherwise break the half-period
        // symmetry gone with it. The waveform now nearly repeats at half its
        // own period, which is exactly the shape that reads 240 Hz. The
        // in-frame guard is the only thing standing between this and that
        // readout; without it the same signal doubles at any fundamental
        // below about a third of the second harmonic.
        let f0 = 120.0f32;
        let rate = 48_000.0f32;
        let sig: Vec<f32> = (0..24_000)
            .map(|i| {
                let t = std::f32::consts::TAU * f0 * i as f32 / rate;
                0.15f32.mul_add(t.sin(), (2.0 * t).sin()) + 0.5 * (4.0 * t).sin()
            })
            .collect();
        let (got, _) = run(rate, &sig);
        assert!(!got.is_empty());
        let read = median(got);
        assert!(
            (read - f0).abs() < 8.0,
            "thinned fundamental read as {read} Hz, not {f0} Hz"
        );
    }

    #[test]
    fn an_intermittently_missing_fundamental_does_not_flip_the_octave() {
        // The fundamental drops out completely for a quarter second at a
        // time, the way a gate or a denoiser chews the low end in and out.
        // Inside one of those stretches the waveform genuinely does repeat at
        // half its period, so no single frame can tell -- only the running
        // median of the frames either side of it can. An intermittently
        // doubling pitch readout is the failure users actually see, and it is
        // worse than no readout at all.
        let rate = 48_000.0f32;
        let f0 = 120.0f32;
        let sig: Vec<f32> = (0..48_000)
            .map(|i| {
                let t = std::f32::consts::TAU * f0 * i as f32 / rate;
                let thin = (i as f32 / rate) % 1.0 > 0.75;
                let a1: f32 = if thin { 0.0 } else { 0.5 };
                a1.mul_add(t.sin(), (2.0 * t).sin()) + 0.5 * (4.0 * t).sin()
            })
            .collect();
        let (got, _) = run(rate, &sig);
        assert!(got.len() > 30, "only {} voiced frames", got.len());
        let doubled = got.iter().filter(|hz| **hz > 180.0).count();
        assert!(
            doubled * 12 < got.len(),
            "{doubled} of {} frames read an octave high",
            got.len()
        );
    }

    #[test]
    fn white_noise_is_unvoiced() {
        let rate = 48_000.0f32;
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let sig: Vec<f32> = (0..48_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 40) as f32 / 8_388_608.0 - 1.0
            })
            .collect();
        let (got, frames) = run(rate, &sig);
        assert!(frames > 20, "the harness only tried {frames} frames");
        assert!(
            got.len() * 10 < frames,
            "white noise produced {} voiced frames out of {frames}",
            got.len()
        );
    }

    #[test]
    fn the_search_range_covers_adult_speech_at_every_rate() {
        for &rate in &[44_100.0f32, 48_000.0, 96_000.0] {
            let decim = Decimator::new(rate);
            let det = PitchDetector::new(decim.rate(rate));
            let lo = decim.rate(rate) / det.tau_max as f32;
            let hi = decim.rate(rate) / det.tau_min as f32;
            assert!(lo <= F0_MIN_HZ + 1.0, "{rate}: bottom is {lo} Hz");
            assert!(hi >= F0_MAX_HZ - 1.0, "{rate}: top is {hi} Hz");
        }
    }
}
