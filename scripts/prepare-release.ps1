param(
    [string]$WindowsExe,
    [Parameter(Mandatory = $true)][string]$AndroidApk,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$repository = Split-Path $PSScriptRoot -Parent
$windowsVersion = [regex]::Match((Get-Content -LiteralPath (Join-Path $repository 'campus-tray\Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$androidVersion = [regex]::Match((Get-Content -LiteralPath (Join-Path $repository 'shunet-android\app\build.gradle') -Raw), "versionName\s+'([^']+)'").Groups[1].Value
if (-not $windowsVersion -or -not $androidVersion) { throw 'Cannot read application versions.' }
if (-not $WindowsExe) { $WindowsExe = Join-Path $repository "dist\windows\SHUNet-v$windowsVersion.exe" }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repository 'dist\github' }
foreach ($inputFile in @($WindowsExe, $AndroidApk)) {
    if (-not (Test-Path -LiteralPath $inputFile -PathType Leaf)) { throw "Artifact not found: $inputFile" }
}
$fileVersion = (Get-Item -LiteralPath $WindowsExe).VersionInfo.ProductVersion
if ($fileVersion -ne $windowsVersion) { throw "EXE version $fileVersion does not match source $windowsVersion." }
if ([IO.Path]::GetFileName($AndroidApk) -ne "SHUNet-Android-v$androidVersion-debug.apk") {
    throw 'Expected the versioned debug APK produced by shunet-android/build.ps1.'
}
if (Test-Path -LiteralPath $OutputDirectory) {
    if (Get-ChildItem -LiteralPath $OutputDirectory -Force | Select-Object -First 1) {
        throw 'Output directory is not empty. Choose a fresh directory to avoid mixing releases.'
    }
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$windowsName = "SHUNet-v$windowsVersion.exe"
$androidName = "SHUNet-Android-v$androidVersion-debug.apk"
Copy-Item -LiteralPath $WindowsExe -Destination (Join-Path $OutputDirectory $windowsName)
Copy-Item -LiteralPath $AndroidApk -Destination (Join-Path $OutputDirectory $androidName)
$portable = Join-Path $OutputDirectory "SHUNet-v$windowsVersion-windows-x64"
New-Item -ItemType Directory -Path $portable | Out-Null
Copy-Item -LiteralPath $WindowsExe -Destination (Join-Path $portable 'SHUNet.exe')
Copy-Item -LiteralPath (Join-Path $repository 'LICENSE') -Destination $portable
Copy-Item -LiteralPath (Join-Path $repository 'THIRD_PARTY_NOTICES.md') -Destination $portable
Copy-Item -LiteralPath (Join-Path $repository 'campus-tray\assets\使用说明.txt') -Destination (Join-Path $portable 'README.txt')
Compress-Archive -LiteralPath $portable -DestinationPath (Join-Path $OutputDirectory "$([IO.Path]::GetFileName($portable)).zip")
$windowsFiles = @($windowsName, "$([IO.Path]::GetFileName($portable)).zip")
$windowsHashes = foreach ($name in $windowsFiles) {
    '{0}  {1}' -f (Get-FileHash -LiteralPath (Join-Path $OutputDirectory $name) -Algorithm SHA256).Hash.ToLowerInvariant(), $name
}
$androidHash = '{0}  {1}' -f (Get-FileHash -LiteralPath (Join-Path $OutputDirectory $androidName) -Algorithm SHA256).Hash.ToLowerInvariant(), $androidName
Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS-windows.txt') -Value $windowsHashes -Encoding ascii
Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS-android.txt') -Value $androidHash -Encoding ascii
Write-Host "Prepared Windows v$windowsVersion and Android v$androidVersion (debug preview) in $OutputDirectory"
