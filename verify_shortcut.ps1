$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$ico = Join-Path $root 'src-tauri\icons\pet_shortcut.ico'
$stableExe = Join-Path $root 'desktop-release\AI Desktop Pet.exe'
$oldTargetExe = Join-Path $root 'src-tauri\target\release\ai-desktop-pet.exe'
$lnk = [IO.Path]::Combine([Environment]::GetFolderPath('Desktop'), 'AI-Pet.lnk')

Write-Host "=== Verification ==="

$icoExists = Test-Path $ico
$lnkExists = Test-Path $lnk
$exeExists = Test-Path $stableExe
$usesStableExe = $false

Write-Host "Icon file  : $icoExists  -> $ico"
Write-Host "Shortcut   : $lnkExists  -> $lnk"
Write-Host "Stable exe : $exeExists  -> $stableExe"

if ($lnkExists) {
    $sh = New-Object -ComObject WScript.Shell
    $s  = $sh.CreateShortcut($lnk)
    Write-Host "  Shortcut target : $($s.TargetPath)"
    Write-Host "  Working dir     : $($s.WorkingDirectory)"
    Write-Host "  Shortcut icon   : $($s.IconLocation)"
    Write-Host "  Description     : $($s.Description)"
    $usesStableExe = $s.TargetPath -eq $stableExe
    Write-Host "  Uses stable exe : $usesStableExe"
    Write-Host "  Uses old target : $($s.TargetPath -eq $oldTargetExe)"
}

if ($icoExists) {
    $size = (Get-Item $ico).Length
    Write-Host "  Icon size       : $size bytes"
}

Write-Host ""
if ($icoExists -and $lnkExists -and $exeExists -and $usesStableExe) {
    Write-Host "SUCCESS! Your AI-Pet shortcut is ready on the desktop."
} else {
    Write-Host "Shortcut is not ready. Run create_shortcut.ps1 to rebuild and repoint it."
}
