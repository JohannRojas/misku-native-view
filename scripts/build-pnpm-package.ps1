param(
    [string] $TargetTriple = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot "build-env.ps1")

function Get-Sha256 {
    param([Parameter(Mandatory = $true)][string] $Path)

    $stream = [System.IO.File]::OpenRead($Path)
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $algorithm.ComputeHash($stream)
        return ([System.BitConverter]::ToString($bytes)).Replace("-", "").ToLowerInvariant()
    } finally {
        $algorithm.Dispose()
        $stream.Dispose()
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
$cargo = Get-CargoPath
$targetDir = if ($env:CARGO_TARGET_DIR) { [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) } else { Join-Path $repoRoot "target\pnpm-package" }
$releaseExe = Join-Path $targetDir "$TargetTriple\release\misku-native-views.exe"
$runtimeDir = Join-Path $repoRoot "runtime"
$runtimeExe = Join-Path $runtimeDir "misku-native-views.exe"
$packageJsonPath = Join-Path $repoRoot "package.json"

if (-not (Test-Path -LiteralPath $packageJsonPath -PathType Leaf)) {
    throw "No encontre package.json en $repoRoot."
}

$packageJson = Get-Content -LiteralPath $packageJsonPath -Raw | ConvertFrom-Json
$env:CARGO_TARGET_DIR = $targetDir
$env:PATH = "$(Split-Path -Parent $cargo);$env:PATH"

Initialize-MsvcBuildEnvironment | Out-Null

Push-Location $repoRoot
try {
    $metadataText = (& $cargo metadata --locked --no-deps --format-version 1) -join "`n"
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata --locked fallo con codigo $LASTEXITCODE."
    }

    $metadata = $metadataText | ConvertFrom-Json
    $nativePackage = @($metadata.packages) |
        Where-Object { $_.name -eq "misku-native-views" } |
        Select-Object -First 1
    if ($null -eq $nativePackage) {
        throw "No encontre el paquete Rust misku-native-views en el workspace."
    }
    if ([string] $nativePackage.version -ne [string] $packageJson.version) {
        throw "Las versiones no coinciden: package.json=$($packageJson.version), Cargo.toml=$($nativePackage.version)."
    }

    if ($env:MISKU_SKIP_NATIVE_BUILD -ne "1") {
    & $cargo build `
        --locked `
        --release `
        --target $TargetTriple `
        --package misku-native-views
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --locked fallo con codigo $LASTEXITCODE."
    }
    }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $releaseExe -PathType Leaf)) {
    throw "Cargo termino, pero no encontre el ejecutable esperado: $releaseExe."
}
$nativeVersion = (& node (Join-Path $PSScriptRoot 'read-native-version.cjs') $releaseExe) -join "`n"
if ($LASTEXITCODE -ne 0 -or $nativeVersion.Trim() -ne "misku-nv $($packageJson.version)") {
    throw "El runtime no corresponde a la version del paquete: $nativeVersion"
}

New-Item -ItemType Directory -Path $runtimeDir -Force | Out-Null
[System.IO.File]::Copy($releaseExe, $runtimeExe, $true)

if ($env:MISKU_SKIP_NATIVE_BUILD -eq '1') {
    if ($null -ne (Get-AuthenticodeSignature -LiteralPath $runtimeExe).SignerCertificate) {
        throw 'Un runtime firmado debe extraerse del instalador; no se modifica su firma.'
    }
    & node (Join-Path $PSScriptRoot 'match-nsis-runtime.cjs') $runtimeExe
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo igualar la metadata NSIS del runtime' }
}

$runtimeHash = Get-Sha256 -Path $runtimeExe
Write-Output "PackageRuntime=$runtimeExe"
Write-Output "PackageRuntimeSha256=$runtimeHash"
Write-Output "PackageTarget=$TargetTriple"
