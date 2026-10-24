# Collects the license texts of every third-party crate compiled into Savepoint,
# as their licenses (MIT, Apache-2.0, ...) require when shipping binaries.
param([Parameter(Mandatory = $true)][string]$Out)

$ErrorActionPreference = "Stop"
$host_triple = ((rustc -vV) | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
$meta = cargo metadata --format-version 1 --locked --filter-platform $host_triple | ConvertFrom-Json

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("Third-party software in Savepoint")
$lines.Add("=================================")
$lines.Add("")
$lines.Add("Savepoint itself is MIT licensed (see LICENSE). It is built with the libraries below,")
$lines.Add("each under its own license. Game save locations come from the Ludusavi Manifest")
$lines.Add("(https://github.com/mtkennerly/ludusavi-manifest, MIT), downloaded at runtime.")
$lines.Add("")

$packages = $meta.packages | Where-Object { $_.source } | Sort-Object name, version
foreach ($p in $packages) {
    $lines.Add("-" * 78)
    $lines.Add("$($p.name) $($p.version)  ($($p.license))")
    if ($p.repository) { $lines.Add($p.repository) }
    $lines.Add("")
    $dir = Split-Path -Parent $p.manifest_path
    $files = Get-ChildItem $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE)' } | Sort-Object Name
    if ($files) {
        foreach ($f in $files) {
            $lines.Add("[$($f.Name)]")
            $lines.Add((Get-Content $f.FullName -Raw).TrimEnd())
            $lines.Add("")
        }
    } else {
        $lines.Add("License: $($p.license) (no license file shipped with the crate; see the SPDX text for this identifier).")
        $lines.Add("")
    }
}
[IO.File]::WriteAllText($Out, ($lines -join "`r`n") + "`r`n", (New-Object Text.UTF8Encoding $false))
Write-Host "third-party licenses: $($packages.Count) crates -> $Out"
