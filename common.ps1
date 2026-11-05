# Shared by install.ps1 and uninstall.ps1.

$InstallDir = "$env:LOCALAPPDATA\Programs\AddToPath"
$PackageName = 'SimpleAddToPath'
$AddVerb = 'AddToUserPath'
$RemoveVerb = 'RemoveFromUserPath'

# %V is the clicked folder, %1 is the clicked file.
$ClassicMenuTargets = @(
    @{ Key = 'Software\Classes\Directory\shell'; Arg = '%V' }
    @{ Key = 'Software\Classes\Directory\Background\shell'; Arg = '%V' }
    @{ Key = 'Software\Classes\SystemFileAssociations\.exe\shell'; Arg = '%1' }
    @{ Key = 'Software\Classes\SystemFileAssociations\.bat\shell'; Arg = '%1' }
    @{ Key = 'Software\Classes\SystemFileAssociations\.cmd\shell'; Arg = '%1' }
)

function Set-ClassicVerb([string]$Key, [string]$Label, [string]$Icon, [string]$Command, [switch]$Extended) {
    $verb = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($Key)
    $verb.SetValue('', $Label)
    $verb.SetValue('Icon', $Icon)
    if ($Extended) { $verb.SetValue('Extended', '') }
    $commandKey = $verb.CreateSubKey('command')
    $commandKey.SetValue('', $Command)
    $commandKey.Close()
    $verb.Close()
}

function Remove-MenuPackage {
    Get-AppxPackage -Name $PackageName | Remove-AppxPackage
}

# The Windows 11 menu command runs inside dllhost.exe, which keeps the DLL locked.
function Stop-MenuHost {
    Get-Process dllhost -ErrorAction SilentlyContinue | Where-Object {
        try { $_.Modules.ModuleName -contains 'add_to_path_ext.dll' } catch { $false }
    } | Stop-Process -Force
}
