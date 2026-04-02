# Match Lens - Setup Script
# Run this once before building for the first time.
# Requires: internet connection, Windows 10/11

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Write-Host "=== Match Lens Setup ===" -ForegroundColor Cyan

# 1. Rust
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Write-Host "Installing Rust (rustup)..." -ForegroundColor Yellow
    $rustupInit = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest "https://win.rustup.rs/x86_64" -OutFile $rustupInit
    & $rustupInit -y --default-toolchain stable
    # Reload PATH so cargo is available
    $env:PATH += ";$env:USERPROFILE\.cargo\bin"
} else {
    Write-Host "Rust already installed." -ForegroundColor Green
    rustup update stable
}

# 2. Node packages
Write-Host "Installing Node packages..." -ForegroundColor Yellow
npm install

# 3. FFmpeg sidecar binary
$binDir = "src-tauri\binaries"
$ffmpegExe = "$binDir\ffmpeg-x86_64-pc-windows-msvc.exe"
New-Item -ItemType Directory -Force -Path $binDir | Out-Null

if (-not (Test-Path $ffmpegExe)) {
    Write-Host "Downloading FFmpeg..." -ForegroundColor Yellow

    # The bundled FFmpeg sidecar is used for desktop video capture and final
    # muxing. System audio is captured natively in the Rust backend via WASAPI.
    $ffmpegZipUrl = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
    $zipPath = "$env:TEMP\ffmpeg-release.zip"
    $extractDir = "$env:TEMP\ffmpeg-extract"

    Write-Host "Downloading FFmpeg (gyan.dev essentials build)..."
    Invoke-WebRequest $ffmpegZipUrl -OutFile $zipPath

    Expand-Archive $zipPath -DestinationPath $extractDir -Force

    # Find ffmpeg.exe inside the extracted folder (bin/ subdirectory)
    $extracted = Get-ChildItem "$extractDir" -Recurse -Filter "ffmpeg.exe" |
        Where-Object { $_.DirectoryName -like "*\bin" } |
        Select-Object -First 1
    if ($extracted) {
        Copy-Item $extracted.FullName $ffmpegExe
        Write-Host "FFmpeg installed to $ffmpegExe" -ForegroundColor Green
    } else {
        Write-Host "ERROR: Could not find ffmpeg.exe in the downloaded archive." -ForegroundColor Red
        exit 1
    }

    Remove-Item $zipPath -Force
    Remove-Item $extractDir -Recurse -Force
} else {
    Write-Host "FFmpeg already present." -ForegroundColor Green
}

# 4. Icons — generate all required sizes from the SVG source
Write-Host "Generating icons from src-tauri\icons\icon.svg..." -ForegroundColor Yellow
npx tauri icon "src-tauri\icons\icon.svg"
Write-Host "Icons generated." -ForegroundColor Green

# 5. Done
Write-Host ""
Write-Host "=== Setup complete ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "To run in development mode:  npm run tauri dev" -ForegroundColor White
Write-Host "To build a release binary:   npm run tauri build" -ForegroundColor White
Write-Host ""
Write-Host "FIRST LAUNCH CHECKLIST:" -ForegroundColor Yellow
Write-Host "  1. Start League of Legends client" -ForegroundColor White
Write-Host "  2. Launch Match Lens - it will appear in your system tray" -ForegroundColor White
Write-Host "  3. Start a game (Practice Tool works) to test recording" -ForegroundColor White
Write-Host "  4. After the game ends, the review window opens automatically" -ForegroundColor White
