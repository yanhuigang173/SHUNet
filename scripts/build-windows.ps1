param(
    [string]$TargetDirectory = (Join-Path $env:TEMP 'shunet-build-target'),
    [switch]$SkipTests,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$manifest = Join-Path $repository 'campus-tray\Cargo.toml'
foreach ($tool in @('cargo', 'gcc', 'windres')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool is required. Add Rust GNU and MinGW-w64 bin directories to PATH."
    }
}
$cargoArguments = @('--locked', '--manifest-path', $manifest, '--target', 'x86_64-pc-windows-gnu', '--target-dir', $TargetDirectory)
if ($Offline) { $cargoArguments += '--offline' }
if (-not $SkipTests) {
    & cargo test @cargoArguments
    if ($LASTEXITCODE -ne 0) { throw 'Windows tests failed.' }
}
& cargo build --release @cargoArguments
if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }
$version = [regex]::Match((Get-Content -LiteralPath $manifest -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Cannot read Windows version.' }
$output = Join-Path $repository 'dist\windows'
New-Item -ItemType Directory -Path $output -Force | Out-Null
$builtExe = Join-Path $TargetDirectory 'x86_64-pc-windows-gnu\release\shunet.exe'
Copy-Item -LiteralPath $builtExe -Destination (Join-Path $output "SHUNet-v$version.exe") -Force
Write-Host "Windows v$version built in $output"
