# Fetch build-time voice assets into src-tauri/resources/ so `tauri build`
# can bundle them into the installer. This covers BOTH the AI voice-conversion
# assets (onnxruntime.dll + LLVC narrator) and the v1.17.0 text-to-speech
# "Speak" assets (Kokoro-82M + espeak-ng). These binaries are NOT stored in
# git — they live on the `voice-assets-v2` GitHub release. Run this before a
# full `pnpm tauri build`.
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

# v1.17.0: text-to-speech ("Speak") assets — Kokoro-82M (Apache-2.0) for
# synthesis + espeak-ng (GPL-3.0) for phonemization, the latter bundled as a
# separate arm's-length CLI (its exe + dll + data). Fetched into
# resources/tts/ and mapped by tauri.bundle.conf.json to <resource>/tts/.
$tts = Join-Path $res "tts"
New-Item -ItemType Directory -Force -Path $tts | Out-Null

$ttsFiles = @(
  "kokoro-v1.0.int8.onnx",     # Kokoro int8 model (~88 MB)
  "voices-divora.bin",         # compact DVTS style pack (preset voices)
  "kokoro-config.json",        # model config (holds the phoneme vocab)
  "espeak-ng.exe",             # espeak-ng CLI (GPL-3.0)
  "libespeak-ng.dll",          # espeak-ng runtime lib
  "openvoice-extractor.onnx",  # v1.20.0: OpenVoice tone-color extractor (MIT)
  "openvoice-converter.onnx"   # v1.20.0: OpenVoice tone-color converter (~157 MB)
)
foreach ($f in $ttsFiles) {
  $dest = Join-Path $tts $f
  gh release download $tag --repo $repo --pattern $f --output $dest --clobber
  if (-not (Test-Path $dest)) { throw "missing $dest after fetch" }
}

# espeak-ng-data is a directory (~130 files); it ships zipped. Unzip into tts/.
$zip = Join-Path $tts "espeak-ng-data.zip"
gh release download $tag --repo $repo --pattern "espeak-ng-data.zip" --output $zip --clobber
if (Test-Path (Join-Path $tts "espeak-ng-data")) {
  Remove-Item -Recurse -Force (Join-Path $tts "espeak-ng-data")
}
Expand-Archive -Path $zip -DestinationPath $tts -Force
Remove-Item -Force $zip
if (-not (Test-Path (Join-Path $tts "espeak-ng-data\phontab"))) {
  throw "espeak-ng-data not unzipped into $tts"
}
Write-Host "TTS assets ready in $tts (Kokoro + espeak-ng)"
