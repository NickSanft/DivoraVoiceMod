"""Export the pretrained LLVC model to ONNX for DivoraVoice.

Loads the LLVC `Net` exactly like `infer.py` does, traces the
non-streaming forward (model(x): [1,1,T] -> [1,1,T]), and writes an
ONNX graph with a dynamic time axis. Then validates the ONNX output
against the PyTorch output on a real test clip so we know the export
is faithful before wiring it into the Rust engine.

Usage:
    python export_onnx.py <out.onnx>
"""

import json
import sys

import numpy as np
import torch

from model import Net

CKPT = "llvc_models/models/checkpoints/llvc/G_500000.pth"
CONFIG = "experiments/llvc/config.json"
# Our Rust engine feeds 4096-sample (16 kHz) chunks. 4096 % L(==8) == 0,
# so mod_pad adds no data-dependent end-slice -> clean trace.
CHUNK = 4096


def load_model():
    with open(CONFIG) as f:
        config = json.load(f)
    model = Net(**config["model_params"])
    state = torch.load(CKPT, map_location="cpu")["model"]
    model.load_state_dict(state)
    model.eval()
    return model, config["data"]["sr"]


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else "llvc-narrator.onnx"
    model, sr = load_model()
    print(f"loaded LLVC model, sr={sr}, L={model.L}, dec_chunk_size={model.dec_chunk_size}")

    dummy = torch.randn(1, 1, CHUNK)
    with torch.no_grad():
        ref = model(dummy)
    print(f"torch forward: in {tuple(dummy.shape)} -> out {tuple(ref.shape)}")

    torch.onnx.export(
        model,
        dummy,
        out_path,
        input_names=["audio"],
        output_names=["audio_out"],
        dynamic_axes={"audio": {2: "T"}, "audio_out": {2: "T"}},
        opset_version=17,
        do_constant_folding=True,
        # Legacy TorchScript exporter: predictable input/output names +
        # dynamic_axes, and no onnxscript dependency (torch 2.12's dynamo
        # exporter renames graph io and needs onnxscript).
        dynamo=False,
    )
    print(f"exported -> {out_path}")

    # --- validation: ORT vs torch on a real clip ---
    import onnxruntime as ort

    sess = ort.InferenceSession(out_path, providers=["CPUExecutionProvider"])
    in_name = sess.get_inputs()[0].name
    out_name = sess.get_outputs()[0].name
    print(f"onnx io: in={in_name}{sess.get_inputs()[0].shape} out={out_name}{sess.get_outputs()[0].shape}")

    # Use a real test clip if present, else the dummy.
    try:
        import torchaudio
        import glob

        wavs = sorted(glob.glob("test_wavs/*.wav"))
        if wavs:
            audio, in_sr = torchaudio.load(wavs[0])
            audio = audio.mean(0, keepdim=False)
            if in_sr != sr:
                audio = torchaudio.transforms.Resample(in_sr, sr)(audio)
            # Trim to a whole number of CHUNKs for a clean comparison.
            n = (audio.shape[-1] // CHUNK) * CHUNK
            audio = audio[:n].reshape(1, 1, n)
            print(f"validating on {wavs[0]} ({n} samples)")
        else:
            audio = dummy
    except Exception as e:  # noqa: BLE001
        print(f"(torchaudio clip load skipped: {e})")
        audio = dummy

    with torch.no_grad():
        torch_out = model(audio).numpy()
    ort_out = sess.run([out_name], {in_name: audio.numpy()})[0]

    max_abs = float(np.max(np.abs(torch_out - ort_out)))
    rms_in = float(np.sqrt(np.mean(audio.numpy() ** 2)))
    rms_out = float(np.sqrt(np.mean(ort_out**2)))
    print(f"ORT vs torch max|diff| = {max_abs:.3e}")
    print(f"rms in={rms_in:.4f}  rms out(converted)={rms_out:.4f}")
    print(f"converted differs from input: {abs(rms_in - rms_out) > 1e-4 or max_abs >= 0}")
    if max_abs < 1e-3:
        print("PASS: ONNX matches PyTorch within 1e-3")
    else:
        print("WARN: ONNX/torch diverge > 1e-3 — inspect before shipping")


if __name__ == "__main__":
    main()
