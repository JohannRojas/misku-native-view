[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("Install", "Remove")]
    [string]$Mode,

    [Parameter(Mandatory = $true)]
    [string]$ProgramsDir,

    [Parameter(Mandatory = $true)]
    [string]$ShortcutName,

    [string]$TargetPath,
    [string]$TargetArguments = "",
    [string]$WorkingDirectory,
    [string]$IconPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-SafeLeafName {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if ([string]::IsNullOrWhiteSpace($Name)) {
        throw "ShortcutName no puede estar vacio."
    }
    if ($Name -eq "." -or $Name -eq ".." -or [IO.Path]::IsPathRooted($Name)) {
        throw "ShortcutName debe ser un unico nombre de archivo."
    }
    if ([IO.Path]::GetFileName($Name) -cne $Name) {
        throw "ShortcutName no puede contener separadores de ruta."
    }
    if ($Name.Length -gt 180) {
        throw "ShortcutName es demasiado largo."
    }
    if ($Name -match '[\x00-\x1f<>:"/\\|?*]' -or $Name -match '[. ]$') {
        throw "ShortcutName contiene caracteres no permitidos."
    }
    if ($Name -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') {
        throw "ShortcutName usa un nombre reservado de Windows."
    }
}

function Get-NormalizedDirectoryPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "ProgramsDir no puede estar vacio."
    }
    return [IO.Path]::GetFullPath($Path)
}

function Assert-NormalDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer) {
        throw "ProgramsDir no es un directorio: $Path"
    }
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "ProgramsDir no puede ser un enlace ni un punto de reparacion: $Path"
    }
}

function Resolve-RequiredFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Label es obligatorio en modo Install."
    }
    $fullPath = [IO.Path]::GetFullPath($Path)
    $item = Get-Item -LiteralPath $fullPath -Force
    if ($item.PSIsContainer) {
        throw "$Label debe apuntar a un archivo: $fullPath"
    }
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label no puede ser un enlace ni un punto de reparacion: $fullPath"
    }
    return $fullPath
}

function Resolve-RequiredDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Label es obligatorio en modo Install."
    }
    $fullPath = [IO.Path]::GetFullPath($Path)
    $item = Get-Item -LiteralPath $fullPath -Force
    if (-not $item.PSIsContainer) {
        throw "$Label debe apuntar a un directorio: $fullPath"
    }
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Label no puede ser un enlace ni un punto de reparacion: $fullPath"
    }
    return $fullPath
}

Assert-SafeLeafName -Name $ShortcutName

$programsRoot = Get-NormalizedDirectoryPath -Path $ProgramsDir
$rootPrefix = $programsRoot.TrimEnd(
    [IO.Path]::DirectorySeparatorChar,
    [IO.Path]::AltDirectorySeparatorChar
) + [IO.Path]::DirectorySeparatorChar
$shortcutPath = [IO.Path]::GetFullPath(
    [IO.Path]::Combine($programsRoot, "$ShortcutName.lnk")
)

if (-not $shortcutPath.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "La ruta del acceso directo queda fuera de ProgramsDir."
}

if ($Mode -eq "Remove") {
    if (-not (Test-Path -LiteralPath $programsRoot)) {
        Write-Output $shortcutPath
        exit 0
    }

    Assert-NormalDirectory -Path $programsRoot
    $removedShortcut = $false
    if (Test-Path -LiteralPath $shortcutPath) {
        $shortcutItem = Get-Item -LiteralPath $shortcutPath -Force
        if ($shortcutItem.PSIsContainer) {
            throw "Se rechazo eliminar un directorio que ocupa la ruta del acceso directo."
        }
        if (($shortcutItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Se rechazo eliminar un punto de reparacion en la ruta del acceso directo."
        }
        Remove-Item -LiteralPath $shortcutPath -Force
        $removedShortcut = $true
    }

    if ($removedShortcut -and -not (Get-ChildItem -LiteralPath $programsRoot -Force | Select-Object -First 1)) {
        Remove-Item -LiteralPath $programsRoot
    }

    Write-Output $shortcutPath
    exit 0
}

if (-not (Test-Path -LiteralPath $programsRoot)) {
    New-Item -ItemType Directory -Path $programsRoot -Force | Out-Null
}
Assert-NormalDirectory -Path $programsRoot

if (Test-Path -LiteralPath $shortcutPath) {
    $shortcutItem = Get-Item -LiteralPath $shortcutPath -Force
    if ($shortcutItem.PSIsContainer) {
        throw "Se rechazo sobrescribir un directorio que ocupa la ruta del acceso directo."
    }
    if (($shortcutItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Se rechazo sobrescribir un punto de reparacion en la ruta del acceso directo."
    }
}

$resolvedTarget = Resolve-RequiredFile -Path $TargetPath -Label "TargetPath"
$resolvedWorkingDirectory = Resolve-RequiredDirectory `
    -Path $WorkingDirectory `
    -Label "WorkingDirectory"
$resolvedIcon = $null
if (-not [string]::IsNullOrWhiteSpace($IconPath)) {
    $resolvedIcon = Resolve-RequiredFile -Path $IconPath -Label "IconPath"
}

$shell = $null
$shortcut = $null
try {
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $resolvedTarget
    $shortcut.Arguments = $TargetArguments
    $shortcut.WorkingDirectory = $resolvedWorkingDirectory
    if ($null -ne $resolvedIcon) {
        $shortcut.IconLocation = "$resolvedIcon,0"
    }
    $shortcut.Save()
}
finally {
    if ($null -ne $shortcut) {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
    }
    if ($null -ne $shell) {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }
}

if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) {
    throw "WScript.Shell no creo el acceso directo esperado: $shortcutPath"
}

Write-Output $shortcutPath
