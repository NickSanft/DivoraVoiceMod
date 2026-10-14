//! BGRA → NV12 on the CPU: the fallback for a machine whose Direct3D device
//! has no video processor (a basic display adapter, some virtual machines),
//! and what the encoder smoke test feeds the software encoder.
//!
//! The same colour as the GPU path: BT.709, studio range (Y 16–235, chroma
//! 16–240), so a take looks the same whichever path made it. Scaling is
//! nearest-neighbour — this is the fallback, and it has a CPU budget.

/// Bytes an NV12 frame of `size` occupies: a full-size luma plane, then
/// interleaved chroma at half size each way.
#[must_use]
pub fn nv12_len(size: (u32, u32)) -> usize {
    let (w, h) = (size.0 as usize, size.1 as usize);
    w * h + w * h.div_ceil(2)
}

/// BT.709 studio-range luma from 8-bit R, G, B, in 16.16 fixed point.
/// Coefficients 0.2126 / 0.7152 / 0.0722, scaled by 219/255. (8.8 is not
/// enough: it puts pure green a code value low.)
fn luma(r: i32, g: i32, b: i32) -> u8 {
    clamp_u8(((11_966 * r + 40_255 * g + 4_064 * b + 32_768) >> 16) + 16)
}

/// BT.709 studio-range Cb and Cr, scaled by 224/255. Each row sums to zero,
/// so grey stays exactly at 128.
fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    let cb = ((-6_598 * r - 22_187 * g + 28_785 * b + 32_768) >> 16) + 128;
    let cr = ((28_785 * r - 26_148 * g - 2_637 * b + 32_768) >> 16) + 128;
    (clamp_u8(cb), clamp_u8(cr))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped first
fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Convert `src` — BGRA rows of `src_size`, `pitch` bytes apart — into NV12
/// of `dst_size` (even sides) in `out`, scaling nearest-neighbour.
///
/// # Panics
///
/// If `src` is shorter than `pitch × height`, or `out` is not
/// [`nv12_len`]`(dst_size)` long.
pub fn bgra_to_nv12(
    src: &[u8],
    pitch: usize,
    src_size: (u32, u32),
    dst_size: (u32, u32),
    out: &mut [u8],
) {
    let (sw, sh) = (src_size.0 as usize, src_size.1 as usize);
    let (dw, dh) = (dst_size.0 as usize, dst_size.1 as usize);
    assert!(src.len() >= pitch * sh, "source too short");
    assert_eq!(
        out.len(),
        nv12_len(dst_size),
        "destination is not one NV12 frame"
    );
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        out.fill(0);
        return;
    }
    let col: Vec<usize> = (0..dw).map(|x| (x * sw / dw).min(sw - 1) * 4).collect();
    let pixel = |x: usize, y: usize| -> (i32, i32, i32) {
        let row = (y * sh / dh).min(sh - 1) * pitch;
        let p = row + col[x];
        (
            i32::from(src[p + 2]),
            i32::from(src[p + 1]),
            i32::from(src[p]),
        )
    };
    let (y_plane, uv_plane) = out.split_at_mut(dw * dh);
    for y in 0..dh {
        for x in 0..dw {
            let (r, g, b) = pixel(x, y);
            y_plane[y * dw + x] = luma(r, g, b);
        }
    }
    // One chroma pair per 2×2 block, from the block's average colour.
    for cy in 0..dh.div_ceil(2) {
        for cx in 0..dw / 2 {
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (pr, pg, pb) = pixel(cx * 2 + dx, (cy * 2 + dy).min(dh - 1));
                r += pr;
                g += pg;
                b += pb;
            }
            let (cb, cr) = chroma((r + 2) / 4, (g + 2) / 4, (b + 2) / 4);
            let at = cy * dw + cx * 2;
            uv_plane[at] = cb;
            uv_plane[at + 1] = cr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bgra_to_nv12, nv12_len};

    /// A `w×h` BGRA frame of one colour.
    fn solid(w: usize, h: usize, rgb: (u8, u8, u8)) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            v.extend_from_slice(&[rgb.2, rgb.1, rgb.0, 255]);
        }
        v
    }

    fn convert(rgb: (u8, u8, u8)) -> (u8, u8, u8) {
        let src = solid(4, 4, rgb);
        let mut out = vec![0; nv12_len((4, 4))];
        bgra_to_nv12(&src, 16, (4, 4), (4, 4), &mut out);
        (out[0], out[16], out[17])
    }

    /// The reference values for BT.709 studio range.
    #[test]
    fn primaries_land_on_the_bt709_studio_values() {
        assert_eq!(convert((255, 255, 255)), (235, 128, 128));
        assert_eq!(convert((0, 0, 0)), (16, 128, 128));
        assert_eq!(convert((128, 128, 128)), (126, 128, 128));
        assert_eq!(convert((255, 0, 0)), (63, 102, 240));
        assert_eq!(convert((0, 255, 0)), (173, 42, 26));
        assert_eq!(convert((0, 0, 255)), (32, 240, 118));
    }

    #[test]
    fn the_layout_is_nv12() {
        assert_eq!(nv12_len((1920, 1080)), 1920 * 1080 * 3 / 2);
        assert_eq!(nv12_len((2560, 1070)), 2560 * 1070 * 3 / 2);
    }

    /// Scaling halves a two-tone frame cleanly, and honours a padded pitch.
    #[test]
    fn it_scales_and_respects_the_pitch() {
        // 8×2, left half black, right half white, rows padded to 40 bytes.
        let (w, h, pitch) = (8usize, 2usize, 40usize);
        let mut src = vec![0xAA; pitch * h];
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 0 } else { 255 };
                src[y * pitch + x * 4..y * pitch + x * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let mut out = vec![0; nv12_len((4, 2))];
        bgra_to_nv12(&src, pitch, (8, 2), (4, 2), &mut out);
        assert_eq!(&out[..4], &[16, 16, 235, 235]);
        assert_eq!(&out[8..12], &[128, 128, 128, 128]);
    }
}
