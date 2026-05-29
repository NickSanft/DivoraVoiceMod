//! Audio device enumeration via cpal.
//!
//! Returns `DeviceInfo` snapshots that the UI can show in pickers.
//! Device identity is stable by *name*; the engine looks devices up by
//! name when starting a stream.

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

/// A snapshot of one audio device, returned to the UI for picker rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// Display name (and the key used to select the device later).
    pub name: String,
    /// True if this is the host's default device for its direction.
    pub is_default: bool,
    /// Default sample rate in Hz, as reported by cpal.
    pub default_sample_rate: u32,
    /// Default channel count, as reported by cpal.
    pub channels: u16,
}

/// List input devices reported by the default host.
#[must_use]
pub fn list_input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    let mut out = Vec::new();
    let Ok(devices) = host.input_devices() else {
        return out;
    };
    for device in devices {
        let Ok(name) = device.name() else { continue };
        let (default_sample_rate, channels) = match device.default_input_config() {
            Ok(cfg) => (cfg.sample_rate().0, cfg.channels()),
            Err(_) => continue,
        };
        let is_default = default_name.as_deref() == Some(name.as_str());
        out.push(DeviceInfo {
            name,
            is_default,
            default_sample_rate,
            channels,
        });
    }
    out
}

/// List output devices reported by the default host.
#[must_use]
pub fn list_output_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    let mut out = Vec::new();
    let Ok(devices) = host.output_devices() else {
        return out;
    };
    for device in devices {
        let Ok(name) = device.name() else { continue };
        let (default_sample_rate, channels) = match device.default_output_config() {
            Ok(cfg) => (cfg.sample_rate().0, cfg.channels()),
            Err(_) => continue,
        };
        let is_default = default_name.as_deref() == Some(name.as_str());
        out.push(DeviceInfo {
            name,
            is_default,
            default_sample_rate,
            channels,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{list_input_devices, list_output_devices};

    /// `cpal`'s WASAPI backend on the headless `windows-2022` runner
    /// intermittently `STATUS_ACCESS_VIOLATION`s when device
    /// enumeration is hit from a unit test. Real hardware is fine,
    /// and these calls are exercised in the manual / e2e paths anyway.
    /// CI sets `CI=true`; we skip just the bare enumeration there.
    fn skip_on_ci() -> bool {
        std::env::var("CI").is_ok()
    }

    // CI runners have no real audio hardware; we don't assert anything about
    // device count, only that the functions don't panic and produce
    // well-formed output (no empty names, plausible channel counts).
    #[test]
    fn lists_inputs_without_panicking() {
        if skip_on_ci() {
            return;
        }
        let devices = list_input_devices();
        for d in &devices {
            assert!(!d.name.is_empty());
            assert!(d.channels >= 1);
        }
    }

    #[test]
    fn lists_outputs_without_panicking() {
        if skip_on_ci() {
            return;
        }
        let devices = list_output_devices();
        for d in &devices {
            assert!(!d.name.is_empty());
            assert!(d.channels >= 1);
        }
    }
}
