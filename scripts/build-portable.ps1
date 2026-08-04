[CmdletBinding()]
param(
    [switch] $ReleaseOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

function Test-FileLocked {
    param([Parameter(Mandatory = $true)][string] $Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $false
    }

    $stream = $null
    try {
        $stream = [System.IO.File]::Open($Path, 'Open', 'ReadWrite', 'None')
        return $false
    }
    catch {
        return $true
    }
    finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

function Test-ReparsePoint {
    param([Parameter(Mandatory = $true)][string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }

    $item = Get-Item -LiteralPath $Path -Force
    return [bool]($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
}

function Get-DirectChildPath {
    param(
        [Parameter(Mandatory = $true)][string] $Root,
        [Parameter(Mandatory = $true)][string] $Leaf
    )

    if ([string]::IsNullOrWhiteSpace($Leaf) -or
        $Leaf -ne [System.IO.Path]::GetFileName($Leaf) -or
        $Leaf -in @(".", "..")) {
        throw "Nombre de directorio portable invalido: $Leaf"
    }

    $rootPath = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootPath $Leaf))
    $prefix = "$rootPath$([System.IO.Path]::DirectorySeparatorChar)"
    if (-not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Ruta portable fuera del directorio administrado: $candidate"
    }

    return $candidate
}

function Remove-GeneratedUuidDirectory {
    param(
        [Parameter(Mandatory = $true)][string] $PortableRoot,
        [Parameter(Mandatory = $true)][System.IO.DirectoryInfo] $Directory
    )

    $parsed = [System.Guid]::Empty
    if (-not [System.Guid]::TryParse($Directory.Name, [ref]$parsed) -or
        $parsed.ToString("D") -ne $Directory.Name.ToLowerInvariant()) {
        throw "Se rechazo borrar un directorio que no tiene nombre UUID canonico: $($Directory.FullName)"
    }

    $expected = Get-DirectChildPath -Root $PortableRoot -Leaf $Directory.Name
    if (-not $Directory.FullName.Equals($expected, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Se rechazo borrar una ruta portable inesperada: $($Directory.FullName)"
    }
    if (Test-ReparsePoint -Path $Directory.FullName) {
        throw "Se rechazo borrar un enlace o reparse point: $($Directory.FullName)"
    }

    Remove-Item -LiteralPath $Directory.FullName -Recurse -Force
}

function Invoke-NativeJson {
    param(
        [Parameter(Mandatory = $true)][string] $Executable,
        [Parameter(Mandatory = $true)][string[]] $Arguments,
        [Parameter(Mandatory = $true)][string] $Operation
    )

    $lines = & $Executable @Arguments
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "$Operation fallo con codigo $exitCode."
    }

    $json = ($lines -join [Environment]::NewLine).Trim()
    if ([string]::IsNullOrWhiteSpace($json)) {
        throw "$Operation no devolvio JSON."
    }

    try {
        $value = ConvertFrom-Json -InputObject $json
    }
    catch {
        throw "$Operation devolvio JSON invalido: $($_.Exception.Message)"
    }

    return [pscustomobject]@{
        Json = $json
        Value = $value
    }
}

$repoRoot = (Resolve-Path -LiteralPath (Split-Path -Parent $PSScriptRoot)).Path
$appsConfig = (Resolve-Path -LiteralPath (Join-Path $repoRoot "apps.toml")).Path
$portableDir = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "portable"))
$targetDir = Join-Path $env:LOCALAPPDATA "misku-native-views\cargo-target"

. (Join-Path $PSScriptRoot "build-env.ps1")

$cargo = Get-CargoPath
New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
$env:CARGO_TARGET_DIR = $targetDir
$env:PATH = "$(Split-Path -Parent $cargo);$env:PATH"
$null = Initialize-MsvcBuildEnvironment

