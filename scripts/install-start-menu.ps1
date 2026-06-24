[CmdletBinding()]
param(
    [string]$PortableDir = (Join-Path (Split-Path -Parent $PSScriptRoot) "portable"),

    [string]$StartMenuFolder = "Misku Native Views",

    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

$programsDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$targetDir = Join-Path $programsDir $StartMenuFolder

if ($Uninstall) {
    if (Test-Path -LiteralPath $targetDir) {
        Remove-Item -LiteralPath $targetDir -Recurse -Force
        Write-Host "Removed Start Menu folder: $targetDir"
    }
    else {
        Write-Host "Start Menu folder does not exist: $targetDir"
    }
    exit 0
}

if (-not (Test-Path -LiteralPath $PortableDir)) {
    throw "Portable folder not found: $PortableDir. Run .\scripts\build-portable.ps1 first."
}

$shortcuts = Get-ChildItem -LiteralPath $PortableDir -Filter "*.lnk" -File
if (-not $shortcuts) {
    throw "No .lnk shortcuts found in $PortableDir. Run .\scripts\build-portable.ps1 first."
}

New-Item -ItemType Directory -Force -Path $targetDir | Out-Null

foreach ($shortcut in $shortcuts) {
    $destination = Join-Path $targetDir $shortcut.Name
    Copy-Item -LiteralPath $shortcut.FullName -Destination $destination -Force
    Write-Host "Installed Start Menu shortcut: $destination"
}

Write-Host ""
Write-Host "Done. Open Start and search for one of the app names."
