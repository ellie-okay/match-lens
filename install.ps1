# Match Lens — one-command installer
# Usage: irm https://raw.githubusercontent.com/ellie-okay/match-lens/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
$repo = "ellie-okay/match-lens"

Write-Host "Fetching latest Match Lens release..." -ForegroundColor Cyan

$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$asset = $release.assets | Where-Object { $_.name -like "*-setup.exe" } | Select-Object -First 1

if (-not $asset) {
    Write-Error "No installer found in the latest release. Check https://github.com/$repo/releases"
    exit 1
}

$installer = "$env:TEMP\match-lens-setup.exe"
Write-Host "Downloading $($asset.name)..." -ForegroundColor Yellow
Invoke-WebRequest $asset.browser_download_url -OutFile $installer

Write-Host "Running installer..." -ForegroundColor Yellow
Start-Process $installer -Wait

Remove-Item $installer -Force
Write-Host "Done. Launch Match Lens from your system tray." -ForegroundColor Green
