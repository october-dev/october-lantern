# Creates what a Windows release publishes: the zip the in-app updater installs (it finds it by
# name and requires exactly these files) and, when Inno Setup is installed, the installer.
# Does not publish anything.
#   pwsh scripts/package-windows.ps1
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $Root 'build\windows'
$Arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'aarch64' } else { 'x86_64' }
# Dictation's speech runtime (whisper-cli.exe) ships next to Lantern, which runs it from there;
# its model is downloaded on first use. Keep desktop/src/updates.rs (FILES) and
# windows/installer.iss in step with this list.
$Files = 'october-lantern.exe', 'lantern-engine.exe', 'october-bus.exe', 'october-lantern.png', 'LICENSE', 'OCTOBER-BUS-LICENSE', 'OCTOBER-BUS-COMMIT', 'whisper-cli.exe', 'WHISPER-LICENSE'
foreach ($file in $Files) {
    if (-not (Test-Path (Join-Path $Dest $file))) { throw "Missing $file; run scripts/build-windows.ps1, scripts/build-bus-windows.ps1 and scripts/build-whisper-windows.ps1 first." }
}
# Signed before packing when a certificate is configured (see sign-windows.ps1).
& (Join-Path $PSScriptRoot 'sign-windows.ps1') @($Files | Where-Object { $_ -like '*.exe' } | ForEach-Object { Join-Path $Dest $_ })
$Name = "October-Lantern-windows-$Arch.zip"
$Out = Join-Path $Root "build\$Name"
Remove-Item $Out -ErrorAction SilentlyContinue
Compress-Archive -Path ($Files | ForEach-Object { Join-Path $Dest $_ }) -DestinationPath $Out
$Hash = (Get-FileHash -Algorithm SHA256 $Out).Hash.ToLower()
Set-Content -Path "$Out.sha256" -Value "$Hash  $Name" -NoNewline -Encoding ascii
Write-Host $Out

$Iscc = @((Get-Command iscc -ErrorAction SilentlyContinue).Source, "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if ($Iscc) {
    $Version = (Select-String -Path (Join-Path $Root 'engine\Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    & $Iscc "/DAppVersion=$Version" "/DArch=$Arch" (Join-Path $Root 'windows\installer.iss')
    if ($LASTEXITCODE -ne 0) { throw 'building the installer failed' }
    $Setup = Join-Path $Root "build\October-Lantern-windows-$Arch-setup.exe"
    & (Join-Path $PSScriptRoot 'sign-windows.ps1') $Setup
    $SetupHash = (Get-FileHash -Algorithm SHA256 $Setup).Hash.ToLower()
    Set-Content -Path "$Setup.sha256" -Value "$SetupHash  $(Split-Path -Leaf $Setup)" -NoNewline -Encoding ascii
    Write-Host $Setup
}
else {
    Write-Host 'Inno Setup not found; skipped the installer (the zip is complete on its own).'
}

# What scripts/publish-windows.sh checks before publishing: the version, the commit, where the
# updater looks, and whether every program is signed.
$Programs = @($Files | Where-Object { $_ -like '*.exe' } | ForEach-Object { Join-Path $Dest $_ })
if ($Setup) { $Programs += $Setup }
$Signed = @($Programs | Where-Object { (Get-AuthenticodeSignature $_).Status -eq 'Valid' }).Count -eq $Programs.Count
$VersionNow = (Select-String -Path (Join-Path $Root 'engine\Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
@{
    version  = $VersionNow
    commit   = (git -C $Root rev-parse HEAD)
    releases = $env:LANTERN_RELEASES_REPO
    signed   = $Signed
} | ConvertTo-Json | Set-Content -Path (Join-Path $Root 'build\release-info.json') -Encoding utf8
