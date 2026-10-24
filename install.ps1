<#
.SYNOPSIS
    whoisport installer for Windows.

.DESCRIPTION
    Finds the release binary, downloads it, checks its SHA-256 against the release's
    SHA256SUMS file, and installs it. Nothing is installed if the check fails.

    irm https://github.com/dpologdvinity/whoisport/releases/latest/download/install.ps1 | iex

    Settings are environment variables, set before running the line above:
      WHOISPORT_VERSION         release to install, e.g. v0.5.7 (default: the latest)
      WHOISPORT_INSTALL_DIR     where to put whoisport.exe
                                (default: %LOCALAPPDATA%\Programs\whoisport)
      WHOISPORT_NO_MODIFY_PATH  set to 1 to leave your PATH alone

    By default the install directory is added to your user PATH (open a new terminal for
    it to take effect). Requires Windows 10 or 11 on a 64-bit x86 or ARM64 PC (ARM64 runs
    the x86-64 binary through Windows' built-in emulation).
#>

# Stop at the first error instead of carrying on with a half-finished install.
$ErrorActionPreference = 'Stop'

# Older Windows PowerShell defaults to protocols GitHub no longer accepts.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

# Download `$Uri` to `$OutFile`. A file: URI is copied instead (used by the tests, which serve
# a fake release from disk; PowerShell 7's Invoke-WebRequest cannot fetch file: URIs).
function Save-Url([string]$Uri, [string]$OutFile) {
    if ($Uri.StartsWith('file:')) {
        Copy-Item -LiteralPath ([Uri]$Uri).LocalPath -Destination $OutFile
    } else {
        Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $OutFile
    }
}

# Everything lives in a function, and the function is called on the last line, so a download
# that is cut off halfway can never run a partial script.
function Install-Whoisport {
    # Windows PowerShell 5 draws a progress bar that makes downloads many times slower.
    $ProgressPreference = 'SilentlyContinue'
    $repo = 'dpologdvinity/whoisport'
    # Overridden by tests to serve a fake release from disk.
    $base = if ($env:WHOISPORT_BASE_URL) { $env:WHOISPORT_BASE_URL } else { "https://github.com/$repo/releases" }

    # --- which release -------------------------------------------------------------
    $tag = $env:WHOISPORT_VERSION
    if ($tag) {
        if (-not $tag.StartsWith('v')) { $tag = "v$tag" }
    } else {
        # /releases/latest redirects to /releases/tag/<tag>; reading the redirect needs no
        # API call, so there is no rate limit.
        $request = [Net.HttpWebRequest]::Create("$base/latest")
        $request.AllowAutoRedirect = $false
        $response = $request.GetResponse()
        $location = $response.Headers['Location']
        $response.Close()
        if ($location -notmatch '/tag/(v[^/]+)$') {
            throw "could not find the latest release version (got: $location)"
        }
        $tag = $Matches[1]
    }

    $target = 'x86_64-pc-windows-msvc'
    $file = "whoisport-$tag-$target.zip"
    $url = "$base/download/$tag"
    Write-Host "whoisport-install: installing whoisport $tag for Windows"

    # --- download into a private temporary directory --------------------------------
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("whoisport-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $zip = Join-Path $tmp $file
        $sums = Join-Path $tmp 'SHA256SUMS'
        try {
            Save-Url "$url/$file" $zip
        } catch {
            throw "could not download $url/$file (does release $tag exist for Windows?)"
        }
        try {
            Save-Url "$url/SHA256SUMS" $sums
        } catch {
            throw "could not download $url/SHA256SUMS; refusing to install an unverified file"
        }

        # --- verify ------------------------------------------------------------------
        # SHA256SUMS lines look like `<hash>  <file name>` (or `<hash> *<file name>`).
        $expected = $null
        foreach ($line in Get-Content $sums) {
            $parts = $line -split '\s+', 2
            if ($parts.Count -eq 2 -and ($parts[1].TrimStart('*') -eq $file)) { $expected = $parts[0].ToLower() }
        }
        if (-not $expected) { throw "no checksum for $file in SHA256SUMS; refusing to install an unverified file" }
        $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
        if ($actual -ne $expected) {
            throw "checksum mismatch for $file (expected $expected, got $actual); nothing was installed"
        }
        Write-Host 'whoisport-install: checksum OK'

        # --- install -----------------------------------------------------------------
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $exe = Join-Path $tmp 'whoisport.exe'
        if (-not (Test-Path $exe)) { throw 'the archive did not contain whoisport.exe' }

        $dir = $env:WHOISPORT_INSTALL_DIR
        if (-not $dir) { $dir = Join-Path $env:LOCALAPPDATA 'Programs\whoisport' }
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        Copy-Item -Force $exe (Join-Path $dir 'whoisport.exe')
        $version = & (Join-Path $dir 'whoisport.exe') --version
        Write-Host "whoisport-install: installed $version to $dir\whoisport.exe"

        # --- PATH --------------------------------------------------------------------
        if ($env:WHOISPORT_NO_MODIFY_PATH -ne '1') {
            $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
            $entries = if ($userPath) { $userPath -split ';' } else { @() }
            if ($entries -notcontains $dir) {
                $newPath = (($entries + $dir) | Where-Object { $_ }) -join ';'
                [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
                Write-Host "whoisport-install: added $dir to your PATH; open a new terminal to use it"
            }
        }
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

Install-Whoisport
