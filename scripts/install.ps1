$ErrorActionPreference = "Stop"

$Repository = if ($env:BOT_REPOSITORY) { $env:BOT_REPOSITORY } else { "woosal1337/bot" }
$Version = if ($env:BOT_VERSION) { $env:BOT_VERSION.TrimStart("v") } else { "latest" }
$InstallDirectory = if ($env:BOT_INSTALL_DIR) { $env:BOT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\Bot" }

$Architecture = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "x86_64" }
    default { throw "Bot does not provide a Windows binary for this architecture." }
}

$Target = "$Architecture-pc-windows-msvc"
$ArchiveName = "bot-$Target.zip"
$ReleasePath = if ($Version -eq "latest") { "latest/download" } else { "download/v$Version" }
$BaseUrl = "https://github.com/$Repository/releases/$ReleasePath"
$TemporaryDirectory = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
$StagedPath = $null

try {
    New-Item -ItemType Directory -Path $TemporaryDirectory | Out-Null
    $ArchivePath = Join-Path $TemporaryDirectory $ArchiveName
    $ChecksumsPath = Join-Path $TemporaryDirectory "SHA256SUMS"
    Invoke-WebRequest -UseBasicParsing "$BaseUrl/$ArchiveName" -OutFile $ArchivePath
    Invoke-WebRequest -UseBasicParsing "$BaseUrl/SHA256SUMS" -OutFile $ChecksumsPath

    $ChecksumLine = Get-Content $ChecksumsPath | Where-Object { $_ -match "\s+$([Regex]::Escape($ArchiveName))$" } | Select-Object -First 1
    if (-not $ChecksumLine) { throw "Bot could not find the release checksum for $ArchiveName." }
    $Expected = ($ChecksumLine -split "\s+")[0].ToUpperInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 $ArchivePath).Hash.ToUpperInvariant()
    if ($Actual -ne $Expected) { throw "The Bot release checksum does not match." }

    Expand-Archive -Path $ArchivePath -DestinationPath $TemporaryDirectory
    New-Item -ItemType Directory -Force -Path $InstallDirectory | Out-Null
    $InstalledPath = Join-Path $InstallDirectory "bot.exe"
    $StagedPath = Join-Path $InstallDirectory "bot-$([System.Guid]::NewGuid().ToString()).new.exe"
    Copy-Item (Join-Path $TemporaryDirectory "bot-$Target\bot.exe") $StagedPath
    if (Test-Path $InstalledPath) {
        $InstalledItem = Get-Item $InstalledPath -Force
        if ($InstalledItem.PSIsContainer -or ($InstalledItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw "Bot will not replace a directory or link at $InstalledPath."
        }
        $BackupPath = Join-Path $InstallDirectory "bot-$([System.Guid]::NewGuid().ToString()).old.exe"
        try {
            [System.IO.File]::Replace($StagedPath, $InstalledPath, $BackupPath)
        }
        catch {
            throw "Could not replace installed Bot. Close Bot, then retry. The old binary was not removed."
        }
        if (Test-Path $BackupPath) { Remove-Item $BackupPath -Force -ErrorAction SilentlyContinue }
    }
    else {
        Move-Item $StagedPath $InstalledPath
    }
    $StagedPath = $null

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $PathEntries = if ($UserPath) { $UserPath -split ";" } else { @() }
    if ($PathEntries -notcontains $InstallDirectory) {
        $NewPath = (@($PathEntries) + $InstallDirectory | Where-Object { $_ }) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    }
    Write-Output "Installed Bot in $InstallDirectory\bot.exe"
    Write-Output "Open a new terminal, then run bot."
}
finally {
    if ($StagedPath -and (Test-Path $StagedPath)) { Remove-Item $StagedPath -Force }
    if (Test-Path $TemporaryDirectory) { Remove-Item -Recurse -Force $TemporaryDirectory }
}
