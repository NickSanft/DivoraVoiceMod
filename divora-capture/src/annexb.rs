//! H.264 Annex B byte streams: NAL units, keyframes, parameter sets (v1.54.0).
//!
//! The hardware encoder hands out each frame as an Annex B access unit: NAL
//! units, each after a start code. The replay ring and the muxers need three
//! facts from one, and take them from the bytes, never from the encoder's own
//! flags:
//!
//! - **Is it a keyframe?** A clip or a take may only start on an IDR slice
//!   (NAL type 5). The encoder's `CleanPoint` agreed on every frame measured,
//!   but it is an attribute a driver sets, and the bytes are what a player
//!   decodes.
//! - **Which parameter sets does it carry?** An IDR repeats the SPS and PPS
//!   in-band. An MP4 holds one copy of them (`avcC`), so a file must open
//!   with the ones its first IDR was encoded against.
//! - **Have they changed?** A later IDR whose sets differ cannot go into the
//!   same file.
//!
//! A start code is `00 00 01`, or `00 00 00 01` before the first unit and
//! wherever an encoder likes; both appear in one access unit. Inside a unit
//! the encoder escapes every `00 00 0x` as `00 00 03 0x` (emulation
//! prevention), so `00 00 01` is never part of a unit and scanning for it is
//! enough. A unit never ends in a zero byte, so zeros before a start code
//! belong to the code (or are padding), never to the unit before it.

use std::sync::Arc;

/// `nal_unit_type` of a coded slice of an IDR picture: a keyframe.
pub const NAL_IDR: u8 = 5;
/// `nal_unit_type` of a sequence parameter set.
pub const NAL_SPS: u8 = 7;
/// `nal_unit_type` of a picture parameter set.
pub const NAL_PPS: u8 = 8;

/// The start code [`parameter_sets`] writes before each set: the four-byte
/// form, which is what both measured encoders put in their media type's
/// sequence header.
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// One NAL unit of an access unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nal<'a> {
    /// `nal_unit_type`: the low five bits of the header byte. The high bits
    /// are `nal_ref_idc`, which differs between encoders for the same type.
    pub kind: u8,
    /// The unit, header byte first, without its start code.
    pub bytes: &'a [u8],
}

/// The NAL units of `au`, in order. Bytes before the first start code are
/// not a unit and are skipped.
#[must_use]
pub fn nal_units(au: &[u8]) -> NalUnits<'_> {
    let pos = find_start_code(au, 0).map_or(au.len(), |i| i + 3);
    NalUnits { data: au, pos }
}

/// Iterator over an access unit's NAL units; see [`nal_units`].
#[derive(Debug, Clone)]
pub struct NalUnits<'a> {
    data: &'a [u8],
    /// Just after the start code of the next unit, or the end.
    pos: usize,
}

impl<'a> Iterator for NalUnits<'a> {
    type Item = Nal<'a>;

    fn next(&mut self) -> Option<Nal<'a>> {
        while self.pos < self.data.len() {
            let from = self.pos;
            let (to, next) = match find_start_code(self.data, from) {
                Some(i) => (i, i + 3),
                None => (self.data.len(), self.data.len()),
            };
            self.pos = next;
            let unit = trim_trailing_zeros(&self.data[from..to]);
            // Two start codes back to back hold nothing.
            if let Some(&header) = unit.first() {
                return Some(Nal {
                    kind: header & 0x1F,
                    bytes: unit,
                });
            }
        }
        None
    }
}

/// True when `au` holds an IDR slice. Only type 5 counts: an access unit
/// that repeats the SPS and PPS (types 7 and 8) or carries SEI (6) is still
/// a P-frame unless its picture is an IDR.
#[must_use]
pub fn has_idr(au: &[u8]) -> bool {
    nal_units(au).any(|n| n.kind == NAL_IDR)
}

/// True when `au` carries both an SPS and a PPS in-band.
#[must_use]
pub fn has_parameter_sets(au: &[u8]) -> bool {
    nal_units(au).any(|n| n.kind == NAL_SPS) && nal_units(au).any(|n| n.kind == NAL_PPS)
}

/// The SPS and PPS units of `au`, in the order they came, each after a
/// four-byte start code: the form Media Foundation's sink takes as a
/// sequence header. `None` unless `au` carries both: a header with one but
/// not the other cannot open a file.
#[must_use]
pub fn parameter_sets(au: &[u8]) -> Option<Arc<[u8]>> {
    if !has_parameter_sets(au) {
        return None;
    }
    let mut out = Vec::new();
    for n in nal_units(au).filter(|n| n.kind == NAL_SPS || n.kind == NAL_PPS) {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(n.bytes);
    }
    Some(out.into())
}

/// True when `a` and `b` carry the same SPS units and the same PPS units.
/// Start codes are not compared: an encoder may write a set after a three-
/// byte code in-band and a four-byte one in its sequence header, and that
/// is the same set. Anything else in either (AUD, SEI, slices) is ignored.
#[must_use]
pub fn same_parameter_sets(a: &[u8], b: &[u8]) -> bool {
    let sets = |au, kind| {
        nal_units(au)
            .filter(move |n: &Nal| n.kind == kind)
            .map(|n| n.bytes)
    };
    sets(a, NAL_SPS).eq(sets(b, NAL_SPS)) && sets(a, NAL_PPS).eq(sets(b, NAL_PPS))
}

/// Where the next `00 00 01` at or after `from` starts.
fn find_start_code(data: &[u8], from: usize) -> Option<usize> {
    data.get(from..)?
        .windows(3)
        .position(|w| w == [0, 0, 1])
        .map(|p| from + p)
}

