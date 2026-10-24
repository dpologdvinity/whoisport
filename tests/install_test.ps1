<#
.SYNOPSIS
    Tests for install.ps1. Run with:  pwsh tests/install_test.ps1 -Exe path\to\whoisport.exe

.DESCRIPTION
    No network: a fake release (a zip holding the given whoisport.exe, plus SHA256SUMS) is
    served from a temporary directory through file: URLs. Covers a normal install, a
    tampered download (must install nothing), a missing checksum file (must refuse), a
    release that does not exist, and the PATH edit (the user PATH is saved and restored).
#>
param([Parameter(Mandatory)][string]$Exe)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$installer = Join-Path $repo 'install.ps1'
$work = Join-Path ([IO.Path]::GetTempPath()) ("whoisport-ps-test-" + [Guid]::NewGuid().ToString('N'))
$failures = 0

# Report one test result.
function Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL ${Name}: $Detail"; $script:failures++ }
}

# A copy of `$Base` with the entries of `$Override` added or replaced (`+` on hashtables
# throws when a key repeats).
function Merge-Settings([hashtable]$Base, [hashtable]$Override) {
    $merged = $Base.Clone()
    foreach ($key in $Override.Keys) { $merged[$key] = $Override[$key] }
    return $merged
}

# Run install.ps1 in a fresh PowerShell process with the given settings; return its
# combined output and exit code. A separate process keeps each test independent.
function Invoke-Installer([hashtable]$Settings) {
    $saved = @{}
    foreach ($name in $Settings.Keys) { $saved[$name] = [Environment]::GetEnvironmentVariable($name); [Environment]::SetEnvironmentVariable($name, $Settings[$name]) }
    try {
        $shell = (Get-Process -Id $PID).Path
        # The installer is expected to write errors to stderr in the failure cases; under
        # 'Stop', PowerShell would turn that output of a child process into a fatal error here.
        $previous = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $output = & $shell -NoProfile -ExecutionPolicy Bypass -File $installer 2>&1 | Out-String
            $code = $LASTEXITCODE
        } finally {
            $ErrorActionPreference = $previous
        }
        return @{ Output = $output; Code = $code }
    } finally {
        foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
}

try {
    # --- a fake release ------------------------------------------------------------------
    $tag = 'v9.9.9'
    $releaseDir = Join-Path $work "releases\download\$tag"
    $zipName = "whoisport-$tag-x86_64-pc-windows-msvc.zip"
    New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
    $pack = Join-Path $work 'pack'
    New-Item -ItemType Directory -Force -Path $pack | Out-Null
    Copy-Item $Exe (Join-Path $pack 'whoisport.exe')
    $zip = Join-Path $releaseDir $zipName
    Compress-Archive -Path (Join-Path $pack 'whoisport.exe') -DestinationPath $zip
    $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
    Set-Content -Path (Join-Path $releaseDir 'SHA256SUMS') -Value "$hash  $zipName"
    $base = ([Uri](Join-Path $work 'releases')).AbsoluteUri
    $common = @{ WHOISPORT_BASE_URL = $base; WHOISPORT_VERSION = $tag; WHOISPORT_NO_MODIFY_PATH = '1' }

    # --- 1. a normal install -----------------------------------------------------------
    $dest = Join-Path $work 'bin1'
    $r = Invoke-Installer (Merge-Settings $common @{ WHOISPORT_INSTALL_DIR = $dest })
    $runs = (Test-Path (Join-Path $dest 'whoisport.exe')) -and ((& (Join-Path $dest 'whoisport.exe') --version) -match '^whoisport ')
    Check 'installs and the binary runs' ($r.Code -eq 0 -and $runs) $r.Output
    Check 'reports the checksum check' ($r.Output -match 'checksum OK') $r.Output

    # --- 2. a version without the v ------------------------------------------------------
    $dest = Join-Path $work 'bin2'
    $r = Invoke-Installer (Merge-Settings $common @{ WHOISPORT_VERSION = '9.9.9'; WHOISPORT_INSTALL_DIR = $dest })
    Check 'accepts a version without the v' ($r.Code -eq 0 -and (Test-Path (Join-Path $dest 'whoisport.exe'))) $r.Output

    # --- 3. a tampered download installs nothing -------------------------------------
    Copy-Item $zip "$zip.good"
    [IO.File]::AppendAllText($zip, 'tampered')
    $dest = Join-Path $work 'bin3'
    $r = Invoke-Installer (Merge-Settings $common @{ WHOISPORT_INSTALL_DIR = $dest })
    Check 'rejects a tampered download' ($r.Code -ne 0 -and $r.Output -match 'checksum mismatch' -and -not (Test-Path (Join-Path $dest 'whoisport.exe'))) $r.Output
    Move-Item -Force "$zip.good" $zip

    # --- 4. a missing SHA256SUMS refuses to install ----------------------------------
    $sumsFile = Join-Path $releaseDir 'SHA256SUMS'
    Move-Item $sumsFile "$sumsFile.bak"
    $dest = Join-Path $work 'bin4'
    $r = Invoke-Installer (Merge-Settings $common @{ WHOISPORT_INSTALL_DIR = $dest })
    Check 'refuses without SHA256SUMS' ($r.Code -ne 0 -and $r.Output -match 'unverified' -and -not (Test-Path (Join-Path $dest 'whoisport.exe'))) $r.Output
    Move-Item "$sumsFile.bak" $sumsFile

    # --- 5. a release that does not exist ----------------------------------------------
    $r = Invoke-Installer (Merge-Settings $common @{ WHOISPORT_VERSION = 'v0.0.1'; WHOISPORT_INSTALL_DIR = (Join-Path $work 'bin5') })
    Check 'fails clearly for an unknown release' ($r.Code -ne 0 -and $r.Output -match 'could not download') $r.Output

    # --- 6. the PATH edit (the real user PATH is saved and restored) ----------------------
    $originalPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    try {
        $dest = Join-Path $work 'bin6'
        $noFlag = $common.Clone(); $noFlag.Remove('WHOISPORT_NO_MODIFY_PATH')
        $r = Invoke-Installer (Merge-Settings $noFlag @{ WHOISPORT_INSTALL_DIR = $dest; WHOISPORT_NO_MODIFY_PATH = '' })
        $after = [Environment]::GetEnvironmentVariable('Path', 'User')
        Check 'adds the directory to the user PATH' (($after -split ';') -contains $dest) $r.Output
        $r = Invoke-Installer (Merge-Settings $noFlag @{ WHOISPORT_INSTALL_DIR = $dest; WHOISPORT_NO_MODIFY_PATH = '' })
        $again = [Environment]::GetEnvironmentVariable('Path', 'User')
        Check 'does not add it twice' ((($again -split ';') | Where-Object { $_ -eq $dest }).Count -eq 1) $again
    } finally {
        [Environment]::SetEnvironmentVariable('Path', $originalPath, 'User')
    }
    Check 'leaves the user PATH as it was' ([Environment]::GetEnvironmentVariable('Path', 'User') -eq $originalPath)
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($failures -eq 0) { Write-Host 'all installer tests passed' } else { Write-Host "$failures installer test(s) failed"; exit 1 }
