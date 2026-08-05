param(
    [Parameter(Mandatory = $true)]
    [Alias("PackagePath")]
    [string] $Package
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Set-ProcessVariable {
    param([string] $Name, [AllowNull()][string] $Value)
    [System.Environment]::SetEnvironmentVariable($Name, $Value, "Process")
}

function Invoke-Cli {
    param([string] $Executable, [string[]] $Arguments)
    $output = & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "misku-nv $($Arguments -join ' ') fallo con codigo $LASTEXITCODE.`n$($output -join "`n")"
    }
    return $output
}

function Get-InstalledProfiles {
    param([string] $Executable)
    $parsed = ((Invoke-Cli $Executable @("--list", "--json")) -join "`n") | ConvertFrom-Json
    return @($parsed)
}

function Remove-SafeTestRoot {
    param([string] $Path, [string] $TemporaryRoot)
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    $root = [System.IO.Path]::GetFullPath($TemporaryRoot).TrimEnd("\", "/")
    $candidate = [System.IO.Path]::GetFullPath($Path).TrimEnd("\", "/")
    $prefix = "$root$([System.IO.Path]::DirectorySeparatorChar)"
    if (
        -not $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase) -or
        -not (Split-Path -Leaf $candidate).StartsWith("mnv-smoke-", [System.StringComparison]::Ordinal)
    ) {
        throw "Se rechazo limpiar una ruta no administrada: $candidate"
    }
    if (((Get-Item -LiteralPath $candidate -Force).Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Se rechazo limpiar un reparse point: $candidate"
    }

    try {
        Remove-Item -LiteralPath $candidate -Recurse -Force
    } catch {
        [System.IO.Directory]::Delete("\\?\$candidate", $true)
    }
}

$packagePath = (Resolve-Path -LiteralPath $Package -ErrorAction Stop).Path
if (
    -not (Test-Path -LiteralPath $packagePath -PathType Leaf) -or
    [System.IO.Path]::GetExtension($packagePath) -ne ".tgz"
) {
    throw "El smoke test requiere el tarball .tgz exacto producido por pnpm pack."
}

$pnpm = (Get-Command "pnpm" -ErrorAction Stop).Source
$tar = (Get-Command "tar" -ErrorAction Stop).Source
$node = (Get-Command "node" -ErrorAction Stop).Source
$archiveEntries = @(& $tar -tf $packagePath) |
    ForEach-Object { ([string] $_).Replace("\", "/") }
if ($LASTEXITCODE -ne 0) {
    throw "No pude inspeccionar el tarball: $packagePath"
}
$allowedEntries = @(
    "package/bin/favicon.js",
    "package/bin/misku.js",
    "package/package.json",
    "package/README.md",
    "package/runtime/misku-native-views.exe",
    "package/scripts/manage-shortcut.ps1"
)
$missingEntries = @($allowedEntries | Where-Object { $_ -notin $archiveEntries })
$unexpectedEntries = @($archiveEntries | Where-Object { $_ -notin $allowedEntries })
if ($missingEntries.Count -ne 0 -or $unexpectedEntries.Count -ne 0) {
    throw "Contenido de tarball invalido. Faltan: $($missingEntries -join ', '). Sobran: $($unexpectedEntries -join ', ')."
}

$temporaryRoot = if (
    -not [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -and
    (Test-Path -LiteralPath $env:RUNNER_TEMP -PathType Container)
) {
    $env:RUNNER_TEMP
} else {
    $env:TEMP
}
$testRoot = Join-Path $temporaryRoot "mnv-smoke-$([Guid]::NewGuid().ToString('N'))"
$installDir = Join-Path $testRoot "consumer"
$miskuHome = Join-Path $testRoot "home"
$programsDir = Join-Path $testRoot "start-menu"
$faviconServerScript = Join-Path $testRoot "favicon-server.js"
$faviconReadyFile = Join-Path $testRoot "favicon-server.port"
$faviconServer = $null
$environmentNames = @("MISKU_NV_HOME", "MISKU_NV_PROGRAMS_DIR", "MISKU_NV_NATIVE_EXE", "CI")
$originalEnvironment = @{}
foreach ($name in $environmentNames) {
    $originalEnvironment[$name] = [System.Environment]::GetEnvironmentVariable($name, "Process")
}

$shortcutShell = $null
try {
    New-Item -ItemType Directory -Path $installDir, $miskuHome, $programsDir -Force | Out-Null
    Set-Content `
        -LiteralPath (Join-Path $installDir "package.json") `
        -Encoding UTF8 `
        -Value "{`"name`":`"misku-smoke-consumer`",`"private`":true}"

    $faviconServerSource = @'
"use strict";
const fs = require("node:fs");
const http = require("node:http");
const [iconPath, readyPath] = process.argv.slice(2);
const icon = fs.readFileSync(iconPath);
const server = http.createServer((request, response) => {
  if (request.url === "/app") {
    response.setHeader("Content-Type", "text/html; charset=utf-8");
    response.end('<link rel="icon" href="/favicon.ico"><title>Favicon smoke</title>');
    return;
  }
  if (request.url === "/favicon.ico") {
    response.setHeader("Content-Type", "image/x-icon");
    response.setHeader("Content-Length", String(icon.length));
    response.end(icon);
    return;
  }
  response.statusCode = 404;
  response.end();
});
server.listen(0, "127.0.0.1", () => {
  fs.writeFileSync(readyPath, String(server.address().port), { flag: "wx" });
});
'@
    Set-Content -LiteralPath $faviconServerScript -Encoding UTF8 -Value $faviconServerSource
    $iconFixture = (
        Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\src-tauri\icons\icon.ico")
    ).Path
    $faviconServer = Start-Process `
        -FilePath $node `
        -ArgumentList @(
            "`"$faviconServerScript`"",
            "`"$iconFixture`"",
            "`"$faviconReadyFile`""
        ) `
        -PassThru `
        -WindowStyle Hidden
    for ($attempt = 0; $attempt -lt 100 -and -not (Test-Path -LiteralPath $faviconReadyFile); $attempt += 1) {
        if ($faviconServer.HasExited) {
            throw "El servidor local de favicon termino antes de iniciar."
        }
        Start-Sleep -Milliseconds 50
    }
    if (-not (Test-Path -LiteralPath $faviconReadyFile -PathType Leaf)) {
        throw "El servidor local de favicon no informo su puerto."
    }
    $faviconPort = [int](Get-Content -LiteralPath $faviconReadyFile -Raw)
    $faviconUrl = "http://127.0.0.1:$faviconPort/app"

    Set-ProcessVariable "MISKU_NV_HOME" $miskuHome
    Set-ProcessVariable "MISKU_NV_PROGRAMS_DIR" $programsDir
    Set-ProcessVariable "MISKU_NV_NATIVE_EXE" $null
    Set-ProcessVariable "CI" "true"

    Push-Location $installDir
    try {
        & $pnpm add --save-exact --ignore-workspace $packagePath
        if ($LASTEXITCODE -ne 0) {
            throw "pnpm add del tarball fallo con codigo $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }

    $misku = Join-Path $installDir "node_modules\.bin\misku-nv.cmd"
    if (-not (Test-Path -LiteralPath $misku -PathType Leaf)) {
        throw "No encontre misku-nv en la instalacion temporal."
    }
    Invoke-Cli $misku @("--help") | Out-Host
    Invoke-Cli $misku @("--version") | Out-Host

    Invoke-Cli $misku @(
        $faviconUrl,
        "--name",
        "Favicon App",
        "--allow-http",
        "--no-open",
        "--json"
    ) | Out-Host

    $cases = @(
        @("https://example.com/alpha", "Same host alpha"),
        @("https://example.com/beta", "Same host beta"),
        @("https://example.org/repeated", "Repeated first"),
        @("https://example.org/repeated", "Repeated second")
    )
    foreach ($case in $cases) {
        Invoke-Cli $misku @(
            $case[0],
            "--name",
            $case[1],
            "--no-favicon",
            "--no-open",
            "--json"
        ) | Out-Host
    }

    if ($null -ne $faviconServer -and -not $faviconServer.HasExited) {
        Stop-Process -Id $faviconServer.Id -Force
        Wait-Process -Id $faviconServer.Id -ErrorAction SilentlyContinue
    }
    $faviconServer = $null
    Invoke-Cli $misku @("repair") | Out-Host

    $profiles = @(Get-InstalledProfiles $misku)
    if ($profiles.Count -ne 5) {
        throw "Esperaba 5 apps independientes y encontre $($profiles.Count)."
    }
    $uuidPattern = "^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
    $instanceIds = @($profiles | ForEach-Object { [string] $_.instance_id })
    if (
        @($instanceIds | Sort-Object -Unique).Count -ne 5 -or
        @($instanceIds | Where-Object { $_ -notmatch $uuidPattern }).Count -ne 0
    ) {
        throw "Las cinco apps no recibieron UUIDs validos y unicos."
    }
    $sameHost = @(
        $profiles |
            Where-Object { $_.url -in @("https://example.com/alpha", "https://example.com/beta") }
    )
    $duplicates = @($profiles | Where-Object { $_.url -eq "https://example.org/repeated" })
    if (
        $sameHost.Count -ne 2 -or
        $duplicates.Count -ne 2 -or
        @($duplicates.instance_id | Sort-Object -Unique).Count -ne 2
    ) {
        throw "Las rutas del mismo host o la URL duplicada no quedaron separadas."
    }

    $faviconProfiles = @($profiles | Where-Object { $_.url -eq $faviconUrl })
    if ($faviconProfiles.Count -ne 1) {
        throw "No se encontro la app local usada para validar el favicon."
    }
    $faviconUuid = [string] $faviconProfiles[0].instance_id
    $faviconIconRelative = ([string] $faviconProfiles[0].icon).Replace("\", "/")
    $assetUuidPattern = "[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}"
    $managedIconPattern = "^icons/$([Regex]::Escape($faviconUuid))-$assetUuidPattern\.ico$"
    if ($faviconIconRelative -notmatch $managedIconPattern) {
        throw "El favicon no quedo persistido con una ruta central versionada por UUID."
    }
    $centralIcon = [System.IO.Path]::GetFullPath(
        (Join-Path $miskuHome $faviconIconRelative.Replace("/", "\"))
    )
    $centralIconsRoot = [System.IO.Path]::GetFullPath(
        (Join-Path $miskuHome "icons")
    ).TrimEnd("\") + "\"
    if (
        -not $centralIcon.StartsWith(
            $centralIconsRoot,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        -not (Test-Path -LiteralPath $centralIcon -PathType Leaf)
    ) {
        throw "La ruta de favicon registrada no apunta al almacen central administrado."
    }

    Invoke-Cli $misku @(
        "update",
        $faviconUuid,
        "--name",
        "favicon app",
        "--json"
    ) | Out-Host
    $profiles = @(Get-InstalledProfiles $misku)
    if ($profiles.Count -ne 5) {
        throw "El rename por casing altero el numero de apps."
    }
    $faviconAfterRestart = @($profiles | Where-Object { $_.instance_id -eq $faviconUuid })
    if (
        $faviconAfterRestart.Count -ne 1 -or
        ([string] $faviconAfterRestart[0].icon).Replace("\", "/") -ne $faviconIconRelative -or
        -not (Test-Path -LiteralPath $centralIcon -PathType Leaf)
    ) {
        throw "El favicon versionado no persistio entre procesos del CLI y repair."
    }

    $manifests = @()
    $shortcuts = @()
    $shortcutShell = New-Object -ComObject WScript.Shell
    $programsRoot = [System.IO.Path]::GetFullPath($programsDir).TrimEnd("\") + "\"
    foreach ($profile in $profiles) {
        $uuid = [string] $profile.instance_id
        $appDir = Join-Path $miskuHome "apps\$uuid"
        $manifest = Join-Path $appDir "app.toml"
        $metadataPath = Join-Path $appDir "install.json"
        if (
            -not (Test-Path -LiteralPath $manifest -PathType Leaf) -or
            -not (Test-Path -LiteralPath $metadataPath -PathType Leaf) -or
            (Get-Content -LiteralPath $manifest -Raw) -notmatch [Regex]::Escape($uuid)
        ) {
            throw "Falta el manifiesto independiente de $uuid."
        }

        $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
        $shortcutPath = [System.IO.Path]::GetFullPath([string] $metadata.shortcutPath)
        if (
            [string] $metadata.instanceId -ne $uuid -or
            [System.IO.Path]::GetFullPath([string] $metadata.manifest) -ne [System.IO.Path]::GetFullPath($manifest) -or
            -not (Test-Path -LiteralPath ([string] $metadata.runtime) -PathType Leaf) -or
            -not $shortcutPath.StartsWith($programsRoot, [System.StringComparison]::OrdinalIgnoreCase) -or
            -not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)
        ) {
            throw "Los metadatos instalados no corresponden a $uuid."
        }

        $shortcut = $shortcutShell.CreateShortcut($shortcutPath)
        if (
            -not (Test-Path -LiteralPath $shortcut.TargetPath -PathType Leaf) -or
            $shortcut.Arguments -notmatch [Regex]::Escape($uuid) -or
            $shortcut.Arguments -notmatch [Regex]::Escape($manifest)
        ) {
            throw "El acceso directo no apunta a su app separada: $shortcutPath"
        }

        if ($uuid -eq $faviconUuid) {
            $installedIcon = Join-Path $appDir "icons\$uuid.ico"
            $shortcutIcon = ([string] $shortcut.IconLocation -replace ",\s*\d+$", "").Trim('"')
            if (
                -not (Test-Path -LiteralPath $centralIcon -PathType Leaf) -or
                -not (Test-Path -LiteralPath $installedIcon -PathType Leaf) -or
                [System.IO.Path]::GetFullPath($shortcutIcon) -ne
                    [System.IO.Path]::GetFullPath($installedIcon)
            ) {
                throw "El favicon persistente no se aplico al acceso directo de $uuid."
            }
        }
        $manifests += [System.IO.Path]::GetFullPath($manifest)
        $shortcuts += $shortcutPath
    }

    if (
        @($manifests | Sort-Object -Unique).Count -ne 5 -or
        @($shortcuts | Sort-Object -Unique).Count -ne 5 -or
        @(Get-ChildItem -LiteralPath $programsDir -Filter "*.lnk" -File).Count -ne 5
    ) {
        throw "No se materializaron exactamente 5 manifiestos y 5 accesos directos."
    }

    Write-Output "Smoke test OK: 5 UUIDs, 5 manifiestos, 5 accesos directos y favicon persistente."
    Write-Output "SmokePackage=$packagePath"
} finally {
    if ($null -ne $faviconServer -and -not $faviconServer.HasExited) {
        Stop-Process -Id $faviconServer.Id -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $faviconServer.Id -ErrorAction SilentlyContinue
    }
    if ($null -ne $shortcutShell) {
        [void] [System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcutShell)
    }
    foreach ($name in $environmentNames) {
        Set-ProcessVariable $name $originalEnvironment[$name]
    }
    Remove-SafeTestRoot $testRoot $temporaryRoot
}
