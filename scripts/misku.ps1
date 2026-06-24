$ErrorActionPreference = "Stop"

$devScript = Join-Path $PSScriptRoot "dev.ps1"
$cliArgs = @($args)

if ($cliArgs.Count -eq 0) {
    $cliArgs = @("--help")
}

$cargoArgs = @("run", "-p", "misku-native-views", "--") + $cliArgs
& $devScript @cargoArgs

if ($LASTEXITCODE) {
    exit $LASTEXITCODE
}
