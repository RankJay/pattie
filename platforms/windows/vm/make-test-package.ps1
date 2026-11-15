<#
.SYNOPSIS
    Builds a test-signed Viaduct driver package for the test VM. Runs on the host.

.DESCRIPTION
    1. Builds the Release driver (x64) and viaduct-cli.
    2. Creates the driver catalog with Inf2Cat.
    3. Signs viaduct.sys and viaduct.cat with a throwaway test certificate that
       exists only in memory: no certificate store, key container or key file on
       this machine is touched. Only its public part (viaduct-test.cer) is saved,
       for the VM to trust.
    4. Collects everything the VM needs into target\<PackageName>\ (vm-package).

    -DriverVersion stamps DriverVer with a chosen version (four numbers); without
    it StampInf derives one from the build time. Two packages with rising
    versions are what an in-place upgrade test needs (vm-steps.md, section R).

    It does not install, load or trust anything on this machine. Copy the package
    folder into the VM and run install-in-vm.ps1 there (.ai/ops/vm-steps.md).
#>
[CmdletBinding()]
param(
    [string]$Configuration = 'Release',
    [ValidatePattern('^\d{1,5}\.\d{1,5}\.\d{1,5}\.\d{1,5}$')]
    [string]$DriverVersion,
    [ValidatePattern('^[A-Za-z0-9._-]+$')]
    [string]$PackageName = 'vm-package'
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$kit = 'C:\Program Files (x86)\Windows Kits\10'
$kitVersion = '10.0.28000.0'
$msbuild = 'C:\Program Files\Microsoft Visual Studio\18\Community\MSBuild\Current\Bin\amd64\MSBuild.exe'
$inf2cat = Join-Path $kit "bin\$kitVersion\x86\Inf2Cat.exe"
$package = Join-Path $repo "target\$PackageName"
$driverOut = Join-Path $repo "target\driver\x64\$Configuration"

# Native tools are judged by exit code only. (Windows PowerShell 5.1 would otherwise
# treat their ordinary stderr output, like cargo's progress, as a fatal error.)
function Invoke-Tool([string]$What, [string]$Exe, [string[]]$Arguments) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Exe @Arguments 2>&1 | ForEach-Object { "$_" } | Out-Host
    } finally {
        $ErrorActionPreference = $saved
    }
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

foreach ($tool in $msbuild, $inf2cat) {
    if (-not (Test-Path $tool)) { throw "Missing tool: $tool" }
}

Write-Host '== Building the driver'
$driverArgs = @(
    (Join-Path $repo 'platforms\windows\driver\viaduct.vcxproj'), '-t:Rebuild',
    "-p:Configuration=$Configuration", '-p:Platform=x64', '-nologo', '-v:minimal')
if ($DriverVersion) {
    $driverArgs += "-p:ViaductDriverVersion=$DriverVersion"
}
Invoke-Tool 'Driver build' $msbuild $driverArgs
if ($DriverVersion) {
    $stamped = Select-String -Path (Join-Path $driverOut 'viaduct.inf') -Pattern '^\s*DriverVer\s*=\s*(.+)$'
    if ($null -eq $stamped -or $stamped.Matches[0].Groups[1].Value -notmatch ",\s*$([regex]::Escape($DriverVersion))\s*$") {
        throw "The built INF doesn't carry version $DriverVersion ($($stamped.Line))"
    }
    Write-Host "Stamped: $($stamped.Line.Trim())"
}

# Static C runtime: a clean Windows VM may lack VCRUNTIME140.dll (the Visual C++
# Redistributable). Its own target folder keeps normal builds from recompiling.
Write-Host '== Building viaduct-cli (static C runtime)'
$cliTarget = Join-Path $repo 'target\vm-build'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
try {
    Invoke-Tool 'viaduct-cli build' 'cargo' @(
        'build', '--release', '-p', 'viaduct-cli', '--manifest-path', (Join-Path $repo 'Cargo.toml'),
        '--target-dir', $cliTarget)
} finally {
    Remove-Item Env:\RUSTFLAGS
}

Write-Host '== Collecting the package'
if (Test-Path $package) { Remove-Item -Recurse -Force $package }
New-Item -ItemType Directory -Force $package | Out-Null
Copy-Item (Join-Path $driverOut 'viaduct.sys'), (Join-Path $driverOut 'viaduct.inf') $package
Copy-Item (Join-Path $cliTarget 'release\viaduct-cli.exe') $package
Copy-Item (Join-Path $PSScriptRoot 'prepare-vm.ps1'), (Join-Path $PSScriptRoot 'install-in-vm.ps1'),
    (Join-Path $PSScriptRoot 'probe-endpoint.ps1'), (Join-Path $PSScriptRoot 'cpu-load.ps1'),
    (Join-Path $PSScriptRoot 'trace-run.ps1'), (Join-Path $PSScriptRoot 'many-mics.ps1'),
    (Join-Path $PSScriptRoot 'race-close.ps1') $package

Write-Host '== Creating the catalog'
Invoke-Tool 'Inf2Cat' $inf2cat @("/driver:$package", '/os:10_CO_X64', '/uselocaltime')

Write-Host '== Creating a throwaway test certificate (files only, no certificate store)'
$rsa = [System.Security.Cryptography.RSA]::Create(3072)
$request = New-Object System.Security.Cryptography.X509Certificates.CertificateRequest(
    'CN=Viaduct Test Signing (VM only)', $rsa,
    [System.Security.Cryptography.HashAlgorithmName]::SHA256,
    [System.Security.Cryptography.RSASignaturePadding]::Pkcs1)
$codeSigning = New-Object System.Security.Cryptography.OidCollection
[void]$codeSigning.Add((New-Object System.Security.Cryptography.Oid '1.3.6.1.5.5.7.3.3'))
$request.CertificateExtensions.Add(
    (New-Object System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension($codeSigning, $false)))
$request.CertificateExtensions.Add(
    (New-Object System.Security.Cryptography.X509Certificates.X509BasicConstraintsExtension($false, $false, 0, $false)))
$certificate = $request.CreateSelfSigned([DateTimeOffset]::Now.AddDays(-1), [DateTimeOffset]::Now.AddDays(90))
[IO.File]::WriteAllBytes((Join-Path $package 'viaduct-test.cer'), $certificate.Export(
    [System.Security.Cryptography.X509Certificates.X509ContentType]::Cert))

# Signs with the in-memory key. (signtool /f would import a .pfx, and Windows then
# keeps a copy of the key in this user's profile; this way nothing is persisted.)
Write-Host '== Signing the driver and catalog'
foreach ($file in 'viaduct.sys', 'viaduct.cat') {
    $path = Join-Path $package $file
    $result = Set-AuthenticodeSignature -FilePath $path -Certificate $certificate -HashAlgorithm SHA256
    $signer = (Get-AuthenticodeSignature $path).SignerCertificate
    # "Not trusted" is expected here: only the VM trusts this certificate.
    if ($null -eq $signer -or $signer.Thumbprint -ne $certificate.Thumbprint) {
        throw "Signing $file failed: $($result.StatusMessage)"
    }
    Write-Host "Signed $file"
}
$rsa.Dispose()

Write-Host ''
Write-Host "Package ready: $package"
Get-ChildItem $package | Format-Table Name, Length -AutoSize | Out-Host
Write-Host 'Copy that folder into the test VM and run install-in-vm.ps1 there as Administrator.'
