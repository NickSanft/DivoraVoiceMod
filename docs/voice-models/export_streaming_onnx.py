"""Export LLVC's streaming forward to ONNX with cache tensors threaded as
named in/out, then validate ONNX vs PyTorch over a multi-chunk streaming
run. Prints everything the Rust cache-threading needs.

Usage: python export_streaming_onnx.py [out.onnx]
"""
import json
import sys

import numpy as np
import torch

from model import Net

CFG = "experiments/llvc/config.json"
CKPT = "llvc_models/models/checkpoints/llvc/G_500000.pth"
OUT = sys.argv[1] if len(sys.argv) > 1 else "llvc-narrator-streaming.onnx"

cfg = json.load(open(CFG))
net = Net(**cfg["model_params"]).eval()
net.load_state_dict(torch.load(CKPT, map_location="cpu")["model"])

L = net.L
CHUNK = net.dec_chunk_size * L  # 208
IN_LEN = CHUNK + 2 * L  # 240
dev = torch.device("cpu")


def fresh_buffers():
    return net.init_buffers(1, dev)


# --- init_buffers all-zero? (determines Rust cache init) ---
eb0, db0, ob0 = fresh_buffers()
for name, t in [("enc_buf", eb0), ("dec_buf", db0), ("out_buf", ob0)]:
    nz = int((t != 0).sum().item())
    print(f"init {name}: shape={tuple(t.shape)} all_zero={nz == 0} nonzero={nz}")

# --- convnet_pre_ctx shape (probe with None) + zeros-vs-None check ---
x_probe = torch.randn(1, 1, IN_LEN) * 0.1
with torch.no_grad():
    o_none, _, _, _, cpc = net(x_probe, *fresh_buffers(), None, pad=False)
cpc_zero = torch.zeros_like(cpc)
print(f"convnet_pre_ctx: shape={tuple(cpc.shape)} all_zero={int((cpc!=0).sum())==0}")
with torch.no_grad():
    o_zero = net(x_probe, *fresh_buffers(), cpc_zero, pad=False)[0]
print("chunk1 max|diff| None-vs-zeros convnet_pre_ctx:",
      (o_none - o_zero).abs().max().item())


class Wrapper(torch.nn.Module):
    def __init__(self, net):
        super().__init__()
        self.net = net

    def forward(self, audio, enc_buf, dec_buf, out_buf, convnet_pre_ctx):
        return self.net(audio, enc_buf, dec_buf, out_buf, convnet_pre_ctx, pad=False)


wrapper = Wrapper(net).eval()
eb, db, ob = fresh_buffers()
dummy = (torch.zeros(1, 1, IN_LEN), eb, db, ob, cpc_zero)

torch.onnx.export(
    wrapper, dummy, OUT,
    input_names=["audio", "enc_buf", "dec_buf", "out_buf", "convnet_pre_ctx"],
    output_names=["output", "enc_buf_out", "dec_buf_out", "out_buf_out",
                  "convnet_pre_ctx_out"],
    opset_version=17,
    dynamo=False,
    do_constant_folding=True,
)
print("exported:", OUT)

# --- validate ONNX vs torch over a streaming run ---
import onnxruntime as ort  # noqa: E402

sess = ort.InferenceSession(OUT, providers=["CPUExecutionProvider"])


def torch_stream(chunks):
    eb, db, ob = fresh_buffers()
    cpc = cpc_zero.clone()
    prev = torch.zeros(2 * L)
    outs = []
    for c in chunks:
        x = torch.cat([prev, c]).reshape(1, 1, -1)
        with torch.no_grad():
            o, eb, db, ob, cpc = net(x, eb, db, ob, cpc, pad=False)
        outs.append(o.reshape(-1).detach().numpy())
        prev = c[-2 * L:]
    return np.concatenate(outs)


def onnx_stream(chunks):
    eb = eb0.detach().numpy().copy(); db = db0.detach().numpy().copy()
    ob = ob0.detach().numpy().copy()
    cpc = cpc_zero.detach().numpy().copy()
    prev = np.zeros(2 * L, np.float32)
    outs = []
    for c in chunks:
        x = np.concatenate([prev, c]).reshape(1, 1, -1).astype(np.float32)
        o, eb, db, ob, cpc = sess.run(
            None,
            {"audio": x, "enc_buf": eb, "dec_buf": db, "out_buf": ob,
             "convnet_pre_ctx": cpc},
        )
        outs.append(o.reshape(-1))
        prev = c[-2 * L:].astype(np.float32)
    return np.concatenate(outs)


chunks = [torch.randn(CHUNK) * 0.1 for _ in range(8)]
ref = torch_stream(chunks)
got = onnx_stream([c.numpy().astype(np.float32) for c in chunks])
print("STREAMING max|diff| ONNX-vs-torch:", float(np.max(np.abs(ref - got))))
print("DONE")
