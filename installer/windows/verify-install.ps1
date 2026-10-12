# Verify the Canvas setup end to end on this PC: install, upgrade over it, uninstall. Prints PASS/FAIL lines.
$ErrorActionPreference = 'Stop'
$setup = 'C:\Users\USER\redrob\setup-out\Redrob Canvas Setup.exe'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Redrob Canvas'
$exe = Join-Path $dir 'bin\Redrob Canvas.exe'
$un = Join-Path $dir 'Uninstall Redrob Canvas.exe'
$arp = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\RedrobCanvas'
$start = Join-Path ([Environment]::GetFolderPath('Programs')) 'Redrob Canvas.lnk'
$desk = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Redrob Canvas.lnk'
function Check($name, $ok) { if ($ok) { "PASS $name" } else { "FAIL $name" } }

$p = Start-Process -FilePath $setup -ArgumentList '--silent' -Wait -PassThru
Check "install exit 0 (got $($p.ExitCode))" ($p.ExitCode -eq 0)
Check 'exe present' (Test-Path -LiteralPath $exe)
Check 'uninstaller present' (Test-Path -LiteralPath $un)
Check 'start menu shortcut' (Test-Path -LiteralPath $start)
Check 'desktop shortcut' (Test-Path -LiteralPath $desk)
$k = Get-ItemProperty -LiteralPath $arp -ErrorAction SilentlyContinue
Check "arp DisplayName=$($k.DisplayName) DisplayVersion=$($k.DisplayVersion) SizeKB=$($k.EstimatedSize)" ($k.DisplayName -eq 'Redrob Canvas' -and $k.DisplayVersion -eq '0.5.5')
Check "arp UninstallString=$($k.UninstallString)" ($k.UninstallString -like '*Uninstall Redrob Canvas.exe*--uninstall')
$lnk = (New-Object -ComObject WScript.Shell).CreateShortcut($start)
Check "shortcut target=$($lnk.TargetPath)" ($lnk.TargetPath -eq $exe)
$ver = (Get-Item -LiteralPath $exe).VersionInfo
Check "exe ProductName=$($ver.ProductName)" ($ver.ProductName -eq 'Redrob Canvas')
$sig = Get-AuthenticodeSignature -LiteralPath $exe
Check "exe signed ($($sig.SignerCertificate.Subject))" ($null -ne $sig.SignerCertificate)

# Upgrade over the existing install.
New-Item -ItemType File -Path (Join-Path $dir 'stale-marker.txt') -Force | Out-Null
$p = Start-Process -FilePath $setup -ArgumentList '--silent' -Wait -PassThru
Check "reinstall exit 0 (got $($p.ExitCode))" ($p.ExitCode -eq 0)
Check 'reinstall replaced the folder (stale file gone)' (-not (Test-Path -LiteralPath (Join-Path $dir 'stale-marker.txt')))
Check 'no .new/.old leftovers' (-not (Test-Path -LiteralPath "$dir.new") -and -not (Test-Path -LiteralPath "$dir.old"))

# Running app blocks the update.
$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 3
$p = Start-Process -FilePath $setup -ArgumentList '--silent' -Wait -PassThru
Check ("install while app runs refused (got 0x{0:X8})" -f $p.ExitCode) ($p.ExitCode -eq [int]0xE0000001 -or $p.ExitCode -eq -536870911)
Check 'app files intact after refusal' (Test-Path -LiteralPath $exe)
Stop-Process -Id $app.Id -Force
Start-Sleep -Seconds 2

# Uninstall, keeping data.
$p = Start-Process -FilePath $un -ArgumentList '--uninstall','--silent' -Wait -PassThru
Check "uninstall exit 0 (got $($p.ExitCode))" ($p.ExitCode -eq 0)
Start-Sleep -Seconds 6
Check 'install folder gone' (-not (Test-Path -LiteralPath $dir))
Check 'start menu shortcut gone' (-not (Test-Path -LiteralPath $start))
Check 'desktop shortcut gone' (-not (Test-Path -LiteralPath $desk))
Check 'arp key gone' (-not (Test-Path -LiteralPath $arp))
