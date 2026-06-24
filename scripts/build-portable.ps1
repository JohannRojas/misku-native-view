param(
    [switch] $ReleaseOnly
)

$ErrorActionPreference = "Stop"

function Test-FileLocked {
    param([Parameter(Mandatory = $true)][string] $Path)

    if (-not (Test-Path $Path)) {
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

function Get-AppProfiles {
    param([Parameter(Mandatory = $true)][string] $Path)

    $profiles = @()
    $current = $null

    foreach ($line in Get-Content -Path $Path) {
        if ($line -match '^\s*\[\[apps\]\]\s*$') {
            if ($null -ne $current -and $current.Id) {
                $profiles += [pscustomobject]$current
            }
            $current = @{ Id = $null; Icon = $null }
            continue
        }

        if ($null -eq $current) {
            continue
        }

        if ($line -match '^\s*id\s*=\s*"([^"]+)"') {
            $current.Id = $Matches[1]
        } elseif ($line -match '^\s*icon\s*=\s*"([^"]+)"') {
            $current.Icon = $Matches[1]
        }
    }

    if ($null -ne $current -and $current.Id) {
        $profiles += [pscustomobject]$current
    }

    return $profiles
}

function Resolve-PortableIconPath {
    param(
        [Parameter(Mandatory = $true)][string] $RepoRoot,
        [Parameter(Mandatory = $true)][string] $PortableDir,
        [AllowNull()][string] $Icon
    )

    if ([string]::IsNullOrWhiteSpace($Icon)) {
        return $null
    }

    if ([System.IO.Path]::IsPathRooted($Icon)) {
        return $Icon
    }

    $source = Join-Path $RepoRoot $Icon
    if (-not (Test-Path $source)) {
        Write-Warning "No encontre el icono configurado: $Icon"
        return $null
    }

    $destination = Join-Path $PortableDir $Icon
    $destinationDir = Split-Path -Parent $destination
    New-Item -ItemType Directory -Path $destinationDir -Force | Out-Null
    Copy-Item -Path $source -Destination $destination -Force
    return $destination
}

function New-AppShortcut {
    param(
        [Parameter(Mandatory = $true)][string] $ShortcutPath,
        [Parameter(Mandatory = $true)][string] $TargetPath,
        [Parameter(Mandatory = $true)][string] $WorkingDirectory,
        [AllowNull()][string] $IconPath
    )

    $resolvedTargetPath = (Resolve-Path -LiteralPath $TargetPath).Path
    $resolvedWorkingDirectory = (Resolve-Path -LiteralPath $WorkingDirectory).Path

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($ShortcutPath)
    $shortcut.TargetPath = $resolvedTargetPath
    $shortcut.WorkingDirectory = $resolvedWorkingDirectory
    if (-not [string]::IsNullOrWhiteSpace($IconPath) -and (Test-Path $IconPath)) {
        $shortcut.IconLocation = "$IconPath,0"
    }
    $shortcut.Save()
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
$vsInstallPath = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
$devShell = Join-Path $vsInstallPath "Common7\Tools\Launch-VsDevShell.ps1"
$targetDir = Join-Path $env:LOCALAPPDATA "misku-native-views\cargo-target"
$portableDir = Join-Path $repoRoot "portable"
$releaseExe = Join-Path $targetDir "release\misku-native-views.exe"
$appsConfig = Join-Path $repoRoot "apps.toml"

if (-not (Test-Path $cargo)) {
    throw "No encontre Cargo en $cargo. Instala Rust o agrega Cargo al PATH."
}

if (-not (Test-Path $devShell)) {
    throw "No encontre Visual Studio Build Tools en $vsInstallPath."
}

if (-not (Test-Path $appsConfig)) {
    throw "No encontre apps.toml en $repoRoot."
}

New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
$env:CARGO_TARGET_DIR = $targetDir
$env:PATH = "$(Split-Path $cargo);$env:PATH"

. $devShell -VsInstallationPath $vsInstallPath -Arch amd64 -HostArch amd64

Push-Location $repoRoot
try {
    & $cargo build --release -p misku-native-views
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo build fallo con codigo $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

if (-not (Test-Path $releaseExe)) {
    throw "Cargo termino, pero no encontre $releaseExe."
}

$appProfiles = @(Get-AppProfiles -Path $appsConfig)
$appIds = @($appProfiles | ForEach-Object { $_.Id })
$expectedExeNames = @("misku-native-views.exe")
$expectedExeNames += @($appIds | ForEach-Object { "$_.exe" })
$expectedShortcutNames = @($appIds | ForEach-Object { "$_.lnk" })

New-Item -ItemType Directory -Path $portableDir -Force | Out-Null

$lockedTargets = foreach ($exeName in $expectedExeNames) {
    $targetPath = Join-Path $portableDir $exeName
    if (Test-FileLocked -Path $targetPath) {
        $exeName
    }
}

if ($lockedTargets.Count -gt 0) {
    throw "Cierra estas apps antes de regenerar portable: $($lockedTargets -join ', ')."
}

$staleExecutables = Get-ChildItem -Path $portableDir -Filter "*.exe" -File -ErrorAction SilentlyContinue |
    Where-Object { $expectedExeNames -notcontains $_.Name }
$staleShortcuts = Get-ChildItem -Path $portableDir -Filter "*.lnk" -File -ErrorAction SilentlyContinue |
    Where-Object { $expectedShortcutNames -notcontains $_.Name }

foreach ($exe in $staleExecutables) {
    Remove-Item -LiteralPath $exe.FullName -Force
}
foreach ($shortcut in $staleShortcuts) {
    Remove-Item -LiteralPath $shortcut.FullName -Force
}

Copy-Item -Path $releaseExe -Destination (Join-Path $portableDir "misku-native-views.exe") -Force
Copy-Item -Path $appsConfig -Destination (Join-Path $portableDir "apps.toml") -Force

foreach ($profile in $appProfiles) {
    $profileExe = Join-Path $portableDir "$($profile.Id).exe"
    Copy-Item -Path $releaseExe -Destination $profileExe -Force

    $portableIconPath = Resolve-PortableIconPath -RepoRoot $repoRoot -PortableDir $portableDir -Icon $profile.Icon
    New-AppShortcut `
        -ShortcutPath (Join-Path $portableDir "$($profile.Id).lnk") `
        -TargetPath $profileExe `
        -WorkingDirectory $portableDir `
        -IconPath $portableIconPath
}

if (-not $ReleaseOnly) {
    $readme = @"
Misku Native Views portable

Ejecutables:
- misku-native-views.exe: abre el primer perfil de apps.toml o acepta --app <id>.
- <id>.exe: abre automaticamente el perfil con ese id.
- <id>.lnk: acceso directo con icono personalizado cuando apps.toml define icon.

Puedes editar apps.toml y volver a ejecutar scripts\build-portable.ps1 para generar nuevas copias por perfil.
El script tambien elimina ejecutables y accesos directos de perfiles que ya no existan en apps.toml.
Cierra las apps portables antes de regenerar para que Windows permita reemplazar los .exe.
"@
    Set-Content -Path (Join-Path $portableDir "README.txt") -Encoding UTF8 -Value $readme
}

Write-Output "Portable=$portableDir"
Write-Output "Profiles=$($appIds -join ', ')"
$removedItems = @($staleExecutables | ForEach-Object { $_.Name }) + @($staleShortcuts | ForEach-Object { $_.Name })
Write-Output "Removed=$($removedItems -join ', ')"

