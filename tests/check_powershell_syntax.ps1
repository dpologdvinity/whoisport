<#
.SYNOPSIS
    Fail if any given PowerShell file has a syntax error. Run by CI on the Linux lint job,
    where PowerShell scripts cannot otherwise be executed:
        pwsh -NoProfile -File tests/check_powershell_syntax.ps1 install.ps1 tests/install_test.ps1
#>
param([Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Path)

$bad = $false
foreach ($file in $Path) {
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $file).ProviderPath, [ref]$null, [ref]$errors)
    foreach ($e in $errors) {
        Write-Host "${file}:$($e.Extent.StartLineNumber): $($e.Message)"
        $bad = $true
    }
}
if ($bad) { exit 1 }
Write-Host "PowerShell syntax OK: $($Path -join ', ')"
