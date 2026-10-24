# Builds a Savepoint release into dist\:
#
#   Savepoint-<version>-windows-x64-portable.zip   unzip and run savepointd.exe
#   Savepoint-<version>-windows-x64-setup.exe      installer (needs Inno Setup 6)
#   SHA256SUMS.txt                                 checksums of both
#
# Usage:  .\scripts\package.ps1            build everything it can
#         .\scripts\package.ps1 -NoInstaller
# Works in Windows PowerShell 5.1 and PowerShell 7 (the GitHub release workflow runs it too).
param([switch]$NoInstaller)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $version = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $name = "Savepoint-$version-windows-x64"
    Write-Host "Savepoint $version"

    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    $dist = Join-Path $root "dist"
    $stage = Join-Path $dist "Savepoint"
    if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
    New-Item -ItemType Directory -Force $stage | Out-Null

    Copy-Item "target\release\savepointd.exe", "target\release\savepoint-ui.exe", "LICENSE" $stage
    # Only the GNU toolchain produces this; MSVC builds link the WebView2 loader statically.
    if (Test-Path "target\release\WebView2Loader.dll") { Copy-Item "target\release\WebView2Loader.dll" $stage }
    & "$PSScriptRoot\third-party-licenses.ps1" -Out (Join-Path $stage "THIRD-PARTY-LICENSES.txt")

    $zip = Join-Path $dist "$name-portable.zip"
    Compress-Archive -Path $stage -DestinationPath $zip
    $artifacts = @($zip)

    if (-not $NoInstaller) {
        $iscc = @(
            (Get-Command iscc -ErrorAction SilentlyContinue).Source,
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
        ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
        if ($iscc) {
            & $iscc /Q "/DAppVersion=$version" "/DSourceDir=$stage" "installer\savepoint.iss"
            if ($LASTEXITCODE -ne 0) { throw "installer build failed" }
            $artifacts += Join-Path $dist "$name-setup.exe"
        } else {
            Write-Warning "Inno Setup 6 not found; skipping the installer."
        }
    }

    $sums = $artifacts | ForEach-Object {
        "{0}  {1}" -f (Get-FileHash $_ -Algorithm SHA256).Hash.ToLower(), (Split-Path $_ -Leaf)
    }
    [IO.File]::WriteAllLines((Join-Path $dist "SHA256SUMS.txt"), [string[]]$sums)

    Get-ChildItem $dist -File | Select-Object Name, @{ n = "KB"; e = { [math]::Round($_.Length / 1KB) } } | Format-Table | Out-String | Write-Host
} finally {
    Pop-Location
}
