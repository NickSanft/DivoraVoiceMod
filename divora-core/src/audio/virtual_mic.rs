//! VB-Cable detection. The user routes `DivoraVoice`'s modulated output
//! into `CABLE Input` and tells Discord / Zoom / OBS to listen on
//! `CABLE Output`. We surface the detection result so the UI can either
//! confirm "✓ detected" or send the user to the download page.

use super::devices::{list_input_devices, list_output_devices, DeviceInfo};

use serde::{Deserialize, Serialize};

/// Status of the virtual-microphone bridge, returned by the Tauri shell
/// to render Settings → Virtual microphone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VirtualMicStatus {
    pub detected: bool,
    /// Output device that `DivoraVoice` can route the modulated voice into.
    pub cable_input_device: Option<DeviceInfo>,
    /// Input device that Discord / Zoom / OBS should pick up the
    /// modulated voice from.
    pub cable_output_device: Option<DeviceInfo>,
    /// Where users can download the cable if it isn't installed.
    pub download_url: &'static str,
}

const VB_CABLE_DOWNLOAD: &str = "https://vb-audio.com/Cable/";

fn looks_like_cable_input(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Match strings like "CABLE Input (VB-Audio Virtual Cable)".
    lower.contains("cable input") || (lower.contains("vb-audio") && lower.contains("input"))
}

fn looks_like_cable_output(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Match strings like "CABLE Output (VB-Audio Virtual Cable)".
    lower.contains("cable output") || (lower.contains("vb-audio") && lower.contains("output"))
}

/// Look at the enumerated device lists for a VB-Cable pair.
#[must_use]
pub fn detect_virtual_mic() -> VirtualMicStatus {
    let outputs = list_output_devices();
    let inputs = list_input_devices();
    let cable_input = outputs
        .into_iter()
        .find(|d| looks_like_cable_input(&d.name));
    let cable_output = inputs
        .into_iter()
        .find(|d| looks_like_cable_output(&d.name));
    let detected = cable_input.is_some() && cable_output.is_some();
    VirtualMicStatus {
        detected,
        cable_input_device: cable_input,
        cable_output_device: cable_output,
        download_url: VB_CABLE_DOWNLOAD,
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_virtual_mic, looks_like_cable_input, looks_like_cable_output};

    #[test]
    fn cable_input_matcher_recognises_canonical_name() {
        assert!(looks_like_cable_input(
            "CABLE Input (VB-Audio Virtual Cable)"
        ));
    }

    #[test]
    fn cable_output_matcher_recognises_canonical_name() {
        assert!(looks_like_cable_output(
            "CABLE Output (VB-Audio Virtual Cable)"
        ));
    }

    #[test]
    fn cable_input_matcher_is_case_insensitive() {
        assert!(looks_like_cable_input("cable input"));
        assert!(looks_like_cable_input(
            "CABLE INPUT (VB-AUDIO VIRTUAL CABLE)"
        ));
    }

    #[test]
    fn cable_matchers_reject_unrelated_devices() {
        assert!(!looks_like_cable_input("Blue Yeti X"));
        assert!(!looks_like_cable_input("Realtek HD Audio"));
        assert!(!looks_like_cable_output("HyperX Cloud II"));
        assert!(!looks_like_cable_output("Microphone Array"));
    }

    #[test]
    fn cable_input_matcher_accepts_vb_audio_variant() {
        // VB-Audio's HiFi Cable and A-only Cable use slightly different
        // names; we hedge by also matching on "VB-Audio" + direction.
        assert!(looks_like_cable_input("VB-Audio Hi-Fi Cable Input"));
        assert!(looks_like_cable_output("VB-Audio Voicemeeter VAIO3 Output"));
    }

    #[test]
    fn detect_runs_without_panicking_on_ci_runners() {
        // CI hosts have no audio hardware; we just want the function to
        // return a status struct (likely `detected: false`).
        let status = detect_virtual_mic();
        assert_eq!(status.download_url, "https://vb-audio.com/Cable/");
        // `detected` must agree with the two device fields.
        assert_eq!(
            status.detected,
            status.cable_input_device.is_some() && status.cable_output_device.is_some()
        );
    }
}
