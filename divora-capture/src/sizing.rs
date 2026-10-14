//! Output size and bitrate for a recording. Pure, so it is tested without a
//! monitor or an encoder.

/// The largest size recorded by default ("match the monitor, up to 1440p").
pub const DEFAULT_MAX: (u32, u32) = (2560, 1440);

/// Bits per pixel per frame for the default bitrate. 0.13 gives ~8 Mbps at
/// 1080p30 and ~14 Mbps at 1440p30, in line with common upload guidance for
/// screen content in H.264.
const BITS_PER_PIXEL: f64 = 0.13;

/// The encoded size for a `src` monitor: the monitor's own size when it fits
/// in `max`, otherwise scaled down to fit, keeping its aspect ratio. Both
/// sides are rounded down to even numbers, which NV12 and H.264 require.
/// Never smaller than 2×2.
pub fn output_size(src: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (src.0.max(2), src.1.max(2));
    let (mw, mh) = (max.0.max(2), max.1.max(2));
    let (w, h) = if sw <= mw && sh <= mh {
        (sw, sh)
    } else {
        // Scale by the tighter of the two limits.
        let k = (f64::from(mw) / f64::from(sw)).min(f64::from(mh) / f64::from(sh));
        let w = (f64::from(sw) * k).floor();
        let h = (f64::from(sh) * k).floor();
        (to_u32(w), to_u32(h))
    };
    (even(w), even(h))
}

/// Where a `src` picture goes inside a `frame` of a fixed size: as large as
/// it fits keeping its aspect ratio, centred, with even sides and offsets
/// (NV12 chroma is per 2×2 block). `(x, y, width, height)`.
///
/// A take's encoded size is fixed when it starts, so a monitor that changes
/// mode mid-take — a game switching a 21:9 screen to 16:9 — is letterboxed
/// into it rather than stretched.
pub fn fit_rect(src: (u32, u32), frame: (u32, u32)) -> (u32, u32, u32, u32) {
    let (fw, fh) = (frame.0.max(2), frame.1.max(2));
    let (sw, sh) = (src.0.max(1), src.1.max(1));
    let k = (f64::from(fw) / f64::from(sw)).min(f64::from(fh) / f64::from(sh));
    let w = even(to_u32((f64::from(sw) * k).round()).min(fw));
    let h = even(to_u32((f64::from(sh) * k).round()).min(fh));
    // The take's size was itself rounded to even sides from this shape: a
    // percent either way is that rounding, not a different screen, and is
    // filled rather than given bars a pixel or two wide.
    if fw - w <= fw / 100 && fh - h <= fh / 100 {
        return (0, 0, fw, fh);
    }
    (((fw - w) / 2) & !1, ((fh - h) / 2) & !1, w, h)
}

/// Default average bitrate in bits per second for `size` at `fps`.
pub fn default_bitrate(size: (u32, u32), fps: u32) -> u32 {
    let bits = f64::from(size.0) * f64::from(size.1) * f64::from(fps.max(1)) * BITS_PER_PIXEL;
    to_u32(bits.round()).max(500_000)
}

fn even(n: u32) -> u32 {
    (n & !1).max(2)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_u32(x: f64) -> u32 {
    if x.is_nan() || x <= 0.0 {
        0
    } else if x >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        x as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_aspect_fills_the_frame() {
        assert_eq!(fit_rect((3440, 1440), (2560, 1070)), (0, 0, 2560, 1070));
        assert_eq!(fit_rect((1920, 1080), (1920, 1080)), (0, 0, 1920, 1080));
    }

    #[test]
    fn another_aspect_is_letterboxed_and_centred_on_even_lines() {
        // A 21:9 take whose screen switches to 16:9: pillarboxed.
        let (x, y, w, h) = fit_rect((1920, 1080), (2560, 1070));
        assert_eq!((y, h), (0, 1070));
        assert_eq!(w, 1902);
        assert_eq!(x, 328);
        // A 16:9 take whose screen switches to 21:9: letterboxed.
        let (x, y, w, h) = fit_rect((3440, 1440), (1920, 1080));
        assert_eq!((x, w), (0, 1920));
        assert_eq!(h, 804);
        assert_eq!(y, 138);
        for r in [
            fit_rect((1366, 768), (2560, 1070)),
            fit_rect((1000, 999), (640, 480)),
        ] {
            assert!(
                r.0 % 2 == 0 && r.1 % 2 == 0 && r.2 % 2 == 0 && r.3 % 2 == 0,
                "{r:?}"
            );
        }
    }

    #[test]
    fn a_monitor_that_fits_is_recorded_at_its_own_size() {
        assert_eq!(output_size((1920, 1080), DEFAULT_MAX), (1920, 1080));
        assert_eq!(output_size((2560, 1440), DEFAULT_MAX), (2560, 1440));
        assert_eq!(output_size((1366, 768), DEFAULT_MAX), (1366, 768));
    }

    #[test]
    fn a_bigger_monitor_is_scaled_down_keeping_its_aspect() {
        assert_eq!(output_size((3840, 2160), DEFAULT_MAX), (2560, 1440));
        // Ultrawide 3440×1440 fits in height but not width: width wins.
        assert_eq!(output_size((3440, 1440), DEFAULT_MAX), (2560, 1070));
        // Portrait 1440×2560 is limited by height.
        assert_eq!(output_size((1440, 2560), DEFAULT_MAX), (810, 1440));
    }

    #[test]
    fn sizes_are_even() {
        let (w, h) = output_size((1365, 767), DEFAULT_MAX);
        assert_eq!((w % 2, h % 2), (0, 0));
        assert_eq!((w, h), (1364, 766));
        assert_eq!(output_size((0, 1), DEFAULT_MAX), (2, 2));
    }

    #[test]
    fn default_bitrate_scales_with_pixels_and_rate() {
        assert_eq!(default_bitrate((1920, 1080), 30), 8_087_040);
        assert!(default_bitrate((2560, 1440), 30) > default_bitrate((1920, 1080), 30));
        assert_eq!(
            default_bitrate((1920, 1080), 60),
            2 * default_bitrate((1920, 1080), 30)
        );
        assert_eq!(default_bitrate((2, 2), 1), 500_000, "floor");
    }
}
