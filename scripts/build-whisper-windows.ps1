# Build dictation's speech runtime for a Windows release: whisper.cpp's whisper-cli.exe, from the
# same pinned source as scripts/setup-dictation.ps1, into build\windows next to Lantern with its
# license (WHISPER-LICENSE). One self-contained program: static libraries and C runtime, no OpenMP,
# so it needs nothing but Windows' own DLLs. The model is downloaded by Lantern on first use.
# Needs git, CMake and the Visual Studio C++ build tools.
#   pwsh scripts/build-whisper-windows.ps1 [-Verify]
# -Verify also downloads the model (checked against its SHA-256) and transcribes whisper.cpp's
# sample recording with the built program.
param([switch] $Verify)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$Tag = 'v1.9.5'
$Commit = 'd1be6fde11ac6e0407606b4e42fe72d34add8037'
$ModelSum = '60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe'
$Source = Join-Path $Root 'build\whisper-src'
$Build = Join-Path $Root 'build\whisper-release-build'
# The built program and its license, kept between runs (CI caches this folder).
$Out = Join-Path $Root 'build\whisper-out'
$Dest = Join-Path $Root 'build\windows'

function Invoke-Checked([string] $Program, [string[]] $Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program $($Arguments -join ' ') failed" }
}

# The Visual Studio tools: vswhere finds CMake and dumpbin when they aren't on PATH.
$VsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$Vs = if (Test-Path $VsWhere) { & $VsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath } else { $null }

$Stamp = Join-Path $Out 'COMMIT'
$Fresh = (Test-Path (Join-Path $Out 'whisper-cli.exe')) -and (Test-Path $Stamp) -and ((Get-Content $Stamp -Raw).Trim() -eq $Commit)
if (-not $Fresh) {
    $CMake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
    if (-not $CMake -and $Vs) {
        $Candidate = Join-Path "$Vs" 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
        if (Test-Path $Candidate) { $CMake = $Candidate }
    }
    if (-not $CMake) { throw 'Install CMake (or the Visual Studio C++ build tools).' }

    # Exactly the pinned source: the tag's commit, checked by hash.
    if (-not (Test-Path (Join-Path $Source '.git'))) {
        Invoke-Checked git @('clone', '--quiet', '--depth', '1', '--branch', $Tag, 'https://github.com/ggml-org/whisper.cpp.git', $Source)
    }
    if ((& git -C $Source rev-parse HEAD) -ne $Commit) { throw 'Unexpected whisper.cpp source revision.' }

    Remove-Item -Recurse -Force $Build -ErrorAction SilentlyContinue
    # The static C runtime (MultiThreaded) and no OpenMP: no vcruntime140.dll or vcomp140.dll to ship.
    # GGML_NATIVE=OFF builds for any x86-64 with AVX2 rather than for this machine's processor.
    Invoke-Checked $CMake @('-S', $Source, '-B', $Build,
        '-DBUILD_SHARED_LIBS=OFF', '-DWHISPER_BUILD_TESTS=OFF', '-DWHISPER_BUILD_SERVER=OFF', '-DWHISPER_CURL=OFF', '-DWHISPER_SDL2=OFF',
        '-DGGML_NATIVE=OFF', '-DGGML_OPENMP=OFF',
        '-DCMAKE_POLICY_DEFAULT_CMP0091=NEW', '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded')
    Invoke-Checked $CMake @('--build', $Build, '--config', 'Release', '--target', 'whisper-cli', '--parallel')
    $Built = @((Join-Path $Build 'bin\Release\whisper-cli.exe'), (Join-Path $Build 'bin\whisper-cli.exe')) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Built) { throw 'whisper-cli.exe was not built.' }

    Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $Out | Out-Null
    Copy-Item $Built (Join-Path $Out 'whisper-cli.exe')
    # whisper.cpp's license, and ggml's when the source carries its own.
    $Notice = "whisper-cli.exe is built from whisper.cpp $Tag ($Commit), https://github.com/ggml-org/whisper.cpp`r`n`r`n"
    $Notice += Get-Content (Join-Path $Source 'LICENSE') -Raw
    $Ggml = Join-Path $Source 'ggml\LICENSE'
    if (Test-Path $Ggml) { $Notice += "`r`n`r`nggml (ggml\LICENSE):`r`n`r`n" + (Get-Content $Ggml -Raw) }
    [System.IO.File]::WriteAllText((Join-Path $Out 'WHISPER-LICENSE'), $Notice)
    Set-Content -Path $Stamp -Value $Commit -Encoding ascii -NoNewline
}
$Cli = Join-Path $Out 'whisper-cli.exe'

