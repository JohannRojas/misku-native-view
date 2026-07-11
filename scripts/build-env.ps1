$ErrorActionPreference = "Stop"

function Get-CargoPath {
    $cargoCommand = Get-Command "cargo.exe" -ErrorAction SilentlyContinue
    if ($cargoCommand) {
        return $cargoCommand.Source
    }

    $homeCargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path -LiteralPath $homeCargo) {
        return $homeCargo
    }

    throw "No encontre Cargo. Instala Rust o agrega Cargo al PATH."
}

function Get-VisualStudioInstallPath {
    if (-not [string]::IsNullOrWhiteSpace($env:VSINSTALLDIR) -and (Test-Path -LiteralPath $env:VSINSTALLDIR)) {
        return $env:VSINSTALLDIR.TrimEnd("\")
    }

    $vswhereCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"),
        (Join-Path $env:ProgramFiles "Microsoft Visual Studio\Installer\vswhere.exe")
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }

    foreach ($vswhere in $vswhereCandidates) {
        if (-not (Test-Path -LiteralPath $vswhere)) {
            continue
        }

        $installPath = & $vswhere `
            -latest `
            -products * `
            -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            -property installationPath

        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installPath)) {
            return $installPath.Trim()
        }
    }

    $knownInstallPaths = @(
        "C:\Program Files\Microsoft Visual Studio\2022\Enterprise",
        "C:\Program Files\Microsoft Visual Studio\2022\Professional",
        "C:\Program Files\Microsoft Visual Studio\2022\Community",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools",
        "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
    )

    foreach ($installPath in $knownInstallPaths) {
        if (Test-Path -LiteralPath (Join-Path $installPath "Common7\Tools\Launch-VsDevShell.ps1")) {
            return $installPath
        }
    }

    throw "No encontre una instalacion de Visual Studio 2022 con C++ Build Tools."
}

function Initialize-MsvcBuildEnvironment {
    $vsInstallPath = Get-VisualStudioInstallPath
    $devShell = Join-Path $vsInstallPath "Common7\Tools\Launch-VsDevShell.ps1"

    if (-not (Test-Path -LiteralPath $devShell)) {
        throw "No encontre Visual Studio DevShell en $devShell."
    }

    . $devShell -VsInstallationPath $vsInstallPath -Arch amd64 -HostArch amd64
    return $vsInstallPath
}

