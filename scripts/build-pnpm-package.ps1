$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "build-env.ps1")

function Test-FileLocked {
    param([Parameter(Mandatory = $true)][string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }

    $stream = $null
    try {
        $stream = [System.IO.File]::Open($Path, 'Open', 'ReadWrite', 'None')
        return $false
    } catch {
        return $true
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$cargo = Get-CargoPath
$targetDir = Join-Path $env:LOCALAPPDATA "misku-native-views\cargo-target"
$portableDir = Join-Path $repoRoot "portable"
$releaseExe = Join-Path $targetDir "release\misku-native-views.exe"
$appsConfig = Join-Path $repoRoot "apps.toml"
$iconsDir = Join-Path $repoRoot "icons"
$portableExe = Join-Path $portableDir "misku-native-views.exe"

if (-not (Test-Path -LiteralPath $appsConfig)) {
    throw "No encontre apps.toml en $repoRoot."
}

New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
$env:CARGO_TARGET_DIR = $targetDir
$env:PATH = "$(Split-Path $cargo);$env:PATH"

Initialize-MsvcBuildEnvironment | Out-Null

Push-Location $repoRoot
try {
    & $cargo build --release -p misku-native-views
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo build fallo con codigo $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $releaseExe)) {
    throw "Cargo termino, pero no encontre $releaseExe."
}

New-Item -ItemType Directory -Path $portableDir -Force | Out-Null

if (Test-FileLocked -Path $portableExe) {
    throw "Cierra portable\misku-native-views.exe antes de empaquetar."
}

Copy-Item -LiteralPath $releaseExe -Destination $portableExe -Force
Copy-Item -LiteralPath $appsConfig -Destination (Join-Path $portableDir "apps.toml") -Force

if (Test-Path -LiteralPath $iconsDir) {
    Copy-Item -LiteralPath $iconsDir -Destination $portableDir -Recurse -Force
}

Write-Output "PackagePortable=$portableDir"
Write-Output "PackageExe=$portableExe"
