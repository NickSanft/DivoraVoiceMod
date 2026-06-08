//! v1.9.0: MIDI control-surface input.
//!
//! Opens a system MIDI input port (via `midir` — `WinMM` on Windows) and
//! forwards each parsed message to the frontend as a `midi-message`
//! event. The frontend owns the mapping layer (MIDI-learn → existing
//! store actions); this module is just transport + parsing.
//!
//! Mirrors the recording/tray pattern: a command opens the port and the
//! live connection is held in `AppState`; dropping it closes the port.
//! The live port open is desktop-verified (a headless CI runner lists
//! zero ports), so the unit tests cover the pure message parser + the
//! frozen wire shape.

use midir::{Ignore, MidiInput, MidiInputConnection};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Client name we register with the OS MIDI subsystem.
const CLIENT_NAME: &str = "DivoraVoice";

/// One available MIDI input port. `id` and `name` are both the port's
/// display name — the stable handle we re-resolve on open, since midir
/// port indices aren't stable across hot-plug.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiInputInfo {
    pub id: String,
    pub name: String,
}

/// Wire payload of the `midi-message` event. `kind` is a stable string
/// the frontend router matches on; `data1`/`data2` are the two MIDI
/// data bytes (note + velocity, or controller + value). A note-on with
/// velocity 0 is normalized to `note-off` (a common keyboard idiom) so
/// momentary bindings release cleanly.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiMessage {
    pub channel: u8,
    pub kind: String,
    pub data1: u8,
    pub data2: u8,
}

/// Parse a raw MIDI message into a [`MidiMessage`]. Returns `None` for
/// system messages (clock, sysex, …) and anything without a leading
/// status byte — those don't drive bindings, so we don't emit them.
#[must_use]
pub fn parse_midi_message(bytes: &[u8]) -> Option<MidiMessage> {
    let status = *bytes.first()?;
    // A data byte (< 0x80) with no leading status: running status isn't
    // something WinMM hands us, so ignore it defensively.
    if status < 0x80 {
        return None;
    }
    let channel = status & 0x0F;
    // Data bytes are 7-bit; mask the high bit defensively.
    let data1 = bytes.get(1).copied().unwrap_or(0) & 0x7F;
    let data2 = bytes.get(2).copied().unwrap_or(0) & 0x7F;
    let kind = match status & 0xF0 {
        0x80 => "note-off",
        0x90 => {
            if data2 == 0 {
                "note-off"
            } else {
                "note-on"
            }
        }
        0xA0 => "aftertouch-poly",
        0xB0 => "control-change",
        0xC0 => "program-change",
        0xD0 => "aftertouch-channel",
        0xE0 => "pitch-bend",
        // 0xF0: system common / real-time — not a binding source.
        _ => return None,
    };
    Some(MidiMessage {
        channel,
        kind: kind.to_owned(),
        data1,
        data2,
    })
}

/// Enumerate the available MIDI input ports. Resilient: returns an empty
/// list (never errors / panics) when the MIDI subsystem is unavailable
/// or no ports exist — the common case on a CI runner.
#[must_use]
pub fn list_inputs() -> Vec<MidiInputInfo> {
    let Ok(input) = MidiInput::new(CLIENT_NAME) else {
        return Vec::new();
    };
    input
        .ports()
        .iter()
        .filter_map(|port| {
            input.port_name(port).ok().map(|name| MidiInputInfo {
                id: name.clone(),
                name,
            })
        })
        .collect()
}

/// Open the MIDI input port whose display name is `name` and forward
/// every parsed message to the frontend as a `midi-message` event.
/// Returns the live connection — the caller keeps it alive in
/// `AppState`; dropping it closes the port.
pub fn open_input(app: &AppHandle, name: &str) -> Result<MidiInputConnection<()>, String> {
    let mut input = MidiInput::new(CLIENT_NAME).map_err(|e| e.to_string())?;
    // We only care about note / CC traffic, not active-sensing or timing.
    input.ignore(Ignore::All);
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| format!("MIDI input '{name}' not found"))?;
    let app = app.clone();
    input
        .connect(
            &port,
            "divora-midi-in",
            move |_stamp, bytes, ()| {
                if let Some(msg) = parse_midi_message(bytes) {
                    // An emit error means the app is shutting down (no
                    // receivers) — nothing to do here.
                    let _ = app.emit("midi-message", &msg);
                }
            },
            (),
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{list_inputs, parse_midi_message, MidiMessage};

    fn sorted_keys(v: &serde_json::Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
        k.sort();
        k
    }

    #[test]
    fn note_on_parses() {
        let msg = parse_midi_message(&[0x90, 60, 100]).expect("note on");
        assert_eq!(msg.kind, "note-on");
        assert_eq!(msg.channel, 0);
        assert_eq!(msg.data1, 60);
        assert_eq!(msg.data2, 100);
    }

    #[test]
    fn note_on_velocity_zero_is_note_off() {
        // 0x95 = note-on, channel 5. Velocity 0 is the standard "off".
        let msg = parse_midi_message(&[0x95, 60, 0]).expect("note on vel 0");
        assert_eq!(msg.kind, "note-off");
        assert_eq!(msg.channel, 5);
    }

    #[test]
    fn note_off_parses() {
        let msg = parse_midi_message(&[0x82, 48, 64]).expect("note off");
        assert_eq!(msg.kind, "note-off");
        assert_eq!(msg.channel, 2);
        assert_eq!(msg.data1, 48);
    }

    #[test]
    fn control_change_parses() {
        let msg = parse_midi_message(&[0xB0, 7, 127]).expect("cc");
        assert_eq!(msg.kind, "control-change");
        assert_eq!(msg.data1, 7);
        assert_eq!(msg.data2, 127);
    }

    #[test]
    fn channel_is_the_status_low_nibble() {
        let msg = parse_midi_message(&[0xBF, 1, 2]).expect("cc ch15");
        assert_eq!(msg.channel, 15);
        assert_eq!(msg.kind, "control-change");
    }

    #[test]
    fn program_change_has_one_data_byte() {
        let msg = parse_midi_message(&[0xC3, 5]).expect("program change");
        assert_eq!(msg.kind, "program-change");
        assert_eq!(msg.data1, 5);
        assert_eq!(msg.data2, 0);
    }

    #[test]
    fn system_messages_are_ignored() {
        assert!(parse_midi_message(&[0xF8]).is_none()); // timing clock
        assert!(parse_midi_message(&[0xF0, 0x7E, 0xF7]).is_none()); // sysex
    }

    #[test]
    fn empty_or_data_only_is_ignored() {
        assert!(parse_midi_message(&[]).is_none());
        assert!(parse_midi_message(&[0x40, 0x50]).is_none()); // no status byte
    }

    #[test]
    fn json_keys_are_frozen() {
        let msg = MidiMessage {
            channel: 0,
            kind: "note-on".to_owned(),
            data1: 60,
            data2: 100,
        };
        assert_eq!(
            sorted_keys(&serde_json::to_value(&msg).expect("serialize")),
            ["channel", "data1", "data2", "kind"]
        );
    }

    #[test]
    fn list_inputs_does_not_panic() {
        // Headless CI has no ports; just assert it returns cleanly
        // (mirrors the cpal device-list tests).
        let _ = list_inputs();
    }
}
