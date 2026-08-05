$ErrorActionPreference = "Stop"

$devScript = Join-Path $PSScriptRoot "dev.ps1"
$repoConfig = Join-Path (Split-Path -Parent $PSScriptRoot) "apps.toml"
$cliArgs = @($args)

if ($cliArgs.Count -eq 0) {
    $cliArgs = @("--help")
}

$hasExplicitConfig = $false
foreach ($argument in $cliArgs) {
    if ($argument -in @("--config", "-c") -or
        $argument.StartsWith("--config=", [System.StringComparison]::Ordinal)) {
        $hasExplicitConfig = $true
        break
    }
}

if (-not $hasExplicitConfig) {
    if (-not (Test-Path -LiteralPath $repoConfig -PathType Leaf)) {
        throw "No encontre la configuracion de desarrollo: $repoConfig"
    }
    $cliArgs = @("--config", $repoConfig) + $cliArgs
}

$cargoArgs = @("run", "-p", "misku-native-views", "--") + $cliArgs
& $devScript @cargoArgs

if ($LASTEXITCODE) {
    exit $LASTEXITCODE
}
