[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$OutputPath,

    [string]$Sizes = "16,24,32,48,64,128,256",

    [string]$Background
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$scriptPath = Join-Path $PSScriptRoot "convert-icon.py"

$pythonCandidates = @()
if ($env:PYTHON) {
    $pythonCandidates += $env:PYTHON
}

$bundledPython = Join-Path $env:USERPROFILE ".cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe"
$pythonCandidates += $bundledPython
$pythonCandidates += "python"
$pythonCandidates += "py"

$python = $null
foreach ($candidate in $pythonCandidates) {
    try {
        $command = Get-Command $candidate -ErrorAction Stop
        $python = $command.Source
        break
    }
    catch {
    }
}

if (-not $python) {
    throw "No encontre Python. Instala Python o define `$env:PYTHON con la ruta a python.exe."
}

$argsList = @($scriptPath, $InputPath)
if ($OutputPath) {
    $argsList += $OutputPath
}
$argsList += "--sizes"
$argsList += $Sizes
if ($Background) {
    $argsList += "--background"
    $argsList += $Background
}

Push-Location $repoRoot
try {
    if ((Split-Path -Leaf $python).ToLowerInvariant() -eq "py.exe") {
        & $python -3 @argsList
    }
    else {
        & $python @argsList
    }
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    Pop-Location
}
