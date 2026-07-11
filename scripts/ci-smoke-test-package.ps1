param(
    [Parameter(Mandatory = $true)]
    [Alias("PackagePath")]
    [string] $Package
)

$ErrorActionPreference = "Stop"

if (Test-Path -LiteralPath $Package) {
    $installTarget = (Resolve-Path -LiteralPath $Package).Path
} else {
    $installTarget = $Package
}

$globalBinDir = Join-Path $env:TEMP "misku-native-view-cli-pnpm-bin"
$testDir = Join-Path $env:TEMP "misku-native-view-cli-smoke"
$testConfig = Join-Path $testDir "apps.toml"
$originalPath = $env:PATH
$previousGlobalBinDir = $null

try {
    $previousGlobalBinDir = (pnpm config get global-bin-dir 2>$null).Trim()
} catch {
    $previousGlobalBinDir = $null
}

try {
    New-Item -ItemType Directory -Path $globalBinDir -Force | Out-Null
    New-Item -ItemType Directory -Path $testDir -Force | Out-Null

    $env:PATH = "$globalBinDir;$env:PATH"
    pnpm config set global-bin-dir $globalBinDir
    pnpm install -g $installTarget
    if ($LASTEXITCODE -ne 0) {
        throw "pnpm install -g fallo con codigo $LASTEXITCODE"
    }

    $misku = Join-Path $globalBinDir "misku-nv.cmd"
    if (-not (Test-Path -LiteralPath $misku)) {
        throw "No encontre el comando instalado: $misku"
    }

    & $misku --help
    if ($LASTEXITCODE -ne 0) {
        throw "misku-nv --help fallo con codigo $LASTEXITCODE"
    }

    & $misku "https://github.com" "--name" "GitHub" "--no-open" "--config" $testConfig
    if ($LASTEXITCODE -ne 0) {
        throw "misku-nv URL smoke test fallo con codigo $LASTEXITCODE"
    }

    $listOutput = & $misku "--list" "--config" $testConfig
    if ($LASTEXITCODE -ne 0) {
        throw "misku-nv --list fallo con codigo $LASTEXITCODE"
    }

    $listOutput | Write-Output
    if (($listOutput -join "`n") -notmatch "github\s+GitHub\s+https://github\.com/") {
        throw "La salida de --list no contiene el perfil esperado para GitHub."
    }

    Write-Output "Smoke test OK: $installTarget"
} finally {
    $env:PATH = $originalPath

    if ([string]::IsNullOrWhiteSpace($previousGlobalBinDir) -or $previousGlobalBinDir -eq "undefined" -or $previousGlobalBinDir -eq "null") {
        pnpm config delete global-bin-dir 2>$null | Out-Null
    } else {
        pnpm config set global-bin-dir $previousGlobalBinDir 2>$null | Out-Null
    }
}
