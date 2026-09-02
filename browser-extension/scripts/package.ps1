# Package the browser extension as both a Chrome/Edge/Brave .zip and a
# Firefox .xpi. Windows / PowerShell equivalent of scripts/package.sh —
# the two should stay in sync on the include/exclude list. CI uses the
# bash variant on Ubuntu; this one is for local packaging on Windows.
#
# Usage: .\scripts\package.ps1 -Version 0.1.0
# Output (relative to browser-extension\):
#   dist\dm-grabber-<version>.zip
#   dist\dm-grabber-<version>.xpi

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version
)

$ErrorActionPreference = "Stop"

# Strip leading "v" if present.
$Version = $Version -replace '^v', ''

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ExtDir    = Split-Path -Parent $ScriptDir
$DistDir   = Join-Path $ExtDir "dist"

if (-not (Get-Command zip -ErrorAction SilentlyContinue)) {
    throw "zip.exe not found in PATH. Install it (e.g. via Git for Windows or Cygwin) before packaging."
}

New-Item -ItemType Directory -Path $DistDir -Force | Out-Null
$OutBase = Join-Path $DistDir "dm-grabber-$Version"

# Stage into a temp dir, stripping the same files as package.sh.
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("dm-ext-stage-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Tmp | Out-Null

try {
    Get-ChildItem -Path $ExtDir -Force | Where-Object {
        $name = $_.Name
        $name -ne ".git"          -and
        $name -ne "dist"          -and
        $name -ne "node_modules"  -and
        $name -ne "scripts"       -and
        $name -ne "Thumbs.db"     -and
        $name -notlike "*.log"    -and
        $name -notlike "com.app.dm.native*.json"
    } | ForEach-Object {
        Copy-Item -Path $_.FullName -Destination $Tmp -Recurse -Force
    }

    $ManifestPath = Join-Path $Tmp "manifest.json"
    if (-not (Test-Path $ManifestPath)) {
        throw "manifest.json missing from staged payload"
    }

    $Manifest = Get-Content $ManifestPath -Raw | ConvertFrom-Json
    if ($Manifest.version -ne $Version) {
        throw "manifest.json declares version `"$($Manifest.version)`" but you asked to package `"$Version)`". Bump manifest.json first."
    }
    if ($Manifest.manifest_version -ne 3) {
        throw "manifest_version must be 3"
    }
    if (-not $Manifest.background.scripts) {
        throw "background.scripts must be an array (Firefox 109+ event-page compat)"
    }

    # Pack the .zip from inside the staging dir so the archive paths
    # are relative, which is what Chrome's "Load unpacked" expects.
    Push-Location $Tmp
    try {
        & zip -r -X "$OutBase.zip" . | Out-Null
    } finally {
        Pop-Location
    }

    # .xpi is a renamed .zip.
    Copy-Item "$OutBase.zip" "$OutBase.xpi" -Force

    Write-Host "✔ packaged:"
    Write-Host "    $OutBase.zip"
    Write-Host "    $OutBase.xpi"
} finally {
    Remove-Item -Recurse -Force $Tmp
}
