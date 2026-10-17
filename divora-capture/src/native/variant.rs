//! `VARIANT`s for the encoder's `ICodecAPI` settings (v1.54.0).
//!
//! windows 0.61's `VARIANT` is the raw union with no `Drop`
//! (Win32/System/Variant/mod.rs), so the scalar values built here own
//! nothing and need no `VariantClear`.

use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4, VT_UI8};

/// A `VT_UI4` holding `v`.
pub fn variant_u32(v: u32) -> VARIANT {
    let mut var = VARIANT::default();
    // SAFETY: plain-data writes into a zeroed VARIANT: the tag, then the
    // union member the tag names.
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = v;
    }
    var
}

/// A `VT_BOOL` holding `b` (`VARIANT_TRUE` is −1, not 1).
pub fn variant_bool(b: bool) -> VARIANT {
    let mut var = VARIANT::default();
    // SAFETY: as above.
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal = if b { VARIANT_TRUE } else { VARIANT_FALSE };
    }
    var
}

/// The number a `VT_UI4`, `VT_UI8` or `VT_BOOL` holds; `None` for anything
/// else — an empty value is what an encoder that won't say gives back.
pub fn variant_as_u32(v: &VARIANT) -> Option<u32> {
    // SAFETY: the tag is read first and only the member it names is read.
    unsafe {
        let inner = &*v.Anonymous.Anonymous;
        match inner.vt {
            VT_UI4 => Some(inner.Anonymous.ulVal),
            VT_UI8 => u32::try_from(inner.Anonymous.ullVal).ok(),
            VT_BOOL => Some(u32::from(inner.Anonymous.boolVal.0 != 0)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{variant_as_u32, variant_bool, variant_u32};

    #[test]
    fn values_round_trip() {
        assert_eq!(variant_as_u32(&variant_u32(60)), Some(60));
        assert_eq!(variant_as_u32(&variant_bool(true)), Some(1));
        assert_eq!(variant_as_u32(&variant_bool(false)), Some(0));
        assert_eq!(
            variant_as_u32(&windows::Win32::System::Variant::VARIANT::default()),
            None
        );
    }
}
