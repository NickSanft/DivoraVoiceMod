# Fetch build-time voice-conversion assets into src-tauri/resources/ so
# `tauri build` can bundle them into the installer. These binaries are
# NOT stored in git — they live on the `voice-assets-v1` GitHub release
# (see docs/voice-models/). Run this before a full `pnpm tauri build`.
#
#   pwsh scripts/fetch-voice-assets.ps1
#
# In CI, `gh` authenticates via the GH_TOKEN env var. Locally it uses
# your `gh auth login` session.

$ErrorActionPreference = "Stop"

# voice-assets-v2 hosts the STREAMING LLVC narrator (v1.3.0). v1 kept the
# non-streaming model so older tags still build against their own asset.
$repo = "NickSanft/DivoraVoiceMod"
$tag = "voice-assets-v2"

$root = Split-Path -Parent $PSScriptRoot
$res = Join-Path $root "src-tauri\resources"
$voices = Join-Path $res "voices"
New-Item -ItemType Directory -Force -Path $voices | Out-Null

$dll = Join-Path $res "onnxruntime.dll"
$model = Join-Path $voices "llvc-narrator.onnx"

Write-Host "Fetching voice assets from $repo@$tag ..."
gh release download $tag --repo $repo --pattern "onnxruntime.dll" --output $dll --clobber
gh release download $tag --repo $repo --pattern "llvc-narrator.onnx" --output $model --clobber

if (-not (Test-Path $dll)) { throw "missing $dll after fetch" }
if (-not (Test-Path $model)) { throw "missing $model after fetch" }
Write-Host "Voice assets ready:"
Write-Host "  $dll ($([math]::Round((Get-Item $dll).Length/1MB,1)) MB)"
Write-Host "  $model ($([math]::Round((Get-Item $model).Length/1MB,1)) MB)"