Push-Location $repoRoot
try {
    & $cargo build --locked --release --package misku-native-views
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo build fallo con codigo $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

$releaseExe = Join-Path $targetDir "release\misku-native-views.exe"
if (-not (Test-Path -LiteralPath $releaseExe -PathType Leaf)) {
    throw "Cargo termino, pero no encontre $releaseExe."
}

$listResult = Invoke-NativeJson `
    -Executable $releaseExe `
    -Arguments @("--config", $appsConfig, "--json", "--list") `
    -Operation "La lectura del registro"
$profiles = @($listResult.Value)

$profilesByUuid = @{}
foreach ($profile in $profiles) {
    if ($null -eq $profile -or $null -eq $profile.instance_id) {
        throw "El runtime devolvio un perfil sin instance_id."
    }

    $parsedUuid = [System.Guid]::Empty
    if (-not [System.Guid]::TryParse([string]$profile.instance_id, [ref]$parsedUuid)) {
        throw "El runtime devolvio un instance_id invalido: $($profile.instance_id)"
    }
    $uuid = $parsedUuid.ToString("D")
    if ($profilesByUuid.ContainsKey($uuid)) {
        throw "El runtime devolvio un instance_id duplicado: $uuid"
    }
    $profilesByUuid[$uuid] = $profile
}

if (Test-Path -LiteralPath $portableDir) {
    if (Test-ReparsePoint -Path $portableDir) {
        throw "Se rechazo usar portable porque es un enlace o reparse point: $portableDir"
    }

    $lockedExecutables = @(
        Get-ChildItem -LiteralPath $portableDir -Directory -Force |
            Where-Object {
                $candidateUuid = [System.Guid]::Empty
                [System.Guid]::TryParse($_.Name, [ref]$candidateUuid)
            } |
            ForEach-Object {
                $candidate = Join-Path $_.FullName "misku-native-views.exe"
                if (Test-FileLocked -Path $candidate) {
                    $candidate
                }
            }
    )
    if ($lockedExecutables.Count -gt 0) {
        throw "Cierra estas apps antes de regenerar portable: $($lockedExecutables -join ', ')."
    }
}
else {
    New-Item -ItemType Directory -Path $portableDir | Out-Null
}

# Solo se eliminan directorios hijos cuyo nombre es un UUID canonico. Los reparse
# points se rechazan y la pertenencia directa a portable se comprueba antes del
# unico borrado recursivo del script.
foreach ($directory in @(Get-ChildItem -LiteralPath $portableDir -Directory -Force)) {
    $candidateUuid = [System.Guid]::Empty
    if ([System.Guid]::TryParse($directory.Name, [ref]$candidateUuid)) {
        Remove-GeneratedUuidDirectory -PortableRoot $portableDir -Directory $directory
    }
}

# Limpia solamente los dos archivos raiz exactos del formato heredado. Cualquier
# otro archivo del usuario se preserva.
foreach ($legacyName in @("misku-native-views.exe", "apps.toml")) {
    $legacyPath = Join-Path $portableDir $legacyName
    if (Test-Path -LiteralPath $legacyPath -PathType Leaf) {
        Remove-Item -LiteralPath $legacyPath -Force
    }
}

foreach ($uuid in @($profilesByUuid.Keys | Sort-Object)) {
    $profile = $profilesByUuid[$uuid]
    $appDir = Get-DirectChildPath -Root $portableDir -Leaf $uuid
    New-Item -ItemType Directory -Path $appDir | Out-Null

    $portableExe = Join-Path $appDir "misku-native-views.exe"
    $portableManifest = Join-Path $appDir "apps.toml"
    Copy-Item -LiteralPath $releaseExe -Destination $portableExe

    $exportResult = Invoke-NativeJson `
        -Executable $releaseExe `
        -Arguments @(
            "--config", $appsConfig,
            "--json",
            "export", $uuid,
            "--output", $portableManifest
        ) `
        -Operation "La exportacion de $uuid"

    $exportOperations = @($exportResult.Value)
    if ($exportOperations.Count -ne 1 -or
        [string]$exportOperations[0].profile.instance_id -ne $uuid) {
        throw "La exportacion de $uuid no confirmo la identidad esperada."
    }
    if (-not (Test-Path -LiteralPath $portableManifest -PathType Leaf)) {
        throw "La exportacion de $uuid no genero $portableManifest."
    }
}

$indexJson = ConvertTo-Json -InputObject @($profiles) -Depth 10
$utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText(
    (Join-Path $portableDir "apps.json"),
    "$indexJson$([Environment]::NewLine)",
    $utf8WithoutBom
)

if (-not $ReleaseOnly) {
    $portableReadme = @"
Misku Native Views portable

Cada directorio con nombre UUID es una aplicacion independiente:
  <uuid>\misku-native-views.exe

El ejecutable usa el apps.toml de su propio directorio. apps.json es un indice
generado desde la salida JSON del runtime nativo.

Los accesos directos no se precalculan aqui. Ejecuta:
  scripts\install-start-menu.ps1

Cierra las apps portables antes de regenerarlas.
"@
    [System.IO.File]::WriteAllText(
        (Join-Path $portableDir "README.txt"),
        $portableReadme,
        $utf8WithoutBom
    )
}

Write-Output "Portable=$portableDir"
Write-Output "Profiles=$($profilesByUuid.Count)"
foreach ($uuid in @($profilesByUuid.Keys | Sort-Object)) {
    Write-Output "$uuid`t$($profilesByUuid[$uuid].id)`t$($profilesByUuid[$uuid].url)"
}
