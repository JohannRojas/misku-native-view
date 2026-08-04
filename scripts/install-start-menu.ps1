[CmdletBinding()]
param(
    [string] $PortableDir = (Join-Path (Split-Path -Parent $PSScriptRoot) "portable"),
    [string] $StartMenuFolder = "Misku Native Views",
    [switch] $Uninstall
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version 3.0

function Test-SafeLeafName {
    param([Parameter(Mandatory = $true)][string] $Value)

    return -not (
        [string]::IsNullOrWhiteSpace($Value) -or
        $Value -ne [System.IO.Path]::GetFileName($Value) -or
        $Value -in @(".", "..") -or
        $Value -match '[<>:"/\\|?*\x00-\x1f]' -or
        $Value -match '[. ]$' -or
        $Value -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?$'
    )
}

function Test-ReparsePoint {
    param([Parameter(Mandatory = $true)][string] $Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }
    $item = Get-Item -LiteralPath $Path -Force
    return [bool]($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)
}

function Get-ContainedChildPath {
    param(
        [Parameter(Mandatory = $true)][string] $Root,
        [Parameter(Mandatory = $true)][string] $Leaf
    )

    if (-not (Test-SafeLeafName -Value $Leaf)) {
        throw "Nombre de archivo o directorio no seguro: $Leaf"
    }
    $rootPath = [System.IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $rootPath $Leaf))
    $prefix = "$rootPath$([System.IO.Path]::DirectorySeparatorChar)"
    if (-not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Ruta fuera del directorio administrado: $candidate"
    }
    return $candidate
}

function ConvertTo-ShortcutBaseName {
    param(
        [AllowNull()][string] $Name,
        [AllowNull()][string] $Id,
        [Parameter(Mandatory = $true)][string] $Uuid
    )

    $base = if (-not [string]::IsNullOrWhiteSpace($Name)) {
        $Name
    }
    elseif (-not [string]::IsNullOrWhiteSpace($Id)) {
        $Id
    }
    else {
        "App"
    }
    $base = $base -replace '[\x00-\x1f<>:"/\\|?*]', ' '
    $base = ($base -replace '\s+', ' ').Trim().TrimEnd('.', ' ')
    if ($base.Length -gt 80) {
        $base = $base.Substring(0, 80).TrimEnd('.', ' ')
    }
    if ([string]::IsNullOrWhiteSpace($base) -or
        $base -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?$') {
        $base = "App"
    }
    return "$base-$($Uuid.Substring(0, 8))"
}

function Get-OwnershipManifestPath {
    param([Parameter(Mandatory = $true)][string] $Directory)

    return Get-ContainedChildPath `
        -Root $Directory `
        -Leaf ".misku-native-views-shortcuts.json"
}

function Get-OwnedShortcutNames {
    param([Parameter(Mandatory = $true)][string] $Directory)

    $manifestPath = Get-OwnershipManifestPath -Directory $Directory
    if (-not (Test-Path -LiteralPath $manifestPath)) {
        return @()
    }
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf) -or
        (Test-ReparsePoint -Path $manifestPath)) {
        throw "Se rechazo leer un manifiesto de propiedad invalido o reparse point: $manifestPath"
    }

    try {
        $manifest = ConvertFrom-Json -InputObject (Get-Content -Raw -LiteralPath $manifestPath)
    }
    catch {
        throw "El manifiesto de accesos directos no es valido: $($_.Exception.Message)"
    }

    $owned = @()
    foreach ($name in @($manifest.shortcuts)) {
        $leaf = [string]$name
        if (-not (Test-SafeLeafName -Value $leaf) -or
            [System.IO.Path]::GetExtension($leaf) -ne ".lnk") {
            throw "El manifiesto contiene un acceso directo no seguro: $leaf"
        }
        $owned += $leaf
    }
    return @($owned | Select-Object -Unique)
}

function Remove-OwnedShortcuts {
    param([Parameter(Mandatory = $true)][string] $Directory)

    if (-not (Test-Path -LiteralPath $Directory)) {
        return
    }
    if (-not (Test-Path -LiteralPath $Directory -PathType Container) -or
        (Test-ReparsePoint -Path $Directory)) {
        throw "Se rechazo administrar una carpeta invalida o reparse point: $Directory"
    }

    foreach ($name in @(Get-OwnedShortcutNames -Directory $Directory)) {
        $shortcutPath = Get-ContainedChildPath -Root $Directory -Leaf $name
        if (Test-Path -LiteralPath $shortcutPath) {
            if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf) -or
                (Test-ReparsePoint -Path $shortcutPath)) {
                throw "Se rechazo eliminar un acceso directo invalido o reparse point: $shortcutPath"
            }
            Remove-Item -LiteralPath $shortcutPath -Force
            Write-Host "Removed Start Menu shortcut: $shortcutPath"
        }
    }

    $manifestPath = Get-OwnershipManifestPath -Directory $Directory
    if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
        Remove-Item -LiteralPath $manifestPath -Force
    }
}

