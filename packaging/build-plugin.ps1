param(
    [switch]$Install,
    [switch]$UpdateExisting,
    [switch]$SkipBuild,
    [string]$VcRuntimePath
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$pluginRoot = Join-Path $projectRoot 'plugins/cash-ledger'

if (-not $SkipBuild) {
    Push-Location $projectRoot
    try {
        & rtk cargo build --locked --bin cash-ledger-mcp
        if ($LASTEXITCODE -ne 0) { throw 'Plugin build failed.' }
    } finally { Pop-Location }
}
$serverRoot = Join-Path $pluginRoot 'server'
New-Item -ItemType Directory -Path $serverRoot -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $projectRoot 'target/debug/cash-ledger-mcp.exe') -Destination $serverRoot -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/till.png') -Destination (Join-Path $pluginRoot 'assets/till.png') -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $pluginRoot 'LICENSE') -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md') -Destination (Join-Path $pluginRoot 'THIRD_PARTY_NOTICES.md') -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'third-party') -Destination $pluginRoot -Recurse -Force
$noticesScript = Join-Path $PSScriptRoot 'windows/generate-notices.ps1'
$noticesPath = Join-Path $pluginRoot 'THIRD_PARTY_LICENSES.txt'
if ($PSVersionTable.PSVersion.Major -ge 7) {
    & $noticesScript -OutputPath $noticesPath
} else {
    $modernPowerShell = Get-Command pwsh -ErrorAction SilentlyContinue
    if (-not $modernPowerShell) { throw 'PowerShell 7 is required to generate the dependency notices.' }
    & rtk proxy $modernPowerShell.Source -NoProfile -File $noticesScript -OutputPath $noticesPath
    if ($LASTEXITCODE -ne 0) { throw 'Dependency notice generation failed.' }
}
# Use the redistributable SDK copy, as the desktop installer does.
if (-not $VcRuntimePath) {
    $vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Supply -VcRuntimePath with the x64 redistributable vcruntime140.dll.' }
    $visualStudioRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $visualStudioRoot) { throw 'Visual Studio C++ Build Tools were not found.' }
    $redistVersion = (Get-Content -LiteralPath (Join-Path $visualStudioRoot 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt') -Raw).Trim()
    $runtimeCandidates = @(Get-ChildItem -Path (Join-Path $visualStudioRoot "VC/Redist/MSVC/$redistVersion/x64/Microsoft.VC*.CRT/vcruntime140.dll") -File)
    if ($runtimeCandidates.Count -ne 1) { throw 'Cannot identify the x64 redistributable runtime.' }
    $VcRuntimePath = $runtimeCandidates[0].FullName
}
$runtimeBytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $VcRuntimePath).Path)
if ($runtimeBytes.Length -lt 64) { throw 'Invalid runtime DLL.' }
$peOffset = [BitConverter]::ToInt32($runtimeBytes, 60)
if ($peOffset -lt 0 -or $peOffset -gt $runtimeBytes.Length - 6 -or
    [BitConverter]::ToUInt32($runtimeBytes, $peOffset) -ne 0x00004550 -or
    [BitConverter]::ToUInt16($runtimeBytes, $peOffset + 4) -ne 0x8664 -or
    [System.IO.Path]::GetFileName($VcRuntimePath) -ine 'vcruntime140.dll') { throw 'Supply an x64 vcruntime140.dll redistributable.' }
Copy-Item -LiteralPath $VcRuntimePath -Destination $serverRoot -Force
$portable = Get-Content -LiteralPath (Join-Path $pluginRoot 'plugin.json') -Raw | ConvertFrom-Json
$overlay = [ordered]@{
    name = $portable.name
    version = $portable.version
    description = $portable.description
    author = $portable.author
    repository = $portable.repository
    license = $portable.license
    skills = './skills/'
    mcpServers = './.mcp.json'
    interface = $portable.extensions.'com.openai'.interface
}
$utf8 = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText((Join-Path $pluginRoot '.codex-plugin/plugin.json'), ($overlay | ConvertTo-Json -Depth 20), $utf8)
$mcp = Get-Content -LiteralPath (Join-Path $pluginRoot 'mcp.json') -Raw | ConvertFrom-Json
# The compatibility manifest uses the fixed .mcp.json location.
$mcp.mcpServers.'cash-ledger'.command = '${PLUGIN_ROOT}/server/cash-ledger-mcp.exe'
[System.IO.File]::WriteAllText((Join-Path $pluginRoot '.mcp.json'), ($mcp | ConvertTo-Json -Depth 20), $utf8)

& rtk proxy python (Join-Path $PSScriptRoot 'verify-plugin.py') $pluginRoot
if ($LASTEXITCODE -ne 0) { throw 'Plugin package verification failed.' }
$artifactRoot = Join-Path $projectRoot 'dist'
New-Item -ItemType Directory -Path $artifactRoot -Force | Out-Null
$archivePath = Join-Path $artifactRoot "cash-ledger-plugin-$($portable.version).zip"
Add-Type -AssemblyName System.IO.Compression.FileSystem
if (Test-Path -LiteralPath $archivePath) { Remove-Item -LiteralPath $archivePath }
[System.IO.Compression.ZipFile]::CreateFromDirectory($pluginRoot, $archivePath, [System.IO.Compression.CompressionLevel]::Optimal, $true)

if ($Install) {
    $userProfileRoot = [Environment]::GetFolderPath('UserProfile')
    $personalRoot = Join-Path $userProfileRoot 'plugins/cash-ledger'
    if (Test-Path -LiteralPath $personalRoot) {
        if (-not $UpdateExisting) { throw "Plugin source already exists at $personalRoot; use -UpdateExisting to replace this managed package." }
        $existingManifest = Get-Content -LiteralPath (Join-Path $personalRoot 'plugin.json') -Raw | ConvertFrom-Json
        if ($existingManifest.name -ne 'cash-ledger') { throw 'Existing source is not the Cash Ledger package.' }
    }
    $creatorScript = Join-Path $userProfileRoot '.codex/.tmp/plugins/.agents/skills/plugin-creator/scripts/create_basic_plugin.py'
    if (-not (Test-Path -LiteralPath $creatorScript)) { throw 'Codex local plugin authoring helper was not found.' }
    if (-not (Test-Path -LiteralPath $personalRoot)) {
        & rtk proxy python $creatorScript cash-ledger --with-marketplace
        if ($LASTEXITCODE -ne 0) { throw 'Marketplace registration failed.' }
    }
    Copy-Item -Path (Join-Path $pluginRoot '*') -Destination $personalRoot -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $pluginRoot '.codex-plugin') -Destination $personalRoot -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $pluginRoot '.mcp.json') -Destination $personalRoot -Force
    # Windows Codex does not expand plugin-root placeholders in executable names.
    foreach ($configurationName in @('mcp.json', '.mcp.json')) {
        $configurationPath = Join-Path $personalRoot $configurationName
        $configuration = Get-Content -LiteralPath $configurationPath -Raw | ConvertFrom-Json
        $configuration.mcpServers.'cash-ledger'.command = Join-Path $personalRoot 'server/cash-ledger-mcp.exe'
        [System.IO.File]::WriteAllText($configurationPath, ($configuration | ConvertTo-Json -Depth 20), $utf8)
    }
    & rtk proxy codex plugin add cash-ledger@local-plugins --json
    if ($LASTEXITCODE -ne 0) { throw 'Codex plugin installation failed.' }
}
Write-Output "Plugin package ready: $pluginRoot"
Write-Output "Plugin archive: $archivePath"
