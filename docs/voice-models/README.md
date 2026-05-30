# Voice models (Phase 12 — AI voice conversion)

DivoraVoice's **Voice Convert** effect runs an ONNX voice-conversion
model over the mic stream. This folder documents how to produce a
compatible model and install it. The reference model is **LLVC**
(KoeAI's *Low-latency Low-resource Voice Conversion*, MIT-licensed),
whose released checkpoint converts to LibriSpeech speaker 8312 — a
public-domain audiobook narrator.

## Model I/O contract

The Rust engine (`divora-core/src/dsp/voice_convert.rs`) feeds the
model **4096-sample, 16 kHz mono chunks** and expects:

| | name | shape | dtype |
|---|---|---|---|
| input | `audio` | `[1, 1, T]` | f32 |
| output | (first output) | `[1, 1, T]` | f32 |

`T` is dynamic; the engine always sends `T = 4096`. Any ONNX model
matching this contract works — drop a different one in and select it.
If the shapes/names don't match, inference errors and the effect
degrades to passthrough (never crashes).

## Producing the LLVC model

Requires Python 3.11/3.12 + ~2 GB for PyTorch. **fairseq is NOT
needed** — LLVC's own `Net` only imports torch / torchaudio /
speechbrain. (fairseq is only for LLVC's RVC *comparison baseline*.)

```bash
# 1. Clone LLVC
git clone --depth 1 https://github.com/KoeAI/LLVC.git llvc-export
cd llvc-export

# 2. Isolated env + the MINIMAL deps (skip requirements.txt → no fairseq)
python -m venv .venv
.venv/Scripts/python -m pip install torch torchaudio --index-url https://download.pytorch.org/whl/cpu
.venv/Scripts/python -m pip install speechbrain numpy huggingface_hub onnx onnxruntime soundfile

# 3. Download just the LLVC checkpoint (not the RVC baseline models)
.venv/Scripts/python -c "from huggingface_hub import hf_hub_download as d; d(repo_id='KoeAI/llvc', filename='models/checkpoints/llvc/G_500000.pth', local_dir='llvc_models')"

# 4. Vendor PositionalEncoding (speechbrain's lazy-import breaks ONNX
#    export tracing). In model.py, replace:
#        from speechbrain.lobes.models.transformer.Transformer import PositionalEncoding
#    with the verbatim class from speechbrain (it's a fixed sinusoidal
#    buffer, so values stay bit-identical to the trained checkpoint):
.venv/Scripts/python -c "import inspect; from speechbrain.lobes.models.transformer.Transformer import PositionalEncoding as P; print(inspect.getsource(P))"

# 5. Export + validate (copy export_onnx.py + validate_chunked.py here)
.venv/Scripts/python export_onnx.py llvc-narrator.onnx
.venv/Scripts/python validate_chunked.py        # converts a test_wavs clip
```

`export_onnx.py` traces the **non-streaming** `Net.forward` (it
self-pads to a multiple of L=16, so 4096-sample chunks need no boundary
padding) with `dynamo=False` for a predictable graph, then checks ORT
output vs PyTorch (expect `max|diff| < 1e-3`).

## Installing

### From a release installer (v0.12.4+)

The MSI/NSIS installer **bundles `onnxruntime.dll` + `llvc-narrator.onnx`**
— voice conversion works out of the box. Install, launch, open
*Settings → Voice library* ("ONNX Runtime detected", `llvc-narrator`
listed), select the voice, and pick the *Deep Narrator* preset.

### Manual (dev builds / your own model)

1. **Model** → `%APPDATA%\com.divora.voicemod\voices\<id>.onnx`
   (the folder *Settings → Voice library* "Open folder" opens). User
   voices here shadow a bundled voice with the same id.
2. **ONNX Runtime DLL** → copy `onnxruntime.dll` next to the app
   executable, or set `ORT_DYLIB_PATH` to it (`onnxruntime`'s pip wheel
   ships it at `.venv/Lib/site-packages/onnxruntime/capi/onnxruntime.dll`).
3. Launch → select the voice → enable **Voice Convert**.

## Bundling pipeline (maintainers)

The binaries are **not** in git. They live on the `voice-assets-v1`
GitHub release and are fetched at release-build time:

- `scripts/fetch-voice-assets.ps1` downloads them into
  `src-tauri/resources/` (gitignored).
- `src-tauri/tauri.bundle.conf.json` is a config **overlay** that adds
  `bundle.resources` mapping them to `<resource_dir>/onnxruntime.dll` +
  `<resource_dir>/voices/llvc-narrator.onnx`. It's kept separate from
  the base `tauri.conf.json` because Tauri validates resource paths at
  build-script time — referencing them in the base config would break
  every `cargo build` / CI run that lacks the assets.
- `release.yml` runs the fetch, then
  `pnpm tauri build --config src-tauri/tauri.bundle.conf.json`.
- At runtime the app sets `ORT_DYLIB_PATH` to the bundled DLL and
  `list_voices` merges the bundled `voices/` dir with the user dir.

To refresh the hosted assets (new model / runtime version), re-run the
export, then `gh release upload voice-assets-v1 <files> --clobber`.

## Notes

- The non-streaming forward converts each 4096-sample chunk
  independently. LLVC's small internal lookahead keeps seam artifacts
  negligible at this chunk size; a future streaming export could thread
  the model's cache tensors between chunks for even lower latency.
- Bundling the DLL + a model into the installer (so it works without
  this manual step) is tracked as future packaging work.
