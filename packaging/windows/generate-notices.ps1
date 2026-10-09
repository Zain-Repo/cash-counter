[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputPath)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$OutputPath = [IO.Path]::GetFullPath($OutputPath)
Push-Location $projectRoot
try {
    $metadataText = & cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Unable to collect locked dependency metadata.' }
    $metadata = $metadataText | ConvertFrom-Json
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add('Cash Ledger - third-party dependency license inventory')
    $lines.Add('Includes Windows runtime, build and test dependencies resolved by Cargo.')
    $lines.Add('License expressions and texts below are supplied by the respective packages.')
    $lines.Add('See THIRD_PARTY_NOTICES.md for embedded assets and Microsoft runtime attribution.')
    foreach ($file in (Get-ChildItem -LiteralPath (Join-Path $projectRoot 'third-party') -File | Sort-Object Name)) {
        $lines.Add('')
        $lines.Add("--- Supplemental upstream license: $($file.Name) ---")
        $lines.Add([IO.File]::ReadAllText($file.FullName))
    }

    # Inventory crate-supplied notices, including nested asset licenses, without generated paths.
    foreach ($package in ($metadata.packages | Where-Object { $_.name -ne 'cash-counter' } | Sort-Object name, version)) {
        $lines.Add('')
        $lines.Add(('=' * 72))
        $lines.Add("$($package.name) $($package.version)")
        $lines.Add("License: $($package.license)")
        $lines.Add("Authors: $($package.authors -join ', ')")
        $lines.Add("Repository: $($package.repository)")
        $lines.Add("Package: https://crates.io/crates/$($package.name)/$($package.version)")
        $crateRoot = Split-Path -Parent $package.manifest_path
        $licenseFiles = @(Get-ChildItem -LiteralPath $crateRoot -Recurse -File |
            Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)([._-].*)?$' })
        if ($package.license_file) {
            $declaredFile = Join-Path $crateRoot $package.license_file
            if (Test-Path -LiteralPath $declaredFile) { $licenseFiles += Get-Item -LiteralPath $declaredFile }
        }
        foreach ($file in ($licenseFiles | Sort-Object FullName -Unique)) {
            $relative = [IO.Path]::GetRelativePath($crateRoot, $file.FullName)
            $lines.Add('')
            $lines.Add("--- $relative ---")
            $lines.Add([IO.File]::ReadAllText($file.FullName))
        }
        if (-not $licenseFiles.Count) {
            $lines.Add('No separate license text was included in this crate; consult the package source and declared license above.')
        }
    }
    $null = New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutputPath)
    [IO.File]::WriteAllLines($OutputPath, $lines, [Text.UTF8Encoding]::new($false))
    Write-Output "Dependency notices: $OutputPath"
}
finally { Pop-Location }
