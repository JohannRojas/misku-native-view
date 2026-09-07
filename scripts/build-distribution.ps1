$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-StrictMode -Version Latest
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
$previousSkip = $env:MISKU_SKIP_NATIVE_BUILD
try {
    if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $repoRoot 'target' }
    . ./scripts/build-env.ps1
    Initialize-MsvcBuildEnvironment | Out-Null
    node scripts/release-policy.cjs
    if ($LASTEXITCODE -ne 0) { throw 'Versiones inconsistentes' }
    pnpm exec tauri build --ci --target x86_64-pc-windows-msvc --bundles nsis -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'La compilación del instalador falló' }
    $distribution = Join-Path $repoRoot 'artifacts/distribution'
    New-Item -ItemType Directory -Path $distribution -Force | Out-Null
    # Only the known output leaves of this package are replaced; other artifacts stay.
    Get-ChildItem -LiteralPath $distribution -File | Where-Object { $_.Name -match '^(misku-native-view-cli-.*\.tgz|Misku-Native-Views-.*-setup\.exe|SHA256SUMS|build-info\.json)$' } | Remove-Item -Force
    $version = (Get-Content package.json -Raw | ConvertFrom-Json).version
    $installers = @(Get-ChildItem (Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-msvc/release/bundle/nsis') -Filter "*${version}*x64*.exe")
    if ($installers.Count -ne 1) { throw 'Se esperaba un instalador x64 de la versión actual' }
    Copy-Item -LiteralPath $installers[0].FullName -Destination (Join-Path $distribution "Misku-Native-Views-$version-x64-setup.exe")
    $env:MISKU_SKIP_NATIVE_BUILD = '1'
    pnpm pack --pack-destination $distribution
    if ($LASTEXITCODE -ne 0) { throw 'pnpm pack falló' }
    if (@(Get-ChildItem $distribution -Filter '*.tgz').Count -ne 1) { throw 'Se esperaba un solo tarball' }
    $commit = if ($env:GITHUB_SHA) { $env:GITHUB_SHA } else { (git rev-parse HEAD) }
    @{ version = $version; commit = $commit; target = 'x86_64-pc-windows-msvc'; runtimeSha256 = (Get-FileHash runtime/misku-native-views.exe -Algorithm SHA256).Hash.ToLowerInvariant(); authenticode = (Get-AuthenticodeSignature runtime/misku-native-views.exe).Status.ToString() } | ConvertTo-Json | Set-Content (Join-Path $distribution 'build-info.json') -Encoding utf8NoBOM
    $lines = Get-ChildItem -LiteralPath $distribution -File | Sort-Object Name | ForEach-Object { "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $($_.Name)" }
    $lines | Set-Content (Join-Path $distribution 'SHA256SUMS') -Encoding ascii
} finally { $env:MISKU_SKIP_NATIVE_BUILD = $previousSkip; Pop-Location }
