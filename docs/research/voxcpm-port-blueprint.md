# VoxCPM-0.5B → Rust `ort` port blueprint

**Status:** Phase A validated (2026-06-15). Implementation spec for Phases B–E of
the accent-preserving cloning productization (see
[accent-preserving-cloning.md](accent-preserving-cloning.md)).

## Phase A result — pure ONNX Runtime on CPU works

Ran VoxCPM-0.5B end-to-end under **pure onnxruntime on CPU** (no PyTorch in the
graph execution) via the community export `bluryar/voxcpm-onnx` + its `infer.py`,
cloning `me_12s.wav`:

- ✅ Produces a valid clone: 5.04 s @ 16 kHz, **timbre cosine 0.868 vs me.wav**
  (≥ the torch path's 0.854, on par with OpenVoice's ~0.89).
- ✅ 4-graph decomposition loads + runs under ORT CPU; VAE-encoded the 12 s prompt
  to a (149, 2, 64) patch tensor; 46 text tokens.
- fp32 export is **4.8 GB** (prefill 2.4 GB + decode_step 2.4 GB + VAE enc 0.19 +
  dec 0.11). **Quantized Q8 ≈ 1.5 GB** and DakeQQ benchmarks Q8 at **RTF ~1.5 on
  an i3-12300 CPU** — the product target (vs the torch fp32 run's RTF ~3).

**Conclusion:** the `ort` port is viable. The hard part is engineering the decode
loop + KV cache + tokenizer in Rust, not whether ORT can run the model.

## The pipeline (port target — bluryar 4-graph)

Two community exports were inspected:
- **bluryar** (4 graphs): `voxcpm_prefill`, `voxcpm_decode_step`,
  `audio_vae_encoder`, `audio_vae_decoder` + `tokenizer.json`. **Simpler — chosen
  port target.**
- **DakeQQ v1.5** (8 graphs, more fused/optimized): adds split Prefill /
  Feat_Encoder_Cond / Rotary_Mask / Main with a Feat_Decoder that bakes the whole
  diffusion loop into one call; published Q8 RTF 1.5. Keep as the optimization
  reference for Phase C tuning.

Flow (one-time prompt encode, then per-sentence generate):

```
prompt audio (int16 @44.1k, [1,1,T]) ─► audio_vae_encoder ─► audio_feat (patches)
text (target) ─► LlamaTokenizerFast ─► token ids
prefill(prompt_ids, target_ids, audio_feat/feat_embed) ─► hidden, rotary, mask, kv_seed
loop (autoregressive, until stop token "1" or max_len):
    decode_step(kv_cache, feat_embed, hidden, rotary, mask, cfg) ─► kv_cache', latent, stop
    accumulate latent
audio_vae_decoder(concat latents) ─► waveform @ output rate
```

## Rust port plan (Phase C)

`ort` is already a workspace dep. New `divora-core/src/tts/voxcpm.rs`, mirroring the
existing `clone.rs`/`kokoro.rs` `ort` patterns.

| Component | Source of truth | Rust approach | Risk |
|---|---|---|---|
| 4 ORT sessions | bluryar `runtime.py` | `ort::Session` load (load-dynamic, like kokoro) | low |
| Tokenizer (LlamaTokenizerFast) | `tokenizer.json` | `tokenizers` crate (`Tokenizer::from_file`); drop the Chinese multichar masking for EN | low–med |
| Prompt audio → VAE encode | bluryar `inputs.py` | reuse `decode_clip` → int16 @44.1k → `audio_vae_encoder` session | low |
| Decode loop + KV cache | bluryar `infer_loop.py` | port the loop; thread KV `ort::Value`s between steps; stop on token 1 / max_len | **high** (the core work) |
| Rotary / mask / feat_embed plumbing | inference scripts | follow the I/O names exactly; `#[ignore]` test compares to the Python ORT oracle | med |
| VAE decode → f32 | bluryar `vae.py` | `audio_vae_decoder` → f32; resample to 48 k via existing `rubato` mixer | low |
| Quantize to Q8 (~1.5 GB) | DakeQQ `Optimize_ONNX.py` / ORT quantization | produce + host Q8 models; validate quality vs fp32 | med |

**Validation strategy:** keep the working Python ORT runner (`_spike/accent/bluryar`)
as the **reference oracle**. Port graph-by-graph, asserting Rust ORT outputs match
the Python ORT outputs numerically, then audition the final audio. An `#[ignore]`d
`voxcpm_clones_match_python` test (local, with staged models) gates correctness —
same pattern as `clone.rs::real_clone_moves_timbre_toward_target`.

## Phase B — the reference transcript (ASR)

VoxCPM-0.5B continuation mode needs the prompt's transcript. Options:
1. **User types it** — simplest, zero new model. The in-app recorder (v1.23.0)
   could ask the user to read a known sentence (so the transcript is fixed/known)
   — elegant: no ASR at all. **Leading option.**
2. **Bundled whisper ONNX** via `ort` (e.g. whisper-base.en ONNX, ~150 MB) for the
   "import an arbitrary clip" path. Adds a model + a decode loop. Defer unless (1)
   proves insufficient.

Decision leaning: **fixed read-aloud sentence** for the in-app recorder (no ASR),
optional whisper-ONNX later for imported clips.

## Phases D–E

- **D:** route cloned voices through VoxCPM (presets stay on Kokoro); host Q8 models
  (~1.5 GB) on the `voice-assets-v2` release for download-on-demand (extend the
  v1.21.0 `download_clone_models` machinery); transcript capture UX.
- **E:** full checklist + docs + manual tests; ship as a **major** version (new
  engine, larger download). OpenVoice stays as the fallback path.

## Risks

- **The decode loop in Rust `ort`** is the bulk of the effort (KV cache lifetimes,
  exact I/O ordering). Mitigated by the Python ORT oracle.
- **Download size**: ~1.5 GB even quantized (vs 157 MB today) — on-demand only.
- **CPU latency**: ~RTF 1.5 (Q8) → ~7 s for a 4–5 s sentence on a budget CPU;
  acceptable for batch Speak, must show progress UI.
- **Maintenance**: a second TTS engine alongside Kokoro+OpenVoice.
