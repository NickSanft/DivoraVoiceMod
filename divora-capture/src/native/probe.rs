//! Can this machine record the screen?
//!
//! In order: Media Foundation loads; it starts; an H.264 encoder and an AAC
//! encoder are registered; Windows is new enough for programmatic monitor capture
//! (Windows 10 1903, `UniversalApiContract` 8). `GraphicsCaptureSession::
//! IsSupported` alone is not enough: it is true on Windows 10 1809 / LTSC
//! 2019 too, where `CreateForMonitor` doesn't exist.

use std::sync::OnceLock;

use windows::core::HSTRING;
use windows::Foundation::Metadata::ApiInformation;
use windows::Graphics::Capture::GraphicsCaptureSession;
use windows::Win32::Media::MediaFoundation::{
    MFAudioFormat_AAC, MFMediaType_Audio, MFMediaType_Video, MFT_FRIENDLY_NAME_Attribute,
    MFVideoFormat_H264, MFT_CATEGORY_AUDIO_ENCODER, MFT_CATEGORY_VIDEO_ENCODER,
    MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_REGISTER_TYPE_INFO,
};

use super::encoder::string_attr;
use super::mf;
use crate::take::CaptureStatus;

const OS_TOO_OLD: &str = "Screen recording needs Windows 10 version 1903 or later.";
const NO_ENCODER: &str = "This PC has no H.264 video encoder. \
     Install the Media Feature Pack (Settings › Apps › Optional features), then restart.";
const NO_AAC: &str = "This PC has no AAC audio encoder. \
     Install the Media Feature Pack (Settings › Apps › Optional features), then restart.";

static STATUS: OnceLock<CaptureStatus> = OnceLock::new();

/// Whether a software H.264 encoder and an AAC encoder are registered —
/// what the encoder smoke test needs before it can expect a file.
pub fn software_encoders(session: &mf::MfSession) -> (bool, bool) {
    let find = |category, major, subtype| {
        let info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: major,
            guidSubtype: subtype,
        };
        let flags = MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0;
        !session
            .mf()
            .tenum_ex(
                category,
                u32::try_from(flags).unwrap_or(0),
                None,
                Some(&info),
            )
            .unwrap_or_default()
            .is_empty()
    };
    (
        find(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFMediaType_Video,
            MFVideoFormat_H264,
        ),
        find(
            MFT_CATEGORY_AUDIO_ENCODER,
            MFMediaType_Audio,
            MFAudioFormat_AAC,
        ),
    )
}

/// Whether screen recording can work here. Checked once per process.
pub fn capture_status() -> CaptureStatus {
    STATUS.get_or_init(check).clone()
}

fn check() -> CaptureStatus {
    let unsupported = |reason: &str| CaptureStatus {
        supported: false,
        reason: Some(reason.to_string()),
        h264_encoder: None,
        hardware: false,
    };

    let api = match mf::api() {
        Ok(api) => api,
        Err(e) => return unsupported(&e.to_string()),
    };
    let session = match api.session() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "MFStartup failed");
            return unsupported(mf::FEATURE_PACK_HINT);
        }
    };
    let h264 = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let aac = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Audio,
        guidSubtype: MFAudioFormat_AAC,
    };
    let find = |category, output: &MFT_REGISTER_TYPE_INFO, flags: i32| {
        session
            .mf()
            .tenum_ex(
                category,
                u32::try_from(flags | MFT_ENUM_FLAG_SORTANDFILTER.0).unwrap_or(0),
                None,
                Some(output),
            )
            .unwrap_or_default()
    };
    let hardware = find(MFT_CATEGORY_VIDEO_ENCODER, &h264, MFT_ENUM_FLAG_HARDWARE.0);
    let software = find(MFT_CATEGORY_VIDEO_ENCODER, &h264, MFT_ENUM_FLAG_SYNCMFT.0);
    let first = hardware.first().or_else(|| software.first());
    let Some(first) = first else {
        return unsupported(NO_ENCODER);
    };
    let name = string_attr(first, &MFT_FRIENDLY_NAME_Attribute);
    if find(MFT_CATEGORY_AUDIO_ENCODER, &aac, MFT_ENUM_FLAG_SYNCMFT.0).is_empty() {
        return unsupported(NO_AAC);
    }

    let contract_ok = ApiInformation::IsApiContractPresentByMajor(
        &HSTRING::from("Windows.Foundation.UniversalApiContract"),
        8,
    )
    .unwrap_or(false);
    if !contract_ok || !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
        return unsupported(OS_TOO_OLD);
    }

    CaptureStatus {
        supported: true,
        reason: None,
        h264_encoder: name,
        hardware: !hardware.is_empty(),
    }
}
