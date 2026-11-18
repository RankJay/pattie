<#
.SYNOPSIS
    Builds the .cab to submit to Microsoft's Partner Center for attestation
    signing. Runs on the host; it signs nothing and installs nothing.

.DESCRIPTION
    1. Builds the Release driver (x64) with DriverVer's version set to -Version.
    2. Checks the stamped INF carries that version.
    3. Stages pattie.inf, pattie.sys and pattie.pdb in one subfolder (Partner
       Center wants no files at the cab's root, one subfolder per driver package)
       and packs them with makecab into target\attestation\pattie-<Version>.cab.

    No catalog is made: Microsoft creates and signs it. The cab itself must then
    be signed with the company's EV code-signing certificate before upload; the
    script prints the signtool command, since that certificate lives on its own
    token or service and never on this machine's build path.

    Steps around it (upload, download, gate run): .ai/ops/vm-steps.md, section H.
#>
[CmdletBinding()]
param(
    # The driver version, four numbers (e.g. 0.1.0.0). Raise it for every
    # submission: Windows picks the highest DriverVer among installed packages.
    [Parameter(Mandatory)]
    [ValidatePattern('^\d{1,5}\.\d{1,5}\.\d{1,5}\.\d{1,5}$')]
    [string]$Version,

    # Build against the WDK NuGet packages (restored into target\nuget from
    # driver\packages.config) instead of an installed WDK: CI.
    [switch]$WdkNuGet
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$driverOut = Join-Path $repo 'target\driver\x64\Release'
$out = Join-Path $repo 'target\attestation'
$stage = Join-Path $out 'pattie'
$cabName = "pattie-$Version.cab"

# MSBuild from the newest Visual Studio, whichever edition (vswhere ships with the
# Visual Studio Installer).
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw "Missing tool: $vswhere (installed with Visual Studio)" }
$msbuild = & $vswhere -latest -prerelease -requires Microsoft.Component.MSBuild -find 'MSBuild\**\Bin\amd64\MSBuild.exe' |
    Select-Object -First 1
if (-not $msbuild) { throw 'No Visual Studio with MSBuild found' }
$nugetArgs = @() # an explicit array: splatting a lone string would pass its characters
if ($WdkNuGet) { $nugetArgs += '-p:PattieWdkNuGet=true' }

Write-Host "== Building the Release driver, version $Version"
& $msbuild (Join-Path $repo 'driver\pattie.vcxproj') -t:Rebuild `
    -p:Configuration=Release -p:Platform=x64 "-p:PattieDriverVersion=$Version" @nugetArgs -nologo -v:minimal
if ($LASTEXITCODE -ne 0) { throw "Driver build failed (exit $LASTEXITCODE)" }

$stamped = Select-String -Path (Join-Path $driverOut 'pattie.inf') -Pattern '^\s*DriverVer\s*=\s*(.+)$'
if ($null -eq $stamped -or $stamped.Matches[0].Groups[1].Value -notmatch ",\s*$([regex]::Escape($Version))\s*$") {
    throw "The built INF doesn't carry version $Version ($($stamped.Line))"
}
Write-Host "Stamped: $($stamped.Line.Trim())"

Write-Host '== Staging the package'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $stage | Out-Null
foreach ($file in 'pattie.inf', 'pattie.sys', 'pattie.pdb') {
    Copy-Item (Join-Path $driverOut $file) $stage
}

Write-Host '== Packing the cab'
$ddf = Join-Path $out 'pattie.ddf'
$lines = @(
    '.OPTION EXPLICIT'
    '.Set CabinetFileCountThreshold=0'
    '.Set FolderFileCountThreshold=0'
    '.Set FolderSizeThreshold=0'
    '.Set MaxCabinetSize=0'
    '.Set MaxDiskFileCount=0'
    '.Set MaxDiskSize=0'
    '.Set CompressionType=MSZIP'
    '.Set Cabinet=on'
    '.Set Compress=on'
    ".Set CabinetNameTemplate=$cabName"
    ".Set DiskDirectoryTemplate=$out"
    '.Set DestinationDir=pattie'
)
$lines += Get-ChildItem $stage | ForEach-Object { '"' + $_.FullName + '"' }
Set-Content -Path $ddf -Value $lines -Encoding Ascii
Push-Location $out
try {
    & makecab.exe /F $ddf | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "makecab failed (exit $LASTEXITCODE)" }
} finally {
    Pop-Location
}
Remove-Item (Join-Path $out 'setup.inf'), (Join-Path $out 'setup.rpt') -ErrorAction SilentlyContinue

$cab = Join-Path $out $cabName
Write-Host ''
Write-Host "Cab ready: $cab"
& expand.exe -D $cab | Out-Host
Write-Host 'Next, with the EV certificate (its token or signing service), sign the cab:'
Write-Host "  signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 /n `"<EV certificate subject>`" `"$cab`""
Write-Host 'then submit it in Partner Center (.ai/ops/vm-steps.md, section H).'
