$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Este test de instalación solo se ejecuta en un runner efímero de GitHub Actions.' }
$version = (Get-Content package.json -Raw | ConvertFrom-Json).version
$installer = (Resolve-Path "artifacts/distribution/Misku-Native-Views-$version-x64-setup.exe").Path
$testRoot = Join-Path $env:RUNNER_TEMP "misku-installer-$([guid]::NewGuid().ToString('N'))"
$installPath = Join-Path $testRoot 'installed'
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
try {
    $process = Start-Process -FilePath $installer -ArgumentList "/S /D=$installPath" -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(120000)) { $process.Kill(); throw 'Timeout instalando' }
    if ($process.ExitCode -ne 0) { throw "Installer devolvió $($process.ExitCode)" }
    $executable = Join-Path $installPath 'misku-native-views.exe'
    $actual = (& node (Join-Path $PSScriptRoot 'read-native-version.cjs') $executable) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $actual.Trim() -ne "misku-nv $version") { throw "Versión instalada incorrecta: $actual" }
    $sourceHash = (Get-FileHash runtime/misku-native-views.exe).Hash
    if ((Get-FileHash $executable).Hash -ne $sourceHash) { throw 'El instalador no contiene el mismo runtime probado en npm' }
    $uninstaller = @(Get-ChildItem -LiteralPath $installPath -Filter '*uninstall*.exe')
    if ($uninstaller.Count -ne 1) { throw 'No se encontró un único desinstalador' }
    $process = Start-Process -FilePath $uninstaller[0].FullName -ArgumentList '/S' -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(120000)) { $process.Kill(); throw 'Timeout desinstalando' }
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    while ((Test-Path -LiteralPath $executable) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 250 }
    if (Test-Path -LiteralPath $executable) { throw 'El desinstalador dejó el ejecutable instalado' }
    Write-Output 'Instalación, integridad y desinstalación verificadas.'
} finally {
    $resolved = [IO.Path]::GetFullPath($testRoot)
    $prefix = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\misku-installer-'
    if (-not $resolved.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Limpieza fuera del runner rechazada' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