function Write-OwnershipManifest {
    param(
        [Parameter(Mandatory = $true)][string] $Directory,
        [Parameter(Mandatory = $true)][string[]] $ShortcutNames
    )

    $manifestPath = Get-OwnershipManifestPath -Directory $Directory
    if ((Test-Path -LiteralPath $manifestPath) -and
        (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf) -or
         (Test-ReparsePoint -Path $manifestPath))) {
        throw "Se rechazo reemplazar un manifiesto invalido o reparse point: $manifestPath"
    }

    $temporary = "$manifestPath.$PID.tmp"
    if (Test-Path -LiteralPath $temporary) {
        throw "La ruta temporal del manifiesto ya existe: $temporary"
    }
    $payload = [ordered]@{
        schema_version = 1
        shortcuts = @($ShortcutNames)
    } | ConvertTo-Json -Depth 3
    $utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
    try {
        [System.IO.File]::WriteAllText(
            $temporary,
            "$payload$([Environment]::NewLine)",
            $utf8WithoutBom
        )
        Move-Item -LiteralPath $temporary -Destination $manifestPath -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

function Remove-TransactionDirectory {
    param([Parameter(Mandatory = $true)][string] $Directory)

    if (-not (Test-Path -LiteralPath $Directory)) {
        return
    }
    if (-not (Test-Path -LiteralPath $Directory -PathType Container) -or
        (Test-ReparsePoint -Path $Directory)) {
        throw "Se rechazo limpiar una transaccion invalida o reparse point: $Directory"
    }
    foreach ($item in @(Get-ChildItem -LiteralPath $Directory -Force)) {
        if ($item.PSIsContainer -or
            [bool]($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw "Se rechazo limpiar un elemento inesperado de la transaccion: $($item.FullName)"
        }
        Remove-Item -LiteralPath $item.FullName -Force
    }
    if (@(Get-ChildItem -LiteralPath $Directory -Force).Count -ne 0) {
        throw "No se pudo vaciar el directorio de transaccion: $Directory"
    }
    Remove-Item -LiteralPath $Directory -Force
}

if ([string]::IsNullOrWhiteSpace($env:APPDATA)) {
    throw "APPDATA no esta definido."
}
if (-not (Test-SafeLeafName -Value $StartMenuFolder)) {
    throw "StartMenuFolder debe ser un unico nombre de carpeta seguro."
}

$programsDir = [System.IO.Path]::GetFullPath(
    (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs")
)
$targetDir = Get-ContainedChildPath -Root $programsDir -Leaf $StartMenuFolder

if ($Uninstall) {
    Remove-OwnedShortcuts -Directory $targetDir
    if (Test-Path -LiteralPath $targetDir) {
        $remaining = @(Get-ChildItem -LiteralPath $targetDir -Force)
        if ($remaining.Count -eq 0) {
            Remove-Item -LiteralPath $targetDir -Force
            Write-Host "Removed empty Start Menu folder: $targetDir"
        }
        else {
            Write-Host "Preserved non-empty Start Menu folder: $targetDir"
        }
    }
    exit 0
}

$portableRoot = (Resolve-Path -LiteralPath $PortableDir).Path
if (Test-ReparsePoint -Path $portableRoot) {
    throw "Se rechazo usar un portable que es enlace o reparse point: $portableRoot"
}
$indexPath = Join-Path $portableRoot "apps.json"
if (-not (Test-Path -LiteralPath $indexPath -PathType Leaf) -or
    (Test-ReparsePoint -Path $indexPath)) {
    throw "No encontre un apps.json regular y seguro en $portableRoot."
}
try {
    $profiles = @(ConvertFrom-Json -InputObject (Get-Content -Raw -LiteralPath $indexPath))
}
catch {
    throw "apps.json no es valido: $($_.Exception.Message)"
}

$targetExists = Test-Path -LiteralPath $targetDir
if ($targetExists -and
    (-not (Test-Path -LiteralPath $targetDir -PathType Container) -or
     (Test-ReparsePoint -Path $targetDir))) {
    throw "Se rechazo administrar una carpeta invalida o reparse point: $targetDir"
}
$previouslyOwned = if ($targetExists) {
    @(Get-OwnedShortcutNames -Directory $targetDir)
}
else {
    @()
}
$ownershipManifestPath = Get-OwnershipManifestPath -Directory $targetDir
$previousManifestExists = $targetExists -and
    (Test-Path -LiteralPath $ownershipManifestPath -PathType Leaf)

# Fase 1: valida el estado previo y cada app sin mutar el menu Inicio.
$previousShortcutFiles = @()
foreach ($name in $previouslyOwned) {
    $shortcutPath = Get-ContainedChildPath -Root $targetDir -Leaf $name
    $exists = $targetExists -and (Test-Path -LiteralPath $shortcutPath)
    if ($exists -and
        (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf) -or
         (Test-ReparsePoint -Path $shortcutPath))) {
        throw "El manifiesto anterior apunta a una ruta invalida o reparse point: $shortcutPath"
    }
    $previousShortcutFiles += [pscustomobject]@{
        Name = $name
        Path = $shortcutPath
        Exists = $exists
    }
}

$plannedNames = @{}
$plan = @()
foreach ($profile in $profiles) {
    if ($null -eq $profile -or $null -eq $profile.instance_id) {
        throw "apps.json contiene un perfil sin instance_id."
    }
    $parsedUuid = [System.Guid]::Empty
    if (-not [System.Guid]::TryParse([string]$profile.instance_id, [ref]$parsedUuid)) {
        throw "apps.json contiene un instance_id invalido: $($profile.instance_id)"
    }
    $uuid = $parsedUuid.ToString("D")
    $appDir = Get-ContainedChildPath -Root $portableRoot -Leaf $uuid
    if (-not (Test-Path -LiteralPath $appDir -PathType Container) -or
        (Test-ReparsePoint -Path $appDir)) {
        throw "No encontre un directorio portable regular y seguro para $uuid."
    }

    $targetPath = Join-Path $appDir "misku-native-views.exe"
    $manifestPath = Join-Path $appDir "apps.toml"
    foreach ($requiredPath in @($targetPath, $manifestPath)) {
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf) -or
            (Test-ReparsePoint -Path $requiredPath)) {
            throw "El portable $uuid contiene una ruta requerida invalida: $requiredPath"
        }
    }

    $shortcutBase = ConvertTo-ShortcutBaseName `
        -Name ([string]$profile.name) `
        -Id ([string]$profile.id) `
        -Uuid $uuid
    $shortcutName = "$shortcutBase.lnk"
    if ($plannedNames.ContainsKey($shortcutName)) {
        throw "Dos apps producirian el mismo acceso directo: $shortcutName"
    }
    $plannedNames[$shortcutName] = $true
    $shortcutPath = Get-ContainedChildPath -Root $targetDir -Leaf $shortcutName
    if ($targetExists -and
        (Test-Path -LiteralPath $shortcutPath) -and
        $previouslyOwned -notcontains $shortcutName) {
        throw "Se rechazo sobrescribir un archivo no administrado: $shortcutPath"
    }

    $iconPath = Join-Path $appDir "icons\$uuid.ico"
    if (Test-Path -LiteralPath $iconPath) {
        if (-not (Test-Path -LiteralPath $iconPath -PathType Leaf) -or
            (Test-ReparsePoint -Path $iconPath)) {
            throw "El icono de $uuid no es un archivo regular y seguro: $iconPath"
        }
    }
    else {
        $iconPath = $targetPath
    }
    $plan += [pscustomobject]@{
        Uuid = $uuid
        ShortcutName = $shortcutName
        ShortcutPath = $shortcutPath
        TargetPath = $targetPath
        ManifestPath = $manifestPath
        AppDirectory = $appDir
        IconPath = $iconPath
    }
}

# Fase 2: respalda todos los artefactos propios antes de la primera mutacion.
$targetCreated = $false
if (-not $targetExists) {
    New-Item -ItemType Directory -Path $targetDir | Out-Null
    $targetCreated = $true
}
$transactionName = ".misku-native-views-txn-$([System.Guid]::NewGuid().ToString('N'))"
$transactionDir = Get-ContainedChildPath -Root $targetDir -Leaf $transactionName
New-Item -ItemType Directory -Path $transactionDir | Out-Null
if (Test-ReparsePoint -Path $transactionDir) {
    throw "Se rechazo usar un directorio de transaccion que es reparse point: $transactionDir"
}

$backupManifestPath = Get-ContainedChildPath -Root $transactionDir -Leaf "ownership.json"
$backupShortcuts = @()
$attemptedShortcutPaths = @()
$mutationStarted = $false
$transactionCommitted = $false
$shell = $null

try {
    foreach ($previous in $previousShortcutFiles) {
        if ($previous.Exists) {
            $backupPath = Get-ContainedChildPath -Root $transactionDir -Leaf $previous.Name
            Copy-Item -LiteralPath $previous.Path -Destination $backupPath
            $backupShortcuts += [pscustomobject]@{
                Original = $previous.Path
                Backup = $backupPath
            }
        }
    }
    if ($previousManifestExists) {
        Copy-Item -LiteralPath $ownershipManifestPath -Destination $backupManifestPath
    }

    $mutationStarted = $true
    foreach ($previous in $previousShortcutFiles) {
        if ($previous.Exists) {
            Remove-Item -LiteralPath $previous.Path -Force
        }
    }
    if ($previousManifestExists) {
        Remove-Item -LiteralPath $ownershipManifestPath -Force
    }

    $shell = New-Object -ComObject WScript.Shell
    foreach ($entry in $plan) {
        if (Test-Path -LiteralPath $entry.ShortcutPath) {
            throw "Aparecio un archivo no administrado durante la instalacion: $($entry.ShortcutPath)"
        }
        $attemptedShortcutPaths += $entry.ShortcutPath
        $shortcut = $shell.CreateShortcut($entry.ShortcutPath)
        try {
            $shortcut.TargetPath = $entry.TargetPath
            $shortcut.Arguments = "--config `"$($entry.ManifestPath)`" --app `"$($entry.Uuid)`""
            $shortcut.WorkingDirectory = $entry.AppDirectory
            $shortcut.IconLocation = "$($entry.IconPath),0"
            $shortcut.Save()
        }
        finally {
            if ($null -ne $shortcut) {
                [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
            }
        }
    }

    Write-OwnershipManifest `
        -Directory $targetDir `
        -ShortcutNames @($plan | ForEach-Object { $_.ShortcutName })
    $transactionCommitted = $true
}
catch {
    $operationError = $_
    $rollbackErrors = @()
    if ($mutationStarted) {
        foreach ($shortcutPath in $attemptedShortcutPaths) {
            try {
                if (Test-Path -LiteralPath $shortcutPath) {
                    if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf) -or
                        (Test-ReparsePoint -Path $shortcutPath)) {
                        throw "la ruta nueva no es un archivo regular: $shortcutPath"
                    }
                    Remove-Item -LiteralPath $shortcutPath -Force
                }
            }
            catch {
                $rollbackErrors += $_.Exception.Message
            }
        }
        try {
            if (Test-Path -LiteralPath $ownershipManifestPath) {
                if (-not (Test-Path -LiteralPath $ownershipManifestPath -PathType Leaf) -or
                    (Test-ReparsePoint -Path $ownershipManifestPath)) {
                    throw "el manifiesto nuevo no es un archivo regular: $ownershipManifestPath"
                }
                Remove-Item -LiteralPath $ownershipManifestPath -Force
            }
        }
        catch {
            $rollbackErrors += $_.Exception.Message
        }
        foreach ($backup in $backupShortcuts) {
            try {
                if (-not (Test-Path -LiteralPath $backup.Backup -PathType Leaf) -or
                    (Test-ReparsePoint -Path $backup.Backup)) {
                    throw "falta un backup regular: $($backup.Backup)"
                }
                Copy-Item -LiteralPath $backup.Backup -Destination $backup.Original -Force
            }
            catch {
                $rollbackErrors += $_.Exception.Message
            }
        }
        if ($previousManifestExists) {
            try {
                if (-not (Test-Path -LiteralPath $backupManifestPath -PathType Leaf) -or
                    (Test-ReparsePoint -Path $backupManifestPath)) {
                    throw "falta el backup del manifiesto: $backupManifestPath"
                }
                Copy-Item `
                    -LiteralPath $backupManifestPath `
                    -Destination $ownershipManifestPath `
                    -Force
            }
            catch {
                $rollbackErrors += $_.Exception.Message
            }
        }
    }

    try {
        Remove-TransactionDirectory -Directory $transactionDir
    }
    catch {
        $rollbackErrors += $_.Exception.Message
    }
    if ($targetCreated -and (Test-Path -LiteralPath $targetDir)) {
        try {
            if (@(Get-ChildItem -LiteralPath $targetDir -Force).Count -eq 0) {
                Remove-Item -LiteralPath $targetDir -Force
            }
        }
        catch {
            $rollbackErrors += $_.Exception.Message
        }
    }
    if ($rollbackErrors.Count -gt 0) {
        throw "$($operationError.Exception.Message) Rollback incompleto: $($rollbackErrors -join ' | ')"
    }
    throw $operationError
}
finally {
    if ($null -ne $shell) {
        [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }
}

if ($transactionCommitted) {
    try {
        Remove-TransactionDirectory -Directory $transactionDir
    }
    catch {
        Write-Warning "La instalacion termino, pero no se pudo limpiar el backup: $($_.Exception.Message)"
    }
}
foreach ($entry in $plan) {
    Write-Host "Installed Start Menu shortcut: $($entry.ShortcutPath)"
}
Write-Host ""
Write-Host "Done. Open Start and search for an app name."
