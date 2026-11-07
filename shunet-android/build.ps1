param(
    [string]$JavaHome,
    [string]$SdkPath,
    [string]$GradleExecutable,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
$projectDirectory = $PSScriptRoot

if (-not $JavaHome) { $JavaHome = $env:JAVA_HOME }
if (-not $JavaHome) {
    $studioRuntime = Join-Path $env:ProgramFiles 'Android\Android Studio\jbr'
    if (Test-Path -LiteralPath (Join-Path $studioRuntime 'bin\java.exe')) { $JavaHome = $studioRuntime }
}
if (-not $JavaHome) {
    $javaCommand = Get-Command java -ErrorAction SilentlyContinue
    if ($javaCommand) { $JavaHome = Split-Path (Split-Path $javaCommand.Source -Parent) -Parent }
}
if (-not $JavaHome -or -not (Test-Path -LiteralPath (Join-Path $JavaHome 'bin\java.exe'))) {
    throw 'JDK 17+ is required. Install Android Studio or pass -JavaHome.'
}
$env:JAVA_HOME = $JavaHome

if (-not $SdkPath) { $SdkPath = $env:ANDROID_HOME }
if (-not $SdkPath) { $SdkPath = $env:ANDROID_SDK_ROOT }
if ($SdkPath) {
    if (-not (Test-Path -LiteralPath (Join-Path $SdkPath 'platforms\android-36\android.jar'))) {
        throw 'Install Android SDK Platform 36 in SDK Manager before building.'
    }
    # Java properties use Unicode escapes for paths outside ASCII.
    $propertyValue = -join ($SdkPath.Replace('\', '/').ToCharArray() | ForEach-Object {
        if ([int]$_ -gt 127) { '\u{0:x4}' -f [int]$_ } else { [string]$_ }
    })
    Set-Content -LiteralPath (Join-Path $projectDirectory 'local.properties') -Value "sdk.dir=$propertyValue" -Encoding ascii
    $env:ANDROID_HOME = $SdkPath
}
if (-not $SdkPath -and -not (Test-Path -LiteralPath (Join-Path $projectDirectory 'local.properties'))) {
    throw 'Android SDK is required. Open this folder in Android Studio, or pass -SdkPath.'
}

Push-Location -LiteralPath $projectDirectory
try {
    $gradleArguments = @('--no-daemon', '--console', 'plain')
    if ($Offline) { $gradleArguments += '--offline' }
    $gradleArguments += ':app:assembleDebug'
    if ($GradleExecutable) {
        & $GradleExecutable @gradleArguments
    } else {
        & (Join-Path $projectDirectory 'gradlew.bat') @gradleArguments
    }
    if ($LASTEXITCODE -ne 0) { throw 'Android build failed. See the Gradle output above.' }
    $buildConfig = Get-Content -LiteralPath (Join-Path $projectDirectory 'app\build.gradle') -Raw
    $version = [regex]::Match($buildConfig, "versionName\s+'([^']+)'").Groups[1].Value
    if (-not $version) { throw 'Cannot read application version.' }
    $outputDirectory = Join-Path $projectDirectory 'dist'
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    $apk = Join-Path $outputDirectory "SHUNet-Android-v$version-debug.apk"
    Copy-Item -LiteralPath (Join-Path $projectDirectory 'app\build\outputs\apk\debug\app-debug.apk') -Destination $apk -Force
    $hash = (Get-FileHash -LiteralPath $apk -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath ($apk + '.sha256') -Value "$hash  $([IO.Path]::GetFileName($apk))" -Encoding ascii
    Write-Host "APK: $apk"
    Write-Host "SHA-256: $hash"
} finally {
    Pop-Location
}
