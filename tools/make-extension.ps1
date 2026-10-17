# Generates extension/engine.js from the built wasm, so the extension carries the
# same engine the page carries. Rebuild the wasm first:
#   cargo build -p lab-web --target wasm32-unknown-unknown --release
$root = Split-Path -Parent $PSScriptRoot
$wasm = Join-Path $root "target\wasm32-unknown-unknown\release\lab_web.wasm"
if (-not (Test-Path -LiteralPath $wasm)) {
    throw "wasm not found at $wasm; build it first"
}
$b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($wasm))
$js = "globalThis.__LABRONIFY_WASM_B64 = `"$b64`";"
Set-Content -LiteralPath (Join-Path $root "extension\engine.js") -Value $js -NoNewline -Encoding ascii
Copy-Item -LiteralPath (Join-Path $root "crates\lab-core\assets\lebron-target-256.png") -Destination (Join-Path $root "extension\lebron.png") -Force
"wrote extension/engine.js ($([Math]::Round($b64.Length / 1024)) KB base64) and extension/lebron.png"
