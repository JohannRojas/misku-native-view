$ErrorActionPreference = "Stop"

$CargoArgs = @($args)

$repoRoot = Split-Path -Parent $PSScriptRoot
$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
$vsInstallPath = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
$devShell = Join-Path $vsInstallPath "Common7\Tools\Launch-VsDevShell.ps1"
$targetDir = Join-Path $env:LOCALAPPDATA "misku-native-views\cargo-target"

if (-not (Test-Path $cargo)) {
    throw "No encontre Cargo en $cargo. Instala Rust o agrega Cargo al PATH."
}

if (-not (Test-Path $devShell)) {
    throw "No encontre Visual Studio Build Tools en $vsInstallPath."
}

if ($CargoArgs.Count -eq 0) {
    $CargoArgs = @("run", "-p", "misku-native-views", "--", "--app", "tftacademy")
}

New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
$env:CARGO_TARGET_DIR = $targetDir
$env:PATH = "$(Split-Path $cargo);$env:PATH"

. $devShell -VsInstallationPath $vsInstallPath -Arch amd64 -HostArch amd64

Push-Location $repoRoot
try {
    & $cargo @CargoArgs
} finally {
    Pop-Location
}
