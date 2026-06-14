#!/usr/bin/env python3
"""Generate `base-ses.json` — the per-preset speaker embeddings used by Speak's
voice-clone **auto base-match** (v1.22.0).

For each bundled Kokoro preset we synthesize a fixed pangram, resample it to the
OpenVoice rate, and run the OpenVoice tone-color **extractor** to get a 256-d
speaker embedding (SE). At clone time the app (`tts::pick_base_voice`) compares a
user's reference SE against these and generates from the closest-sounding preset,
so a clone inherits the nearest accent/gender instead of one fixed base.

The spectrogram + extractor invocation here mirror `src-tauri/resources/_spike/
spike.py` exactly, which is the same math ported to Rust in
`divora-core/src/tts/clone.rs` — keep all three in sync.

Output: `src-tauri/resources/tts/base-ses.json` — `{ "<voiceId>": [256 floats] }`.
It is tiny (~37 KB) so, unlike the OpenVoice models, it is **bundled** (mapped in
`tauri.bundle.conf.json`, fetched by `fetch-voice-assets.ps1`).

Requirements (local, not in CI): the staged Kokoro + espeak-ng assets in
`src-tauri/resources/tts/`, plus the OpenVoice extractor — either the staged
`openvoice-extractor.onnx` there or `_spike/tone_color_extract_model.onnx`.

    python scripts/prep-base-ses.py

After running, upload the result to the `voice-assets-v2` release:
    gh release upload voice-assets-v2 src-tauri/resources/tts/base-ses.json \
        --repo NickSanft/DivoraVoiceMod --clobber
"""
import json
import os
import re
import struct
import subprocess

import numpy as np
import onnxruntime as ort
import scipy.io.wavfile as wav

HERE = os.path.abspath(os.path.dirname(__file__))
ROOT = os.path.abspath(os.path.join(HERE, ".."))
TTS = os.path.join(ROOT, "src-tauri", "resources", "tts")
SPIKE = os.path.join(ROOT, "src-tauri", "resources", "_spike")

# OpenVoice converter config (matches spike.py / clone.rs).
SR = 22050
NFFT = 1024
HOP = 256
WIN = 1024

# Must match PRESET_VOICES in divora-core/src/tts/mod.rs: (id, espeak lang).
PRESETS = [
    ("af_heart", "en-us"),
    ("af_bella", "en-us"),
    ("af_aoede", "en-us"),
    ("am_michael", "en-us"),
    ("am_puck", "en-us"),
    ("bf_emma", "en-gb"),
    ("bm_george", "en-gb"),
]

# A balanced pangram — every base says the same thing, so the SEs differ only by
# the voice's timbre, not the content.
PANGRAM = "The quick brown fox jumps over the lazy dog."


def find_extractor():
    for p in (
        os.path.join(TTS, "openvoice-extractor.onnx"),
        os.path.join(SPIKE, "tone_color_extract_model.onnx"),
    ):
        if os.path.exists(p):
            return p
    raise SystemExit(
        "OpenVoice extractor not found. Stage openvoice-extractor.onnx in "
        f"{TTS} (or tone_color_extract_model.onnx in {SPIKE})."
    )


def resample(audio, src_rate, dst_rate=SR):
    if src_rate == dst_rate:
        return audio.astype(np.float32)
    dur = audio.shape[0] / src_rate
    n = int(dur * dst_rate)
    return np.interp(
        np.linspace(0, dur, n),
        np.linspace(0, dur, audio.shape[0]),
        audio,
    ).astype(np.float32)


def spectrogram(y):
    y = np.pad(y, int((NFFT - HOP) / 2), mode="reflect")
    window = np.hanning(WIN + 1)[:-1].astype(y.dtype)
    n = int((y.shape[0] - WIN) // HOP) + 1
    spec = np.empty((n, NFFT // 2 + 1, 2), dtype=np.float32)
    s = 0
    for i in range(n):
        f = np.fft.rfft(y[s : s + WIN] * window)
        spec[i] = np.column_stack((f.real, f.imag))
        s += HOP
    spec = np.sqrt(np.sum(spec**2, axis=-1) + 1e-6)
    return np.array([spec], dtype=np.float32)  # [1, frames, 513]


# ---- OpenVoice extractor ----
EXTRACT = ort.InferenceSession(find_extractor(), providers=["CPUExecutionProvider"])


def extract_se(audio):
    spec = spectrogram(audio.astype(np.float32))
    return EXTRACT.run(None, {"input": spec})[0].reshape(256).astype(np.float32)


# ---- Kokoro (reuse the staged Phase-1 assets) ----
KVOCAB = json.load(open(os.path.join(TTS, "kokoro-config.json"), encoding="utf-8"))["vocab"]
KSESS = ort.InferenceSession(
    os.path.join(TTS, "kokoro-v1.0.int8.onnx"), providers=["CPUExecutionProvider"]
)
_b = open(os.path.join(TTS, "voices-divora.bin"), "rb").read()
_o = 4
_ver, _n, _rows, _dim = struct.unpack_from("<IIII", _b, _o)
_o += 16
KPACK = {}
for _ in range(_n):
    (_nl,) = struct.unpack_from("<I", _b, _o)
    _o += 4
    _nm = _b[_o : _o + _nl].decode()
    _o += _nl
    KPACK[_nm] = np.frombuffer(_b, dtype="<f4", count=_rows * _dim, offset=_o).reshape(
        _rows, _dim
    )
    _o += _rows * _dim * 4

_clean = lambda s: re.sub(
    r"\s+", " ", re.sub(r"\([^)]*\)", "", s).replace("͡", "").replace("‍", "")
).strip()


def kokoro(text, voice, lang):
    r = subprocess.run(
        [
            os.path.join(TTS, "espeak-ng.exe"),
            "-q",
            "--ipa",
            "-v",
            lang,
            "--path",
            TTS,
            "--",
            text,
        ],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    ids = [KVOCAB[c] for c in _clean(r.stdout) if c in KVOCAB]
    style = KPACK[voice][min(len(ids), _rows - 1)].astype("<f4").reshape(1, 256)
    a = KSESS.run(
        ["audio"],
        {
            "tokens": np.array([[0, *ids, 0]], dtype=np.int64),
            "style": style,
            "speed": np.array([1.0], dtype=np.float32),
        },
    )[0]
    return np.asarray(a).reshape(-1)  # 24 kHz


def main():
    out = {}
    for voice, lang in PRESETS:
        k = kokoro(PANGRAM, voice, lang)  # 24 kHz
        k22 = resample(k, 24000, SR)  # -> 22.05 kHz
        se = extract_se(k22)
        out[voice] = [float(x) for x in se]
        print(f"  {voice:<11} ({lang}) -> SE[256]")
    dest = os.path.join(TTS, "base-ses.json")
    with open(dest, "w", encoding="utf-8") as f:
        json.dump(out, f)
    kb = os.path.getsize(dest) / 1024
    print(f"\nwrote {dest}: {kb:.1f} KB, {len(out)} voices")


if __name__ == "__main__":
    main()
