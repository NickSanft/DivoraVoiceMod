#!/usr/bin/env python3
"""Prepare the v1.17.0 text-to-speech ("Speak") assets for the release.

This is a ONE-TIME maintainer script documenting how the assets on the
`voice-assets-v2` GitHub release were produced. The app fetches the *outputs*
at build time via `scripts/fetch-voice-assets.ps1`; it never runs this.

Sources (all permissively licensed except espeak-ng, see README License):
  - Kokoro-82M ONNX (int8) + voices NPZ — thewh1teagle/kokoro-onnx release
      https://github.com/thewh1teagle/kokoro-onnx/releases/tag/model-files-v1.0
        kokoro-v1.0.int8.onnx   (~88 MB, Apache-2.0)
        voices-v1.0.bin         (NPZ of 54 voices, each (510, 1, 256) f32)
  - Kokoro config.json (holds the phoneme `vocab`) — hexgrad/Kokoro-82M
      https://huggingface.co/hexgrad/Kokoro-82M/resolve/main/config.json
      -> renamed to kokoro-config.json
  - espeak-ng (GPL-3.0) — official upstream MSI, admin-extracted (no install):
      gh release download 1.52.0 --repo espeak-ng/espeak-ng -p espeak-ng.msi
      msiexec /a espeak-ng.msi /qn TARGETDIR=<dir>
      -> espeak-ng.exe + libespeak-ng.dll + espeak-ng-data/ (zip the dir)

What this script does: convert the upstream voices NPZ into our compact,
dependency-free `voices-divora.bin` (the `DVTS` format read by
`divora-core/src/tts/kokoro.rs::StylePack`) containing only the preset voices.

Then upload to the release:
  gh release upload voice-assets-v2 \
    kokoro-v1.0.int8.onnx voices-divora.bin kokoro-config.json \
    espeak-ng.exe libespeak-ng.dll espeak-ng-data.zip \
    --repo NickSanft/DivoraVoiceMod --clobber

Usage:
  python scripts/prep-tts-voices.py <voices-v1.0.bin npz> <out voices-divora.bin>
"""
import struct
import sys

import numpy as np

# Must match PRESET_VOICES in divora-core/src/tts/mod.rs.
PRESET_VOICES = ["af_heart", "af_bella", "am_michael", "am_puck", "bf_emma", "bm_george"]
ROWS, DIM = 510, 256  # one (1,256) style vector per token length 0..509


def build(npz_path: str, out_path: str) -> None:
    voices = np.load(npz_path)
    missing = [v for v in PRESET_VOICES if v not in voices.files]
    if missing:
        raise SystemExit(f"voice pack missing presets: {missing}")

    out = bytearray()
    out += b"DVTS"
    out += struct.pack("<I", 1)                 # version
    out += struct.pack("<I", len(PRESET_VOICES))  # voice count
    out += struct.pack("<I", ROWS)
    out += struct.pack("<I", DIM)
    for name in PRESET_VOICES:
        arr = np.asarray(voices[name], dtype="<f4").reshape(ROWS, 1, DIM)[:, 0, :]
        nb = name.encode("utf-8")
        out += struct.pack("<I", len(nb))
        out += nb
        out += arr.tobytes()

    with open(out_path, "wb") as f:
        f.write(out)
    print(f"wrote {out_path}: {len(out)} bytes ({len(out) / 1e6:.2f} MB), "
          f"{len(PRESET_VOICES)} voices")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    build(sys.argv[1], sys.argv[2])
