# prepare-bundle.ps1
$ErrorActionPreference = "Stop"

$ProjectRoot = Get-Location
$ResourcesDir = "$ProjectRoot\resources"
$NodeDir = "$ResourcesDir\node"
$ZipPath = "$ResourcesDir\node-portable.zip"

Write-Host "=== Starting bundle preparation ==="

# 1. Create resources folder
if (-not (Test-Path $ResourcesDir)) {
    New-Item -ItemType Directory -Path $ResourcesDir | Out-Null
    Write-Host "Created resources dir: $ResourcesDir"
}

# 2. Download and unzip Node.js portable
if (-not (Test-Path $NodeDir)) {
    Write-Host "Downloading portable Node.js LTS (v22.11.0)..."
    $NodeUrl = "https://nodejs.org/dist/v22.11.0/node-v22.11.0-win-x64.zip"
    
    $WebClient = New-Object System.Net.WebClient
    $WebClient.DownloadFile($NodeUrl, $ZipPath)
    
    Write-Host "Extracting Node.js portable..."
    Expand-Archive -Path $ZipPath -DestinationPath $ResourcesDir
    
    $ExtractedFolder = "$ResourcesDir\node-v22.11.0-win-x64"
    if (Test-Path $ExtractedFolder) {
        Rename-Item -Path $ExtractedFolder -NewName "node"
        Write-Host "Node.js portable extracted successfully!"
    } else {
        throw "Failed to find extracted folder"
    }
    
    if (Test-Path $ZipPath) {
        Remove-Item $ZipPath -Force
    }
} else {
    Write-Host "Node.js portable already exists in resources. Skipping download."
}

# 3. Install Claude Code CLI standalone
Write-Host "Installing Claude Code CLI standalone..."
Set-Location $NodeDir

$NpmCli = "node_modules\npm\bin\npm-cli.js"
if (-not (Test-Path $NpmCli)) {
    Set-Location $ProjectRoot
    throw "Failed to find npm-cli.js inside Node.js directory"
}

# Run standalone local install
& .\node.exe $NpmCli install @anthropic-ai/claude-code --prefix . --no-audit --no-fund --registry=https://registry.npmmirror.com

# Verify installation output
$BinInstalled = Test-Path "claude.cmd"
$ModBinInstalled = Test-Path "node_modules\.bin\claude.cmd"

if ($BinInstalled -or $ModBinInstalled) {
    Write-Host "Successfully verified claude.cmd is generated!"
} else {
    Set-Location $ProjectRoot
    throw "Claude Code CLI installation finished but claude.cmd was not found"
}

# Create debug launcher
$LocalTestCmd = "$NodeDir\claude-local.cmd"
$CmdContent = "@echo off`nSET PATH=%~dp0;%PATH%`n`"%~dp0claude.cmd`" %*"
[System.IO.File]::WriteAllText($LocalTestCmd, $CmdContent)
Write-Host "Created local debug script: $LocalTestCmd"

Set-Location $ProjectRoot
Write-Host "=== Bundle preparation completed successfully ==="
