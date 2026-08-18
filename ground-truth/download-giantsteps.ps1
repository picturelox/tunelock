# GiantSteps key-dataset audio downloader (PowerShell port of audio_dl.sh).
#
# Downloads the 604 Beatport preview mp3s referenced by the md5/ directory,
# verifies each against its hash, and retries from the JKU backup mirror on
# mismatch. Run from anywhere; pass the dataset root as an argument.
#
#   pwsh download-giantsteps.ps1 .\giantsteps-key
#
# Idempotent: files that already exist and hash correctly are skipped.

param(
    [Parameter(Mandatory = $true)]
    [string]$DatasetRoot
)

$md5Dir   = Join-Path $DatasetRoot 'md5'
$audioDir = Join-Path $DatasetRoot 'audio'
$baseUrl  = 'https://www.cp.jku.at/datasets/giantsteps/backup/'
$backupUrl = 'https://geo-samples.beatport.com/lofi/'

New-Item -ItemType Directory -Force $audioDir | Out-Null

$files = Get-ChildItem $md5Dir -File
$ok = 0; $backup = 0; $errors = 0; $skipped = 0; $i = 0
$total = $files.Count

foreach ($f in $files) {
    $i++
    $mp3 = "$($f.BaseName).mp3"
    $dest = Join-Path $audioDir $mp3
    $expected = (Get-Content $f.FullName).Trim()

    if ((Test-Path $dest) -and (Get-FileHash $dest -Algorithm MD5).Hash.ToLower() -eq $expected) {
        $skipped++; continue
    }

    $url = "$baseUrl$mp3"
    try {
        Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing -TimeoutSec 60
    } catch {
        # try backup immediately on request failure
        try { Invoke-WebRequest -Uri "$backupUrl$mp3" -OutFile $dest -UseBasicParsing -TimeoutSec 60 } catch {}
    }

    if ((Test-Path $dest) -and (Get-FileHash $dest -Algorithm MD5).Hash.ToLower() -eq $expected) {
        $ok++
    } else {
        try { Invoke-WebRequest -Uri "$backupUrl$mp3" -OutFile $dest -UseBasicParsing -TimeoutSec 60 } catch {}
        if ((Test-Path $dest) -and (Get-FileHash $dest -Algorithm MD5).Hash.ToLower() -eq $expected) {
            $ok++; $backup++
        } else {
            Remove-Item -Force $dest -ErrorAction SilentlyContinue
            $errors++
            Write-Host "FAIL $mp3" -ForegroundColor Red
        }
    }
    if ($i % 25 -eq 0) { Write-Host "[$i/$total] ok=$ok backup=$backup skipped=$skipped err=$errors" }
}

Write-Host ""
Write-Host "Summary: downloaded=$ok (from backup=$backup)  already-present=$skipped  errors=$errors  total=$total"
