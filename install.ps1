# Installs the context menu entries for the current user. No admin rights needed.
#   Windows 10 and "Show more options": registry entries.
#   Windows 11 top-level menu:          a local package, which needs Developer Mode.

$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\common.ps1"

cargo build --release --manifest-path "$PSScriptRoot\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw 'The build failed.' }

Remove-MenuPackage
Stop-MenuHost

New-Item -ItemType Directory -Force -Path "$InstallDir\Assets" | Out-Null
Copy-Item "$PSScriptRoot\target\release\add-to-path.exe", "$PSScriptRoot\target\release\add_to_path_ext.dll" $InstallDir -Force
Copy-Item "$PSScriptRoot\packaging\AppxManifest.xml" $InstallDir -Force

Add-Type -AssemblyName System.Drawing
foreach ($size in 150, 44) {
    $bitmap = New-Object System.Drawing.Bitmap $size, $size
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([System.Drawing.Color]::FromArgb(0, 120, 212))
    $graphics.Dispose()
    $bitmap.Save("$InstallDir\Assets\Logo$size.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
}

$exe = "$InstallDir\add-to-path.exe"
$icon = "$env:SystemRoot\System32\SystemPropertiesAdvanced.exe,0"
foreach ($target in $ClassicMenuTargets) {
    Set-ClassicVerb "$($target.Key)\$AddVerb" 'Add to user PATH' $icon "`"$exe`" add `"$($target.Arg)`""
    # Stays out of the way: this one only appears on Shift + right-click.
    Set-ClassicVerb "$($target.Key)\$RemoveVerb" 'Remove from user PATH' $icon "`"$exe`" remove `"$($target.Arg)`"" -Extended
}
Write-Host 'Classic menu entries installed.'

if ([Environment]::OSVersion.Version.Build -ge 22000) {
    Add-AppxPackage -Register "$InstallDir\AppxManifest.xml" -ExternalLocation $InstallDir
    Write-Host 'Windows 11 menu entry installed.'
}
Write-Host "Done. Installed to $InstallDir"
