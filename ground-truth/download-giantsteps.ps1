# GiantSteps key-dataset audio downloader (PowerShell port of audio_dl.sh).
#
# Downloads the 604 Beatport preview mp3s referenced by the md5/ directory,
# verifies each against its hash, and retries from the JKU backup mirror on
# mismatch. Run from anywhere; pass the dataset root as an argument.
#
#   powershell -ExecutionPolicy Bypass -File download-giantsteps.ps1 .\giantsteps-key
#
# Idempotent: files that already exist and hash correctly are skipped.

param(
    [Parameter(Mandatory = $true)]
    [string]$DatasetRoot
)

$ErrorActionPreference = 'SilentlyContinue'

$md5Dir   = Join-Path $DatasetRoot 'md5'
$audioDir = Join-Path $DatasetRoot 'audio'
$baseUrl  = 'https://www.cp.jku.at/datasets/giantsteps/backup/'
$backupUrl = 'https://geo-samples.beatport.com/lofi/'

New-Item -ItemType Directory -Force $audioDir | Out-Null

# Helper: safely compute MD5, returning $null on any error (file lock, etc.)
function Safe-Hash($path) {
    $h = Get-FileHash -Path $path -Algorithm MD5 -ErrorAction SilentlyContinue
    if ($h) { return $h.Hash.ToLower() } else { return $null }
}

# Helper: download a URL to a file, returning $true on success.
function Try-Download($url, $dest) {
    try {
        Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing -TimeoutSec 60 -ErrorAction Stop
        return $true
    } catch {
        return $false
    }
}

$files = Get-ChildItem $md5Dir -File
$ok = 0; $backup = 0; $errors = 0; $skipped = 0; $i = 0
$total = $files.Count

foreach ($f in $files) {
    $i++
    $mp3 = "$($f.BaseName).mp3"
    $dest = Join-Path $audioDir $mp3
    $expected = (Get-Content $f.FullName).Trim()

    # Skip if already downloaded and hash matches.
    if (Test-Path $dest) {
        Start-Sleep -Milliseconds 50  # let any file handle release
        $hash = Safe-Hash $dest
        if ($hash -and ($hash -eq $expected)) {
            $skipped++; continue
        }
        # Hash mismatch or locked — delete and re-download.
        Remove-Item -Force $dest -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 50
    }

    # Try JKU backup first, then Beatport.
    $fromBackup = $false
    $downloaded = Try-Download "$baseUrl$mp3" $dest
    if (-not $downloaded) {
        $downloaded = Try-Download "$backupUrl$mp3" $dest
        $fromBackup = $true
    }

    # Verify hash.
    if ($downloaded) {
        Start-Sleep -Milliseconds 50
        $hash = Safe-Hash $dest
        if ($hash -and ($hash -eq $expected)) {
            $ok++
            if ($fromBackup) { $backup++ }
        } else {
            # Hash mismatch — try Beatport as a second attempt.
            Remove-Item -Force $dest -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 50
            $downloaded2 = Try-Download "$backupUrl$mp3" $dest
            if ($downloaded2) {
                Start-Sleep -Milliseconds 50
                $hash2 = Safe-Hash $dest
                if ($hash2 -and ($hash2 -eq $expected)) {
                    $ok++; $backup++
                } else {
                    Remove-Item -Force $dest -ErrorAction SilentlyContinue
                    $errors++
                    Write-Host "FAIL $mp3 (hash mismatch)" -ForegroundColor Red
                }
            } else {
                Remove-Item -Force $dest -ErrorAction SilentlyContinue
                $errors++
                Write-Host "FAIL $mp3 (no source)" -ForegroundColor Red
            }
        }
    } else {
        $errors++
        Write-Host "FAIL $mp3 (no source)" -ForegroundColor Red
    }

    if ($i % 25 -eq 0) { Write-Host "[$i/$total] ok=$ok backup=$backup skipped=$skipped err=$errors" }
}

Write-Host ""
Write-Host "Summary: downloaded=$ok (from backup=$backup)  already-present=$skipped  errors=$errors  total=$total"
