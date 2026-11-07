param(
    [string]$Repository = 'yanhuigang173/SHUNet',
    [string]$GitHubCli = 'gh',
    [string]$ArtifactsDirectory
)
$ErrorActionPreference = 'Stop'
$rootDirectory = Split-Path $PSScriptRoot -Parent
if (-not $ArtifactsDirectory) { $ArtifactsDirectory = Join-Path $rootDirectory 'dist\github' }
Push-Location -LiteralPath $rootDirectory
try {
    & $GitHubCli auth status --hostname github.com
    if ($LASTEXITCODE -ne 0) { throw 'Run gh auth login --hostname github.com --git-protocol https --web first.' }
    $login = & $GitHubCli api user --jq .login
    if ($LASTEXITCODE -ne 0 -or $login.Trim() -ne $Repository.Split('/')[0]) { throw 'Authenticated GitHub account must match the target owner.' }
    $windowsVersion = [regex]::Match((Get-Content -LiteralPath 'campus-tray\Cargo.toml' -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    $androidVersion = [regex]::Match((Get-Content -LiteralPath 'shunet-android\app\build.gradle' -Raw), "versionName\s+'([^']+)'").Groups[1].Value
    if ((& git status --porcelain)) { throw 'Commit reviewed source files before publishing.' }
    $assets = @("SHUNet-v$windowsVersion.exe", "SHUNet-v$windowsVersion-windows-x64.zip", "SHUNet-Android-v$androidVersion-debug.apk", 'SHA256SUMS-windows.txt', 'SHA256SUMS-android.txt')
    foreach ($asset in $assets) {
        if (-not (Test-Path -LiteralPath (Join-Path $ArtifactsDirectory $asset))) { throw "Missing release artifact: $asset" }
    }
    $windowsNotes = "docs\release-notes-v$windowsVersion.md"
    $androidNotes = "docs\release-notes-android-v$androidVersion.md"
    foreach ($notes in @($windowsNotes, $androidNotes)) {
        if (-not (Test-Path -LiteralPath $notes)) { throw "Missing release notes: $notes" }
    }
    # A new repository is created deliberately. Existing non-empty repositories are never overwritten.
    $existing = & $GitHubCli repo view $Repository --json isEmpty 2>$null
    if ($LASTEXITCODE -eq 0) {
        if (-not ($existing | ConvertFrom-Json).isEmpty) { throw 'Target repository is not empty. Publish updates manually after reviewing its history.' }
    } else {
        & $GitHubCli repo create $Repository --public --description 'SHUNet: 上海大学校园网工具，Windows 原生托盘版与 Android 原生精简版。'
        if ($LASTEXITCODE -ne 0) { throw 'Repository creation failed.' }
    }
    $origin = & git remote get-url origin 2>$null
    $expectedOrigin = "https://github.com/$Repository.git"
    if ($LASTEXITCODE -ne 0) {
        & git remote add origin $expectedOrigin
    } elseif ($origin.Trim() -ne $expectedOrigin) {
        throw "Unexpected origin: $origin"
    }
    # Scope Git authentication to this repository and the account verified above.
    $cliPath = (Get-Command $GitHubCli -ErrorAction Stop).Source.Replace('\', '/')
    $gitHelper = '!"' + $cliPath + '" auth git-credential'
    & git config --local --replace-all credential.https://github.com.helper ''
    if ($LASTEXITCODE -ne 0) { throw 'Cannot configure repository Git credentials.' }
    & git config --local --add credential.https://github.com.helper $gitHelper
    if ($LASTEXITCODE -ne 0) { throw 'Cannot configure GitHub CLI credential helper.' }
    & git push -u origin main
    if ($LASTEXITCODE -ne 0) { throw 'Git push failed.' }
    foreach ($tag in @("v$windowsVersion", "android-v$androidVersion")) {
        $existingTag = & git tag --list $tag
        if (-not $existingTag) {
            & git tag -a $tag -m "SHUNet $tag"
            if ($LASTEXITCODE -ne 0) { throw "Cannot create tag $tag" }
        }
        & git push origin $tag
        if ($LASTEXITCODE -ne 0) { throw "Cannot push tag $tag" }
    }
    & $GitHubCli release create "android-v$androidVersion" (Join-Path $ArtifactsDirectory "SHUNet-Android-v$androidVersion-debug.apk") (Join-Path $ArtifactsDirectory 'SHA256SUMS-android.txt') --repo $Repository --verify-tag --prerelease --latest=false --title "SHUNet Android v$androidVersion (Debug preview)" --notes-file $androidNotes
    if ($LASTEXITCODE -ne 0) { throw 'Android release failed. Do not recreate successful releases blindly.' }
    & $GitHubCli release create "v$windowsVersion" (Join-Path $ArtifactsDirectory "SHUNet-v$windowsVersion.exe") (Join-Path $ArtifactsDirectory "SHUNet-v$windowsVersion-windows-x64.zip") (Join-Path $ArtifactsDirectory 'SHA256SUMS-windows.txt') --repo $Repository --verify-tag --latest --title "SHUNet v$windowsVersion - Windows" --notes-file $windowsNotes
    if ($LASTEXITCODE -ne 0) { throw 'Windows release failed. Check existing releases before retrying.' }
    & $GitHubCli release view "v$windowsVersion" --repo $Repository
} finally { Pop-Location }
