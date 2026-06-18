# Research spike — accent-preserving voice cloning for Speak

**Date:** 2026-06-14 · **Status:** research / go-no-go (no shippable code) ·
**Outcome:** **conditional GO for a feasibility spike** (see recommendation).

## The question

Speak's current cloning (v1.20.0–v1.23.0) is **timbre transfer only**: Kokoro-82M
generates the speech (its base voice supplies the accent, rhythm, intonation) and
OpenVoice v2 repaints the *tone color* toward the user. So a clone of a speaker
whose accent isn't US/UK still sounds US/UK. The user's feedback on the original
spike was exactly this — *"the final voice sounds pretty far away… especially the
accent."*

**Can we preserve the user's accent/prosody when cloning a voice for Speak,**
on-device, within DivoraVoice's constraints?

### Hard constraints

1. **On-device, CPU-only.** Offline/batch is fine — Speak is not real-time;
   a few seconds per utterance is acceptable.
2. **Permissive license (MIT/Apache/BSD).** No non-commercial (rules out Coqui
   XTTS-v2). No GPL in app code. **Code license ≠ weights license** — verify both.
3. **ONNX-exportable / runnable via `ort`** (the existing runtime), or cleanly
   subprocess-able. No bundled PyTorch.
4. **Model + deps ideally < ~1 GB** (download-on-demand acceptable, as the current
   ~157 MB OpenVoice models already are). Multi-GB GPU-class models are out.
5. **Short reference clip (~20–30 s); no per-voice training.**

## The core finding: why timbre-only fails, and what actually fixes it

Speech ≈ **content** (words) + **prosody** (F0, rhythm, stress, timing) +
**pronunciation/accent** (phoneme realization, vowel quality) + **timbre** (vocal
identity). **Accent lives in prosody + pronunciation, not timbre.**

- **Voice conversion (VC) is the wrong paradigm.** VC takes content/prosody/accent
  from its *source* waveform and only timbre from the *target* reference. In our
  pipeline `Kokoro → VC(user)`, the source is Kokoro, so the output keeps **Kokoro's**
  accent — the *identical* limitation we have today, for the identical reason.
  Swapping OpenVoice for kNN-VC / FreeVC / Seed-VC / RVC changes the timbre engine
  but **not** the accent. (The user's accent only exists in audio the *user*
  produced.)
- **Accent conversion (AC)** is the conceptually-right but **not-yet-shippable**
  paradigm. All current AC does accent *normalization toward a known native accent*
  (e.g. "foreign → American"), not "re-accent a line into an arbitrary user's
  personal accent from a short clip." The one permissive-*code* toolkit that does AC
  (Amphion **Vevo-Style**) ships **non-commercial weights** (CC-BY-NC, trained on the
  NC Emilia dataset).
- **Reference-conditioned zero-shot TTS is the realistic path.** A single model that
  ingests **text + the user's reference clip** and generates speech infers prosody
  and accent *from the reference*. This is the only class that both fits the product
  promise ("type → hear *you*") and can carry accent — and, unlike when we last
  looked, a few candidates now clear all five constraints.

## Candidate evaluation (mid-2026 landscape)

Merged from three parallel research passes (zero-shot TTS, voice conversion,
CPU/ONNX/license engineering reality). Sources at the end.

