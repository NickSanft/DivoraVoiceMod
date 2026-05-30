"""Validate the exported ONNX on real speech, processed in 4096-sample
16 kHz chunks — exactly how the DivoraVoice Rust engine feeds it. Saves
the converted clip so the voice change is audible/verifiable.
"""

import glob
import sys

import numpy as np
import onnxruntime as ort
import soundfile as sf

CHUNK = 4096
SR = 16000
ONNX = "llvc-narrator.onnx"


def resample_linear(x, src_sr, dst_sr):
    if src_sr == dst_sr:
        return x
    n_out = int(round(len(x) * dst_sr / src_sr))
    xp = np.linspace(0, 1, len(x), endpoint=False)
    fp = x
    q = np.linspace(0, 1, n_out, endpoint=False)
    return np.interp(q, xp, fp).astype(np.float32)


def main():
    wav = sys.argv[1] if len(sys.argv) > 1 else sorted(glob.glob("test_wavs/*.wav"))[0]
    audio, sr = sf.read(wav, dtype="float32")
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    audio = resample_linear(audio, sr, SR)
    print(f"input: {wav}  {sr}Hz -> {SR}Hz, {len(audio)} samples ({len(audio)/SR:.2f}s)")

    sess = ort.InferenceSession(ONNX, providers=["CPUExecutionProvider"])
    in_name = sess.get_inputs()[0].name
    out_name = sess.get_outputs()[0].name

    # Process in independent 4096-sample chunks (mirrors VoiceConverter).
    out = np.zeros_like(audio)
    n_chunks = len(audio) // CHUNK
    for i in range(n_chunks):
        seg = audio[i * CHUNK : (i + 1) * CHUNK].reshape(1, 1, CHUNK)
        res = sess.run([out_name], {in_name: seg})[0]
        out[i * CHUNK : (i + 1) * CHUNK] = np.asarray(res).reshape(-1)[:CHUNK]

    rms_in = float(np.sqrt(np.mean(audio[: n_chunks * CHUNK] ** 2)))
    rms_out = float(np.sqrt(np.mean(out[: n_chunks * CHUNK] ** 2)))
    peak_out = float(np.max(np.abs(out)))
    print(f"chunks={n_chunks}  rms_in={rms_in:.4f}  rms_out={rms_out:.4f}  peak_out={peak_out:.4f}")

    sf.write("converted_chunked.wav", out, SR)
    sf.write("input_16k.wav", audio, SR)
    print("wrote converted_chunked.wav + input_16k.wav")

    # Sanity gates: output should be real speech-energy, not silence or NaN/clip.
    assert np.isfinite(out).all(), "non-finite samples in output"
    assert rms_out > 0.005, f"output too quiet ({rms_out}) — model not converting?"
    assert peak_out <= 1.01, f"output clipping ({peak_out})"
    print("PASS: chunked conversion produced finite, audible, non-clipping speech")


if __name__ == "__main__":
    main()
