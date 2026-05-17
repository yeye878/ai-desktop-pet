$ico = 'D:\ai-desktop-pet\src-tauri\icons\pet_shortcut.ico'
$lnk = [IO.Path]::Combine([Environment]::GetFolderPath('Desktop'), 'AI-Pet.lnk')

Write-Host "=== Verification ==="

$icoExists = Test-Path $ico
$lnkExists = Test-Path $lnk
$exeExists = Test-Path 'D:\ai-desktop-pet\src-tauri\target\release\ai-desktop-pet.exe'

Write-Host "Icon file  : $icoExists  -> $ico"
Write-Host "Shortcut   : $lnkExists  -> $lnk"
Write-Host "Target exe : $exeExists"

if ($lnkExists) {
    $sh = New-Object -ComObject WScript.Shell
    $s  = $sh.CreateShortcut($lnk)
    Write-Host "  Shortcut target : $($s.TargetPath)"
    Write-Host "  Shortcut icon   : $($s.IconLocation)"
    Write-Host "  Description     : $($s.Description)"
}

if ($icoExists) {
    $size = (Get-Item $ico).Length
    Write-Host "  Icon size       : $size bytes"
}

Write-Host ""
if ($icoExists -and $lnkExists -and $exeExists) {
    Write-Host "SUCCESS! Your AI-Pet shortcut is ready on the desktop."
} else {
    Write-Host "Something is missing, please check above."
}
