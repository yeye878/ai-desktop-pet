# prepare-bundle.ps1
$ErrorActionPreference = "Stop"

$ProjectRoot = Get-Location
$ResourcesDir = "$ProjectRoot\resources"
$NodeDir = "$ResourcesDir\node"
$ZipPath = "$ResourcesDir\node-portable.zip"

$NodeVersion = "v24.16.0"
$ClaudeVersion = "2.1.158"

Write-Host "=== Starting bundle preparation ==="

# 1. Create resources folder
if (-not (Test-Path $ResourcesDir)) {
    New-Item -ItemType Directory -Path $ResourcesDir | Out-Null
    Write-Host "Created resources dir: $ResourcesDir"
}

# 2. Check if Node.js needs to be downloaded or updated
$NeedsNodeDownload = $true
if (Test-Path "$NodeDir\node.exe") {
    $CurrentVersion = & "$NodeDir\node.exe" -v
    Write-Host "Current local Node version: $CurrentVersion"
    $NpmCli = "$NodeDir\node_modules\npm\bin\npm-cli.js"
    Write-Host "Forcing clean install of Node.js and Claude Code..."
}

if ($NeedsNodeDownload) {
    # Terminate any running bundled Node processes to prevent file locks
    $RunningNode = Get-Process | Where-Object { $_.Path -like "*resources\node\node.exe*" }
    if ($RunningNode) {
        Write-Host "Stopping running bundled Node.js processes..."
        $RunningNode | Stop-Process -Force
        Start-Sleep -Seconds 1
    }

    # Clean up old Node directory
    if (Test-Path $NodeDir) {
        Write-Host "Removing old Node directory..."
        Remove-Item -Path $NodeDir -Recurse -Force | Out-Null
    }

    $NodeUrl = "https://npmmirror.com/mirrors/node/$NodeVersion/node-$NodeVersion-win-x64.zip"
    
    # Robust retry loop for download
    $MaxRetries = 5
    $RetryCount = 0
    $Success = $false
    while (-not $Success -and $RetryCount -lt $MaxRetries) {
        try {
            Write-Host "Downloading portable Node.js LTS ($NodeVersion) (Attempt $($RetryCount + 1))..."
            if (Test-Path $ZipPath) {
                Remove-Item $ZipPath -Force
            }
            # Prepend --ssl-no-revoke to bypass CRL check drop
            & curl.exe --ssl-no-revoke -L -o $ZipPath $NodeUrl
            if ($LASTEXITCODE -eq 0 -and (Test-Path $ZipPath) -and (Get-Item $ZipPath).Length -gt 10MB) {
                $Success = $true
                Write-Host "Node.js downloaded successfully!"
            } else {
                throw "Download failed or file corrupted"
            }
        } catch {
            $RetryCount++
            if ($RetryCount -lt $MaxRetries) {
                Write-Host "Download failed: $_. Retrying in 3 seconds..."
                Start-Sleep -Seconds 3
            } else {
                throw "Failed to download Node.js after $MaxRetries attempts."
            }
        }
    }
    
    Write-Host "Extracting Node.js portable..."
    Expand-Archive -Path $ZipPath -DestinationPath $ResourcesDir
    
    $ExtractedFolder = "$ResourcesDir\node-$NodeVersion-win-x64"
    if (Test-Path $ExtractedFolder) {
        Rename-Item -Path $ExtractedFolder -NewName "node"
        Write-Host "Node.js portable extracted successfully!"
    } else {
        throw "Failed to find extracted folder $ExtractedFolder"
    }
    
    if (Test-Path $ZipPath) {
        Remove-Item $ZipPath -Force
    }
}

# 3. Install Claude Code CLI standalone
Write-Host "Installing Claude Code CLI standalone version $ClaudeVersion..."
Set-Location $NodeDir

$NpmCli = "node_modules\npm\bin\npm-cli.js"
if (-not (Test-Path $NpmCli)) {
    Set-Location $ProjectRoot
    throw "Failed to find npm-cli.js inside Node.js directory"
}

# Clean up any pre-existing package.json / package-lock.json to prevent npm pruning node_modules
if (Test-Path "package.json") {
    Remove-Item "package.json" -Force
}
if (Test-Path "package-lock.json") {
    Remove-Item "package-lock.json" -Force
}

# Set environment PATH temporarily so npm can find node and its tools
$OldPath = $env:PATH
$env:PATH = "$NodeDir;$env:PATH"

try {
    Write-Host "Running Claude Code & node-gyp helper installation in a single command..."
    & .\node.exe $NpmCli install node-gyp @anthropic-ai/claude-code@$ClaudeVersion --prefix . --no-audit --no-fund --registry=https://registry.npmmirror.com
}
finally {
    # Restore original PATH
    $env:PATH = $OldPath
}

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