/// `unit` without the zeros at its end: the first byte of a four-byte start
/// code, or `trailing_zero_8bits` padding. A unit's own last byte is never
/// zero (H.264 7.4.1), so nothing of it is lost.
fn trim_trailing_zeros(unit: &[u8]) -> &[u8] {
    let len = unit.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &unit[..len]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An access unit from `(start code length, unit)` pairs.
    fn au(units: &[(usize, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(code, unit) in units {
            out.extend(std::iter::repeat_n(0u8, code - 1));
            out.push(1);
            out.extend_from_slice(unit);
        }
        out
    }

    fn kinds(au: &[u8]) -> Vec<u8> {
        nal_units(au).map(|n| n.kind).collect()
    }

    const AUD: &[u8] = &[0x09, 0xF0];
    const SPS: &[u8] = &[0x67, 0x64, 0x00, 0x28, 0xAC, 0x2B, 0x40];
    const PPS: &[u8] = &[0x68, 0xEE, 0x3C, 0x80];
    const SEI: &[u8] = &[0x06, 0x05, 0x11, 0x80];
    const IDR: &[u8] = &[0x65, 0x88, 0x84, 0x00, 0x21];
    const P: &[u8] = &[0x41, 0x9A, 0x02];

    #[test]
    fn nal_units_reads_three_and_four_byte_codes() {
        // NVIDIA's IDR layout, with the codes mixed: AUD, SPS, PPS, IDR.
        let a = au(&[(4, AUD), (3, SPS), (4, PPS), (3, IDR)]);
        let units: Vec<Nal> = nal_units(&a).collect();
        assert_eq!(
            units,
            vec![
                Nal {
                    kind: 9,
                    bytes: AUD
                },
                Nal {
                    kind: 7,
                    bytes: SPS
                },
                Nal {
                    kind: 8,
                    bytes: PPS
                },
                Nal {
                    kind: 5,
                    bytes: IDR
                },
            ]
        );
        // All three-byte, and all four-byte, read the same.
        assert_eq!(
            kinds(&au(&[(3, AUD), (3, SPS), (3, PPS), (3, IDR)])),
            [9, 7, 8, 5]
        );
        assert_eq!(
            kinds(&au(&[(4, AUD), (4, SPS), (4, PPS), (4, IDR)])),
            [9, 7, 8, 5]
        );
        // Leading bytes before the first code, and an empty input, are no unit.
        let mut lead = vec![0xFF, 0x00];
        lead.extend(au(&[(3, P)]));
        assert_eq!(kinds(&lead), [1]);
        assert_eq!(nal_units(&[]).count(), 0);
        assert_eq!(nal_units(&[0, 0, 1]).count(), 0);
    }

    #[test]
    fn idr_is_nal_type_5_only() {
        // Everything an access unit carries besides an IDR slice.
        for unit in [AUD, SPS, PPS, SEI, P] {
            assert!(!has_idr(&au(&[(4, unit)])), "type {}", unit[0] & 0x1F);
        }
        assert!(!has_idr(&au(&[
            (4, AUD),
            (4, SPS),
            (4, PPS),
            (4, SEI),
            (4, P)
        ])));
        // nal_ref_idc 3 and 1 over type 5: both IDRs.
        assert!(has_idr(&au(&[(4, &[0x65, 0x88])])));
        assert!(has_idr(&au(&[(4, &[0x25, 0x88])])));
        // The inbox encoder's layout: AUD, SPS, PPS, SEI, SEI, IDR.
        assert!(has_idr(&au(&[
            (4, AUD),
            (4, SPS),
            (4, PPS),
            (4, SEI),
            (4, SEI),
            (4, IDR)
        ])));
    }

    #[test]
    fn parameter_sets_compare_without_start_codes() {
        let four = au(&[(4, AUD), (4, SPS), (4, PPS), (4, IDR)]);
        let three = au(&[(3, AUD), (3, SPS), (3, PPS), (3, IDR)]);
        assert!(same_parameter_sets(&four, &three));
        // The extracted header, as a sink wants it, matches the unit it came from.
        let header = parameter_sets(&three).expect("SPS and PPS in-band");
        assert_eq!(&header[..], &au(&[(4, SPS), (4, PPS)])[..]);
        assert!(same_parameter_sets(&header, &three));

        // One byte changed in the SPS, then in the PPS: different.
        let mut sps = SPS.to_vec();
        sps[3] = 0x29;
        assert!(!same_parameter_sets(
            &four,
            &au(&[(4, &sps), (4, PPS), (4, IDR)])
        ));
        let mut pps = PPS.to_vec();
        pps[2] ^= 1;
        assert!(!same_parameter_sets(
            &four,
            &au(&[(4, SPS), (4, &pps), (4, IDR)])
        ));

        // Only one of the two is no header.
        assert_eq!(parameter_sets(&au(&[(4, SPS), (4, IDR)])), None);
        assert_eq!(parameter_sets(&au(&[(4, P)])), None);
        assert!(!has_parameter_sets(&au(&[(4, PPS), (4, IDR)])));
    }

    #[test]
    fn emulation_prevention_is_not_a_start_code() {
        // 00 00 03 01 inside a slice is an escaped 00 00 01, and 00 00 03 00
        // an escaped 00 00 00: still one unit.
        let slice: &[u8] = &[
            0x65, 0xAA, 0x00, 0x00, 0x03, 0x01, 0xBB, 0x00, 0x00, 0x03, 0x00, 0xCC,
        ];
        let a = au(&[(4, slice), (3, P)]);
        let units: Vec<Nal> = nal_units(&a).collect();
        assert_eq!(
            units,
            vec![
                Nal {
                    kind: 5,
                    bytes: slice
                },
                Nal { kind: 1, bytes: P }
            ]
        );
        assert!(has_idr(&a));
    }
}
