//! Divora core — audio engine and DSP primitives.
//!
//! Phase 2 lands the audio engine: real-time capture from an input device,
//! optional sidetone monitoring to an output device, level metering, and
//! enumeration of available devices. DSP effects, presets, soundboard, and
//! virtual-mic routing all build on top of this in later phases.

#[cfg(test)]
mod alloc_probe;

pub mod audio;
pub mod dsp;
pub mod presets;
pub mod soundboard;
pub mod tts;

/// Counts this test binary's allocations, so unit tests can assert that a
/// realtime path allocates nothing. Per-thread, so parallel tests do not
/// contaminate each other — see [`alloc_probe`].
#[cfg(test)]
#[global_allocator]
static ALLOC_PROBE: alloc_probe::Counting = alloc_probe::Counting;

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
