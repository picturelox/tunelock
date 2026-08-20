# GiantSteps-MTG key dataset downloader (Zenodo DOI 10.5281/zenodo.1101082)
#
# This is the larger MTG dataset (~1,486 tracks) used for CNN training.
# It is separate from the GiantSteps-key test set (604 tracks).
#
# The dataset is distributed as a ZIP archive on Zenodo. This script:
#   1. Downloads the ZIP from the Zenodo API
#   2. Extracts it to the specified directory
#   3. Verifies the file count
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File download-giantsteps-mtg.ps1 .\giantsteps-mtg-key
#
# Note: The download is ~2 GB. Be patient.

param(
    [Parameter(Mandatory = $true)]
    [string]$DatasetRoot
)

$ErrorActionPreference = 'Stop'

# Zenodo record for GiantSteps-MTG key dataset
# DOI: 10.5281/zenodo.1101082
$zenodoId = '1101082'
$apiUrl = "https://zenodo.org/api/records/$zenodoId"

Write-Host "Fetching Zenodo record $zenodoId..." -ForegroundColor Cyan
$record = Invoke-RestMethod -Uri $apiUrl -Method Get

# Find the ZIP file in the record's files
$zipFile = $record.files | Where-Object { $_.key -like '*.zip' } | Select-Object -First 1
if (-not $zipFile) {
    # If no ZIP, look for any large file
    $zipFile = $record.files | Sort-Object size -Descending | Select-Object -First 1
}

if (-not $zipFile) {
    Write-Host "ERROR: No downloadable file found in Zenodo record $zenodoId" -ForegroundColor Red
    exit 1
}

$downloadUrl = $zipFile.links.self
$fileName = $zipFile.key
$fileSize = [math]::Round($zipFile.size / 1MB, 1)

Write-Host "Found: $fileName ($fileSize MB)" -ForegroundColor Green
Write-Host "URL: $downloadUrl" -ForegroundColor Gray

# Create dataset directory
New-Item -ItemType Directory -Force $DatasetRoot | Out-Null
$zipPath = Join-Path $DatasetRoot $fileName

# Download
if (Test-Path $zipPath) {
    Write-Host "ZIP already exists: $zipPath" -ForegroundColor Yellow
    Write-Host "Skipping download. Delete the file to re-download." -ForegroundColor Gray
} else {
    Write-Host "Downloading..." -ForegroundColor Cyan
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
    Write-Host "Download complete." -ForegroundColor Green
}

# Extract
Write-Host "Extracting to $DatasetRoot..." -ForegroundColor Cyan
Expand-Archive -Path $zipPath -DestinationPath $DatasetRoot -Force
Write-Host "Extraction complete." -ForegroundColor Green

# Verify
$audioFiles = Get-ChildItem -Path $DatasetRoot -Recurse -Include '*.mp3','*.flac','*.wav' -ErrorAction SilentlyContinue
Write-Host ""
Write-Host "Summary: Found $($audioFiles.Count) audio files in $DatasetRoot" -ForegroundColor Green

# Check for key annotations
$keyFiles = Get-ChildItem -Path $DatasetRoot -Recurse -Include '*.key' -ErrorAction SilentlyContinue
Write-Host "Key annotations: $($keyFiles.Count) files" -ForegroundColor Green

if ($audioFiles.Count -lt 1000) {
    Write-Host ""
    Write-Host "WARNING: Expected ~1,486 audio files but found $($audioFiles.Count)." -ForegroundColor Yellow
    Write-Host "The extraction may have been incomplete. Check the ZIP file." -ForegroundColor Yellow
}