# Only Windows' own DLLs: anything from a redistributable would be missing on someone's PC.
$Dumpbin = (Get-Command dumpbin -ErrorAction SilentlyContinue).Source
if (-not $Dumpbin -and $Vs) {
    $Dumpbin = Get-ChildItem -Path (Join-Path "$Vs" 'VC\Tools\MSVC') -Recurse -Filter dumpbin.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match 'Hostx64\\x64' } | Select-Object -First 1 -ExpandProperty FullName
}
if ($Dumpbin) {
    $Dependents = & $Dumpbin /nologo /dependents $Cli | ForEach-Object { $_.Trim() } | Where-Object { $_ -match '^\S+\.dll$' }
    Write-Host "whisper-cli.exe uses: $($Dependents -join ', ')"
    $Foreign = @($Dependents | Where-Object { $_ -match '^(vcruntime|msvcp|vcomp|concrt|libomp|ggml|whisper)' -or -not (Test-Path (Join-Path "$env:SystemRoot\System32" $_)) })
    if ($Foreign.Count) { throw "whisper-cli.exe needs DLLs Windows doesn't have: $($Foreign -join ', ')" }
}
else {
    Write-Warning 'dumpbin not found; skipped checking which DLLs whisper-cli.exe needs.'
}

& $Cli --help *> $null
if ($LASTEXITCODE -ne 0) { throw 'the built whisper-cli.exe does not run' }

if ($Verify) {
    $Speech = Join-Path $Root 'build\speech-test'
    New-Item -ItemType Directory -Force -Path $Speech | Out-Null
    $Model = Join-Path $Speech 'ggml-base.bin'
    if (-not (Test-Path $Model) -or (Get-FileHash -Algorithm SHA256 $Model).Hash.ToLower() -ne $ModelSum) {
        $Part = "$Model.part"
        Invoke-WebRequest -Uri 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin' -OutFile $Part -UseBasicParsing
        if ((Get-FileHash -Algorithm SHA256 $Part).Hash.ToLower() -ne $ModelSum) { Remove-Item $Part; throw 'The speech model download did not match its checksum.' }
        Move-Item $Part $Model -Force
    }
    $Sample = Join-Path $Source 'samples\jfk.wav'
    if (-not (Test-Path $Sample)) {
        Invoke-Checked git @('clone', '--quiet', '--depth', '1', '--branch', $Tag, 'https://github.com/ggml-org/whisper.cpp.git', $Source)
    }
    $Text = (& $Cli -m $Model -f $Sample -l en -nt -np) -join ' '
    if ($LASTEXITCODE -ne 0) { throw 'whisper-cli.exe could not transcribe the sample' }
    Write-Host "Transcribed: $Text"
    if ($Text -notmatch 'ask not what your country') { throw 'whisper-cli.exe transcribed the sample wrongly' }
}

New-Item -ItemType Directory -Force -Path $Dest | Out-Null
Copy-Item $Cli (Join-Path $Dest 'whisper-cli.exe') -Force
Copy-Item (Join-Path $Out 'WHISPER-LICENSE') (Join-Path $Dest 'WHISPER-LICENSE') -Force
Write-Host "Bundled whisper-cli.exe (whisper.cpp $Tag) in $Dest."
