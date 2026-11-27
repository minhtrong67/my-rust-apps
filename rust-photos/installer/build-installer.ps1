# Builds the Windows installer: dist\RustPhotos-Setup-<version>.exe
# Needs: Rust (MSVC), and Inno Setup 6  ->  winget install JRSoftware.InnoSetup
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$ver = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Building Rust Photos $ver ..."

# Static C runtime: the app runs on a PC without the Visual C++ Redistributable.
# A separate target folder keeps your normal development builds untouched (this one is built from scratch once).
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release -p rust-photos --target-dir target-dist
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$candidates = @(
  "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
  "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
  "${env:LOCALAPPDATA}\Programs\Inno Setup 6\ISCC.exe"
)
$iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup" }

& $iscc "/DAppVersion=$ver" "installer\rust-photos.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed" }
Write-Host "Done: dist\RustPhotos-Setup-$ver.exe"
