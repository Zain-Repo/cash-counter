[CmdletBinding()]
param(
    [string]$IsccPath,
    [string]$VcRuntimePath
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path

if (-not $IsccPath) {
    $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($compiler) { $IsccPath = $compiler.Source }
    else {
        $candidates = @(
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
        )
        $IsccPath = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath)) {
    throw 'Install Inno Setup 6.5 or later, or supply -IsccPath with the full path to ISCC.exe.'
}
$IsccPath = (Resolve-Path -LiteralPath $IsccPath).Path

if (-not $VcRuntimePath) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw 'Visual Studio discovery is unavailable. Supply -VcRuntimePath pointing to the x64 redistributable vcruntime140.dll.'
    }
    $visualStudioRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $visualStudioRoot) { throw 'Unable to locate Visual Studio C++ Build Tools.' }
    $redistVersion = (Get-Content -LiteralPath (Join-Path $visualStudioRoot 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt') -Raw).Trim()
    $runtimeRoot = Join-Path $visualStudioRoot "VC/Redist/MSVC/$redistVersion/x64"
    $runtimeCandidates = @(Get-ChildItem -Path (Join-Path $runtimeRoot 'Microsoft.VC*.CRT/vcruntime140.dll') -File)
    if ($runtimeCandidates.Count -ne 1) { throw 'Unable to identify one default x64 Visual C++ redistributable DLL.' }
    $VcRuntimePath = $runtimeCandidates[0].FullName
}
if (-not (Test-Path -LiteralPath $VcRuntimePath)) {
    throw 'The Visual C++ x64 redistributable vcruntime140.dll is missing. Install the C++ redistributable SDK component or supply -VcRuntimePath.'
}
$VcRuntimePath = (Resolve-Path -LiteralPath $VcRuntimePath).Path
# Reject an x86/ARM runtime before building an installer labeled x64.
$runtimeBytes = [System.IO.File]::ReadAllBytes($VcRuntimePath)
if ($runtimeBytes.Length -lt 64) { throw 'Invalid runtime DLL.' }
$peOffset = [BitConverter]::ToInt32($runtimeBytes, 60)
if ($peOffset -lt 0 -or $peOffset -gt $runtimeBytes.Length - 6 -or
    [BitConverter]::ToUInt32($runtimeBytes, $peOffset) -ne 0x00004550 -or
    [BitConverter]::ToUInt16($runtimeBytes, $peOffset + 4) -ne 0x8664) {
    throw 'VcRuntimePath must reference an x64 Windows runtime DLL.'
}
if ([System.IO.Path]::GetFileName($VcRuntimePath) -ine 'vcruntime140.dll') {
    throw 'VcRuntimePath must reference vcruntime140.dll from the Visual Studio redistributable directory.'
}

Push-Location $projectRoot
try {
    $metadataText = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataText | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'cash-counter' }
    if (-not $package) { throw 'Unable to locate the cash-counter package.' }

    # An explicit target prevents the host architecture from being mislabeled as x64.
    & cargo build --locked --release --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    $buildDir = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release'
    $outputDir = Join-Path $projectRoot 'dist'
    $noticesPath = Join-Path $outputDir 'THIRD_PARTY_LICENSES.txt'
    & (Join-Path $PSScriptRoot 'generate-notices.ps1') -OutputPath $noticesPath
    Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $outputDir 'LICENSE')
    Copy-Item -LiteralPath (Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md') -Destination (Join-Path $outputDir 'THIRD_PARTY_NOTICES.md')
    & $IsccPath "/DAppVersion=$($package.version)" "/DBuildDir=$buildDir" "/DOutputDir=$outputDir" "/DVcRuntimePath=$VcRuntimePath" "/DNoticesPath=$noticesPath" (Join-Path $PSScriptRoot 'till.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $installer = Join-Path $outputDir "CashLedger-$($package.version)-windows-x64-setup.exe"
    if (-not (Test-Path -LiteralPath $installer)) { throw 'Installer output is missing.' }
    Get-FileHash -LiteralPath $installer -Algorithm SHA256
    $releaseFiles = @($installer, (Join-Path $outputDir 'LICENSE'), (Join-Path $outputDir 'THIRD_PARTY_NOTICES.md'), $noticesPath)
    $checksums = foreach ($file in $releaseFiles) {
        $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $([IO.Path]::GetFileName($file))"
    }
    # GNU sha256sum treats Windows CRLF as part of each filename; emit portable LF lines.
    $checksumText = ([string[]]$checksums -join "`n") + "`n"
    [IO.File]::WriteAllText((Join-Path $outputDir 'SHA256SUMS.txt'), $checksumText, [Text.UTF8Encoding]::new($false))
    Write-Output "Installer: $installer"
}
finally { Pop-Location }
