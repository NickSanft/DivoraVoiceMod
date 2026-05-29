//! Divora core — audio engine and DSP primitives.
//!
//! Phase 2 lands the audio engine: real-time capture from an input device,
//! optional sidetone monitoring to an output device, level metering, and
//! enumeration of available devices. DSP effects, presets, soundboard, and
//! virtual-mic routing all build on top of this in later phases.

pub mod audio;
pub mod dsp;

/// Returns the project name. Useful as a smoke test for the workspace build.
#[must_use]
pub fn project_name() -> &'static str {
    "Divora"
}

#[cfg(test)]
mod tests {
    use super::project_name;

    #[test]
    fn project_name_is_divora() {
        assert_eq!(project_name(), "Divora");
    }
}