| Engine | License (code / weights) | Preserves accent? | CPU feasibility | ONNX / `ort` fit | Size | Verdict |
|---|---|---|---|---|---|---|
| **VoxCPM-0.5B** (OpenBMB) | **Apache-2.0 / Apache-2.0** | **Yes — explicitly "timbre, accent, rhythm, pacing"** | RTF ~1.5 on a budget CPU (i3-12300, Q8) via community ONNX | Community ONNX; diffusion-AR decode loop wired in `ort` yourself | ~0.5B (avoid the 2B/~16 GB variant) | **Primary candidate** |
| **Chatterbox** (Resemble AI) | **MIT / MIT** (cleanest in field) | **Yes — "timbre, accent, rhythm," holds across languages** | Real community CPU ONNX export; per-utterance CPU speed unverified | **Maintained MIT ONNX-community export, CPU supported** | 350M / 500M (multilingual) | **Secondary candidate** (note: non-removable Perth watermark) |
| **CosyVoice2-0.5B** (Alibaba) | **Apache-2.0 / Apache-2.0** | Yes — strong speaker similarity + prosody | **~70–220 s/utterance on CPU** (AR Qwen-0.5B decode) | Best ONNX story (full pipeline, PyTorch-free) **but** | **~3.8 GB of ONNX weights** | **Watch** — clean license, but too big/slow on CPU *today* |
| **GPT-SoVITS** | **MIT / permissive deps** | Partial (best with ~1 min + quick fine-tune; 5 s zero-shot weaker) | **Best proven: RTF ~0.53 on M4 CPU** | Partial official ONNX (build cnhubert + AR loop yourself); only Rust crate uses **libtorch, not `ort`** | ~1 GB (v2-class; can trim Chinese RoBERTa for EN) | **Fallback** — viable but high effort + leans on fine-tune |
| F5-TTS | MIT / **CC-BY-NC** | Yes | **~7m40s for 8 words on CPU** (32×-DiT flow loop) | Good 3-graph ONNX, but ODE loop outside graph | ~335M | Blocked — NC weights + CPU-hopeless |
| Fish-Speech / OpenAudio | Apache / **CC-BY-NC-SA / Fish Research** | Yes | Very slow (4B+400M AR) | **None** ("not planned") | 4B (0.5B mini) | Blocked — NC weights + no ONNX |
| StyleTTS2 | MIT / weights ok, **but GPL espeak at runtime** | **No — loses target accent zero-shot** | Slow-ish, diffusion-sampler bound | Partial/unofficial | Moderate | Blocked — doesn't preserve accent + GPL G2P |
| Coqui XTTS-v2 | MPL / **CPML (non-commercial)** | Yes | Slow | Community | ~1.8 GB | Blocked — NC; Coqui defunct (no license to buy) |
| Parler-TTS | Apache / Apache | **No clip cloning** (voice from text description) | Slow | — | ~0.6–1 B | Blocked — wrong modality |
| kNN-VC / FreeVC | MIT / MIT | VC (source accent) | FreeVC has CPU/ONNX via w-okada | partial | **+WavLM-Large 1.26 GB** | Blocked — VC paradigm + 1.26 GB SSL dep |
| Seed-VC | **GPL-3.0** / — | VC | CPU ~3× slower | roadmap | modular | Blocked — GPL |
| RVC | MIT / per-model | VC | CPU/ONNX via w-okada | yes | +ContentVec ~360 MB | Blocked — **requires per-voice training** |
| Amphion **Vevo-Style** | MIT / **CC-BY-NC** | **Yes — actual accent conversion** | heavy, no CPU/ONNX | none | large | Blocked — NC weights (watch for a permissive retrain) |

**The recurring trap (confirmed repeatedly):** permissive *code* with
**non-commercial weights** — F5-TTS, Fish/OpenAudio, Spark-TTS, MaskGCT, Llasa,
XTTS-v2, Vevo. One search even mislabeled IndexTTS2 as "Apache, commercial-ready"
when its actual `LICENSE` is a proprietary bilibili agreement. **Always verify the
HF model card + LICENSE file at implementation time.**

## Recommendation: conditional GO for a feasibility spike

This is a **change from the previous flat "blocked on the model landscape"** note —
which was correct for the *real-time* changer (still true), but the bar for
**offline Speak cloning** is lower, and the landscape has matured. There are now
**permissive (Apache/MIT code *and* weights), CPU-runnable, ONNX-friendly,
accent-preserving** zero-shot TTS engines that didn't exist (or weren't permissive)
when we last looked.

**Do a time-boxed spike** — exactly like the Phase-2a OpenVoice spike that de-risked
v1.20.0 before any product build:

1. **Prototype VoxCPM-0.5B first** (best accent claim + Apache code & weights) and
   **Chatterbox second** (cleanest license + a ready CPU ONNX export), in Python,
   using the same non-US/UK reference clip(s) (incl. `me.wav`).
