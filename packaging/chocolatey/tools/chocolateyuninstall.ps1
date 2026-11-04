$ErrorActionPreference = 'Stop'

[array]$keys = Get-UninstallRegistryKey -SoftwareName 'Skykeep'

if ($keys.Count -eq 1) {
  $uninstaller = $keys[0].UninstallString -replace '"', ''
  $packageArgs = @{
    packageName    = $env:ChocolateyPackageName
    fileType       = 'exe'
    file           = $uninstaller
    silentArgs     = '/S'
    validExitCodes = @(0)
  }
  Uninstall-ChocolateyPackage @packageArgs

  # The NSIS uninstaller hands itself to a temporary copy and returns at once; wait for that
  # copy so the package is reported removed only once Skykeep is gone.
  $program = Join-Path (Split-Path $uninstaller) 'skykeep.exe'
  for ($i = 0; $i -lt 60 -and (Test-Path $program); $i++) { Start-Sleep -Seconds 1 }
  if (Test-Path $program) { throw "Skykeep's uninstaller did not remove $program" }
} elseif ($keys.Count -eq 0) {
  Write-Warning "$env:ChocolateyPackageName has already been uninstalled by other means."
} else {
  Write-Warning "$($keys.Count) matches found; uninstall Skykeep from Settings > Apps instead."
  $keys | ForEach-Object { Write-Warning "- $($_.DisplayName)" }
}
