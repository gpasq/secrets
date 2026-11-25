# Packages target\release\secrets.exe into target\installer\Secrets-<version>-<arch>-setup.exe.
# Run it with `just installer`, which builds the release exe first.
# Requires Inno Setup 6.3+ (scoop install extras/inno-setup).
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$arch = if ((rustc -vV | Select-String '^host:') -match 'aarch64') { 'arm64' } else { 'x64' }

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
              "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw 'Inno Setup (ISCC.exe) not found. Install it with: scoop install extras/inno-setup' }

& $iscc /Q "/DAppVersion=$version" "/DArch=$arch" "/DExe=$root\target\release\secrets.exe" installer\Secrets.iss
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
"Built target\installer\Secrets-$version-$arch-setup.exe"
