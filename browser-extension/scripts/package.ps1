# Package the browser extension as a Chrome/Edge/Brave .zip and a
# Firefox .xpi. Windows / PowerShell equivalent of scripts/package.sh —
# the two should stay in sync on the include/exclude list and the
# Chrome-vs-Firefox manifest rewrite. CI uses the bash variant on
# Ubuntu; this one is for local packaging on Windows.
#
# The payload is identical, but the manifest.json at the root is
# rewritten to match the target's MV3 background-page model:
#   * Chrome / Edge / Brave / Arc — `background.service_worker`
#     is the ONLY valid MV3 key. The previous source manifest used
#     `background.scripts`, which Chrome's MV3 parser strictly
#     rejects with `'background.scripts' requires manifest version
#     of 2 or lower` — the extension silently failed to install.
#   * Firefox 109+ — reads `background.scripts` as a non-persistent
#     event page. `background.service_worker` (added in Firefox
#     121) is also accepted, but the
#     `browser_specific_settings.gecko` block forces event-page
#     mode regardless, so we keep the scripts-array form for
#     maximum compatibility.
#
# Usage: .\scripts\package.ps1 -Version 0.1.0
# Output (relative to browser-extension\):
#   dist\dm-grabber-<version>.zip     (Chrome-shaped manifest)
#   dist\dm-grabber-<version>.xpi     (Firefox-shaped manifest)

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

    $SrcManifest = Join-Path $ExtDir "manifest.json"
    $ManifestPath = Join-Path $Tmp "manifest.json"
    if (-not (Test-Path $ManifestPath)) {
        throw "manifest.json missing from staged payload"
    }

    # Validate the *source* manifest as the Firefox template. This
    # is what devs sideload on Firefox 109+ without running the
    # package script, so the source has to be a valid Firefox
    # manifest on its own.
    $SrcContent = Get-Content $SrcManifest -Raw | ConvertFrom-Json
    if ($SrcContent.version -ne $Version) {
        throw "manifest.json declares version `"$($SrcContent.version)`" but you asked to package `"$Version)`". Bump manifest.json first."
    }
    if ($SrcContent.manifest_version -ne 3) {
        throw "manifest_version must be 3"
    }
    if (-not $SrcContent.background.scripts) {
        throw "source manifest.background.scripts must be an array (Firefox template)"
    }
    if ($SrcContent.background.service_worker) {
        throw "source manifest.background.service_worker breaks Firefox 109+ event-page model; restore scripts for the Firefox template"
    }
    if (-not $SrcContent.browser_specific_settings.gecko.id) {
        throw "source manifest missing browser_specific_settings.gecko.id (Firefox template)"
    }

    # Generate the Chrome-shaped manifest in place of the staged one.
    # Chrome MV3 strictly rejects `background.scripts`. We rewrite
    # to `background.service_worker: "background.js"` and drop the
    # `browser_specific_settings.gecko` block (Chrome ignores it
    # but logs a warning; better to omit entirely).
    $Chrome = [ordered]@{}
    foreach ($prop in $SrcContent.PSObject.Properties) {
        if ($prop.Name -eq 'background') {
            $Chrome[$prop.Name] = [ordered]@{ service_worker = "background.js" }
        } elseif ($prop.Name -ne 'browser_specific_settings') {
            $Chrome[$prop.Name] = $prop.Value
        }
    }
    $ChromeJson = $Chrome | ConvertTo-Json -Depth 10
    Set-Content -Path $ManifestPath -Value $ChromeJson -Encoding UTF8

    # Pack the .zip (Chrome / Edge / Brave / Arc).
    Push-Location $Tmp
    try {
        & zip -r -X "$OutBase.zip" . | Out-Null
    } finally {
        Pop-Location
    }

    # Re-stage the *source* manifest (Firefox-shaped) for the .xpi.
    # A .xpi is just a renamed .zip. Firefox is stricter than Chrome
    # about which files are allowed (no top-level __MACOSX/, no
    # .DS_Store, no symlinks). The exclude list above handles the
    # first two; `zip -r` skips symlinks by default.
    Copy-Item -Path $SrcManifest -Destination $ManifestPath -Force
    Push-Location $Tmp
    try {
        & zip -r -X "$OutBase.xpi" . | Out-Null
    } finally {
        Pop-Location
    }

    # Self-verify: open the .zip and confirm the Chrome-shaped
    # manifest has `service_worker` (and no `scripts`); open the
    # .xpi and confirm the Firefox-shaped manifest has `scripts`
    # and `gecko.id`. This catches the v0.4.1 regression where
    # Chrome silently failed to install.
    if (Get-Command unzip -ErrorAction SilentlyContinue) {
        $ZipManifest = & unzip -p "$OutBase.zip" manifest.json 2>$null
        $XpiManifest = & unzip -p "$OutBase.xpi" manifest.json 2>$null
        if (-not $ZipManifest) { throw ".zip is missing manifest.json" }
        if (-not $XpiManifest) { throw ".xpi is missing manifest.json" }

        $ZipObj = $ZipManifest | ConvertFrom-Json
        $XpiObj = $XpiManifest | ConvertFrom-Json

        if (-not $ZipObj.background.service_worker) {
            throw ".zip manifest.background.service_worker is missing (Chrome requires it under MV3)"
        }
        if ($ZipObj.background.scripts) {
            throw ".zip manifest must NOT have background.scripts (Chrome MV3 rejects it)"
        }
        if ($ZipObj.browser_specific_settings.gecko) {
            throw ".zip manifest must NOT have browser_specific_settings.gecko (Chrome logs a warning)"
        }
        if (-not $XpiObj.background.scripts) {
            throw ".xpi manifest.background.scripts is missing (Firefox 109+ requires this)"
        }
        if ($XpiObj.background.service_worker) {
            throw ".xpi manifest must NOT have background.service_worker (Firefox 109+ event-page model expects scripts)"
        }
        if (-not $XpiObj.browser_specific_settings.gecko.id) {
            throw ".xpi manifest missing browser_specific_settings.gecko.id"
        }
    } else {
        Write-Warning "'unzip' not installed; skipping manifest self-verify"
    }

    Write-Host "✔ packaged:"
    Write-Host "    $OutBase.zip   (Chrome-shaped: background.service_worker)"
    Write-Host "    $OutBase.xpi   (Firefox-shaped: background.scripts + gecko.id)"
} finally {
    Remove-Item -Recurse -Force $Tmp
}
