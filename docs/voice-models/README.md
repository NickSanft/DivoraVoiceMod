# Voice models (Phase 12 — AI voice conversion)

DivoraVoice's **Voice Convert** effect runs an ONNX voice-conversion
model over the mic stream. This folder documents how to produce a
compatible model and install it. The reference model is **LLVC**
(KoeAI's *Low-latency Low-resource Voice Conversion*, MIT-licensed),
whose released checkpoint converts to LibriSpeech speaker 8312 — a
public-domain audiobook narrator.

## Model I/O contract

`divora-core/src/dsp/voice_convert.rs` supports **two** contracts and
picks per loaded model (a model is "streaming" if it has an `enc_buf`
input).

### Streaming (v1.3.0+, the bundled narrator) — ~13 ms latency

Threads four cache tensors + a 2·L front-context between 208-sample
chunks. Fixed shapes (batch 1, f32):

| dir | names | shape |
|---|---|---|
| in | `audio` | `[1, 1, 240]` (32 front-ctx + 208 chunk) |
| in | `enc_buf` | `[1, 512, 510]` |
| in | `dec_buf` | `[1, 2, 13, 256]` |
| in | `out_buf` | `[1, 512, 4]` |
| in | `convnet_pre_ctx` | `[1, 1, 24]` |
| out | `output` | `[1, 1, 208]` |
| out | `enc_buf_out` / `dec_buf_out` / `out_buf_out` / `convnet_pre_ctx_out` | same as the matching input |

All caches are zero-initialized (verified equivalent to the model's
`None`-init). The engine streams 208-sample chunks (`dec_chunk_size 13 ·
L 16`) ≈ 13 ms, threading the four `*_out` caches back in next call.

### Non-streaming (fallback) — ~256 ms latency

| dir | name | shape |
|---|---|---|
| in | `audio` | `[1, 1, T]` f32 |
| out | (first output) | `[1, 1, T]` f32 |

The engine sends `T = 4096`, converted statelessly per chunk. Any model
matching either contract works; a mismatch degrades to passthrough
(never crashes).

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

### Streaming export (v1.3.0)

[`export_streaming_onnx.py`](export_streaming_onnx.py) traces the
**streaming** `Net.forward(x, enc_buf, dec_buf, out_buf, convnet_pre_ctx,
pad=False)` — the five-in / five-out cache form — with the speechbrain
`PositionalEncoding` vendored into `model.py` (its lazy import breaks the
tracer) and `dynamo=False`. It then validates an 8-chunk streaming run of
ONNX vs PyTorch (expect `max|diff| ≈ 2e-7`) and prints the cache shapes
the Rust side threads. Run it from the LLVC repo root:

```bash
.venv/Scripts/python export_streaming_onnx.py llvc-narrator.onnx
```

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

The binaries are **not** in git. They live on the `voice-assets-v2`
GitHub release (v1 kept the older non-streaming narrator so pre-v1.3.0
tags still build against their own asset) and are fetched at
release-build time:

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
export, then `gh release upload voice-assets-v2 <files> --clobber`.

## Notes

- v1.3.0 ships the **streaming** export (208-sample chunks, ~13 ms),
  threading the model's four cache tensors between chunks. The
  non-streaming path (4096-sample independent chunks, ~256 ms) remains as
  the fallback for models that only expose the single `audio` → `output`
  contract.
- Bundling the DLL + a model into the installer (so it works without
  this manual step) is tracked as future packaging work.
