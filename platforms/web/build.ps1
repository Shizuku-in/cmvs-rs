param([switch]$Debug)
$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Push-Location $root
try {
    $profile = if ($Debug) { 'debug' } else { 'release' }
    $arguments = @('build', '--locked', '-p', 'cmvs-web', '--target', 'wasm32-unknown-unknown')
    if (!$Debug) { $arguments += '--release' }
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Wasm build failed' }
    $localTool = Join-Path $root 'target/wasm-tools/bin/wasm-bindgen.exe'
    $tool = if (Test-Path -LiteralPath $localTool) { $localTool } else { 'wasm-bindgen' }
    & $tool "target/wasm32-unknown-unknown/$profile/cmvs_web.wasm" --target web --out-dir platforms/web/pkg
    if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed; install CLI version 0.2.130' }
} finally { Pop-Location }
