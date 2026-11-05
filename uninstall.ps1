# Removes the context menu entries and installed files. Your PATH is left as it is.

$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\common.ps1"

Remove-MenuPackage
Stop-MenuHost

foreach ($target in $ClassicMenuTargets) {
    foreach ($verb in $AddVerb, $RemoveVerb) {
        [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree("$($target.Key)\$verb", $false)
    }
}

if (Test-Path -LiteralPath $InstallDir) {
    Remove-Item -LiteralPath $InstallDir -Recurse -Force
}
Write-Host 'Uninstalled.'