2. **Measure the three things that decide it:**
   - **Accent retention** by ear vs. current Kokoro+OpenVoice (the whole point).
   - **Seconds per utterance on a real mid-range Windows x86 CPU** (the cited RTFs
     are from a budget Intel i3 quantized / an Apple M4 — x86 could be 2–4× slower;
     batch Speak can tolerate ~5–10 s, but verify it's not 30 s+).
   - **Total download size** (model + any deps; target < ~1 GB on-demand).
3. **Deliverable: audition clips + a go/no-go**, same as Phase-2a. If accent isn't
   clearly better or CPU latency is intolerable, stop — OpenVoice timbre-transfer
   stays the shipped path.

### What this would cost / risk if greenlit

- **Architectural weight.** These are full TTS engines (0.5–1 B params), so they
  **replace Kokoro for cloned voices** rather than bolt onto it — a much bigger
  change than v1.20–v1.23. Preset voices would stay on Kokoro; cloned voices would
  route to the new engine. Two TTS stacks to maintain.
- **ONNX integration is the real work.** The 0.5B exports are community-maintained;
  AR/diffusion decode loops (token loop, flow/ODE steps) must be orchestrated in
  Rust over `ort`. Budget a **multi-week port**, not a drop-in. (GPT-SoVITS's only
  Rust binding uses libtorch, not `ort` — a tell for how much glue these need.)
- **Size + first-run UX.** A ~0.5–1 GB download-on-demand is ~3–6× the current
  cloning download. Tolerable, but a real step up.
- **Watermark / data-provenance** caveats per engine (e.g. Chatterbox's Perth
  watermark).

**Net:** the question "can a user clone their own *accent*, on-device, MIT/Apache,
on CPU?" has moved from **"no, blocked"** to **"plausibly yes — spike VoxCPM and
Chatterbox to confirm quality + CPU latency before committing."** Keep OpenVoice as
the fallback regardless.

## Spike results (2026-06-14) — VoxCPM-0.5B vs Chatterbox on `me.wav`

Ran both engines' reference Python impls on **CPU** (torch 2.x+cpu, no GPU),
cloning the user's voice (`me.wav`, a male US-English reading) on two sentences.
Audition clips live in `src-tauri/resources/_spike/accent/out/` (gitignored).

| Metric | VoxCPM-0.5B | Chatterbox |
|---|---|---|
| License (code / weights) | **Apache / Apache** | **MIT / MIT** |
| Model download | **1.5 GB** | **3.0 GB** |
| Load (one-time) | ~18 s | ~22 s |
| **CPU RTF (cloning)** | **~3.0** (4.6 s line → 13.6 s) | ~4–6 (4.1 s line → 25.9 s) |
| Timbre cosine vs me.wav (OpenVoice SE) | 0.82–0.85 | 0.83–0.87 |
| Reference transcript needed? | **Yes** (0.5B continuation mode; transcript-free `reference_wav_path` is VoxCPM2/2B-only, ~16 GB) | No (audio prompt only) |
| Watermark | none | non-removable Perth watermark |

Findings:
- **Both clone timbre well** (SE cosine ~0.85, comparable to the Phase-2a
  OpenVoice result ~0.89) and run on CPU with no GPU.
- **VoxCPM is the better candidate**: half the download, ~2× faster on CPU,
  Apache. Caveat: the 0.5B needs the reference *transcript* → an extra ASR step
  in-product (or the user types what they recorded). A wrong/placeholder
  transcript made it ramble (9 s for a 4 s line, RTF ~9); the correct transcript
  fixed both quality and speed (RTF ~3).
- **Chatterbox is simpler** (transcript-free) but 3.0 GB, slower, watermarked.
- **Both are heavy for the product**: 1.5–3 GB download (vs the current 157 MB
  OpenVoice), 13–26 s per sentence on CPU (vs near-instant Kokoro+OpenVoice), and
  would *replace* Kokoro for cloned voices. A real `ort` integration is a
  multi-week port (community ONNX exports exist, but the 0.5B decode loops must be
  wired by hand).

**Accent verdict (the deciding question): pending an ear test** of the audition
clips vs `me.wav`. Objective metrics confirm cloning works and is permissive +
CPU-feasible; whether the *accent* is meaningfully better than today's
timbre-only path is subjective and must be judged by listening. Engineering
take: technically a **GO** (feasible, permissive, on-CPU), but the size/latency/
port cost is real — productize only if the accent gain is clearly worth it.

## Accuracy follow-up: best-of-N reranking (2026-06-16, shipped v1.25.0)

After shipping VoxCPM (v1.24.0), the next accuracy lever was **variance**, not the
model. Empirical sweeps on `me.wav` (per-channel Q8, the shipped graphs):

- **cfg_value**: 2.0 is optimal and stable (mean speaker cosine **0.858** over 4
  seeds; 1.5 → 0.801 unstable, 2.5 → 0.749 with a 0.48 crater). Confirmed
  multi-seed — a single seed had falsely favored 1.5.
- **Per-seed variance dominates.** Even at cfg 2.0 the cosine swings 0.83–0.87,
  and a minority of seeds *ramble* (decode to the length cap). So **best-of-N**
  (generate a few, keep the best) is the dominant lever — bigger than any knob.

Reranking needs a **proper speaker-verification model**, not the OpenVoice
tone-color cosine (it's compressed — all candidates 0.81–0.87 — and **gameable**):
across 8 candidates it rated a 12.5 s rambler **0.866** (near the top), while
**WeSpeaker** ResNet34-LM rated the same take **0.598** (by far the worst).
WeSpeaker sanity: same-speaker **0.966**, different-speaker **0.076** — real range.

Shipped design (`divora-core/src/tts/speaker.rs` + `VoxCpmEngine`): prefill once
(noise-independent), fan out decode+VAE per candidate (~7 s each on CPU), drop any
that hit the decode cap, rerank survivors by WeSpeaker SECS vs the reference, keep
the best. The `fbank` front-end is a from-scratch Kaldi reimplementation
parity-validated against `torchaudio` (Rust↔Python embedding cosine > 0.999).
Exposed as a **Fast / Balanced / Best** (1 / 3 / 6) latency-vs-fidelity control.

## GPU feasibility (2026-06-17) — DirectML is a NO-GO for this export

After the async un-freeze (v1.27.0) the CPU path is responsive but still ~7 s /
take. GPU offload was the obvious next lever, so we spiked **DirectML** (the only
*portable* Windows GPU runtime — works on any DX12 GPU, built into Windows, no
CUDA toolkit), measured on an **RTX 4090** via `onnxruntime-directml`.

**Result: the `bluryar` VoxCPM graphs don't run on DirectML at all.** Both the
shipped per-channel **Q8** and the **fp32** models crash identically:
`RUNTIME_EXCEPTION … Reshape node 'node_view'` (DML's `MLOperatorAuthorImpl`).
Because fp32 fails the same way, it's **architectural** (a reshape the export
emits that DML's operator can't author), not a quantization issue. CPU baseline
on the same machine was ~6 s; DML never completed a single take.

**Verdict: defer GPU.** DirectML can't run it; CUDA could (NVIDIA-only) but isn't
portable for a free app and needs the CUDA toolkit; and the cost is dominated by
the **autoregressive `decode_step` loop** (hundreds of tiny sequential dispatches
+ CPU↔GPU syncs), which is exactly the GPU-overhead-sensitive pattern — so even a
working GPU path would likely give a modest gain, not the 5–10× big-matmul wins.
The real path, if GPU becomes a priority, is a **DirectML-compatible re-export**
(drop/replace the `node_view` reshape — e.g. the `DakeQQ` export) or a CUDA
fast-path for a future VoxCPM2-2B premium tier. The v1.27.0 thread-cap +
async-off-thread changes remain the shipped mitigation.

## Sources

Zero-shot TTS: VoxCPM ([repo](https://github.com/OpenBMB/VoxCPM),
[weights](https://huggingface.co/openbmb/VoxCPM-0.5B),
[paper](https://arxiv.org/abs/2509.24650),
[ONNX](https://github.com/DakeQQ/Text-To-Speech-TTS-ONNX)) ·
Chatterbox ([repo](https://github.com/resemble-ai/chatterbox),
[weights MIT](https://huggingface.co/ResembleAI/chatterbox),
[ONNX](https://huggingface.co/onnx-community/chatterbox-ONNX)) ·
CosyVoice2 ([weights apache-2.0](https://huggingface.co/FunAudioLLM/CosyVoice2-0.5B),
[full ONNX + CPU timings/sizes](https://huggingface.co/ayousanz/cosy-voice3-onnx)) ·
GPT-SoVITS ([repo MIT](https://github.com/RVC-Boss/GPT-SoVITS),
[ONNX writeup](https://medium.com/axinc-ai/gpt-sovits-a-zero-shot-speech-synthesis-model-with-customizable-fine-tuning-e4c72cd75d87),
[RTF 0.53 on M4](https://github.com/RVC-Boss/GPT-SoVITS/issues/2579)) ·
F5-TTS ([NC weights](https://github.com/SWivid/F5-TTS),
[CPU "hopeless"](https://github.com/SWivid/F5-TTS/issues/403)) ·
StyleTTS2 ([MIT code, GPL phonemizer](https://github.com/yl4579/StyleTTS2)) ·
Fish/OpenAudio ([NC weights](https://huggingface.co/fishaudio/openaudio-s1-mini),
[ONNX "not planned"](https://github.com/fishaudio/fish-speech/issues/903)) ·
XTTS-v2 ([CPML non-commercial](https://huggingface.co/coqui/XTTS-v2/blob/main/LICENSE.txt)).

Voice conversion / accent conversion: OpenVoice (timbre-only by design,
[paper](https://arxiv.org/html/2312.01479v4)) · kNN-VC / FreeVC (WavLM-Large
**1.26 GB**, [files](https://huggingface.co/microsoft/wavlm-large/tree/main)) ·
Seed-VC ([GPL-3.0](https://github.com/Plachtaa/seed-vc)) ·
RVC (per-voice training,
[README](https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI/blob/main/docs/en/README.en.md)) ·
Amphion Vevo ([MIT code, NC weights](https://huggingface.co/amphion/Vevo)) ·
VC paradigm survey ([arxiv 2203.12813](https://arxiv.org/pdf/2203.12813)).
