<#
.SYNOPSIS
    Probes the live Pattie endpoint's KS filters. Runs INSIDE THE TEST VM ONLY.

.DESCRIPTION
    For every enabled audio interface on a Pattie child device (start
    pattie-cli first so one exists), opens the interface the way Windows' audio
    service does and asks the KS filter how many pin types it has
    (KSPROPERTY_PIN_CTYPES). A healthy ACX capture or render circuit answers 2.

    It only opens handles and reads one property; nothing is changed. If the open
    or the query fails, the Win32 error says why Windows may consider the
    endpoint absent.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class KsProperty {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern SafeFileHandle CreateFile(string name, uint access, uint share, IntPtr security,
                                                   uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool DeviceIoControl(SafeFileHandle device, uint code, byte[] input, int inputLength,
                                              byte[] output, int outputLength, out int returned, IntPtr overlapped);
}
'@

$categories = [ordered]@{
    AUDIO    = '{6994AD04-93EF-11D0-A3CC-00A0C9223196}'
    CAPTURE  = '{65E8773D-8F56-11D0-A3B9-00A0C9223196}'
    RENDER   = '{65E8773E-8F56-11D0-A3B9-00A0C9223196}'
    REALTIME = '{EB115FFC-10C8-4964-831D-6DCB02E6F23F}'
    TOPOLOGY = '{DDA54A40-1E4C-11D1-A050-405705C10000}'
}

# KSPROPERTY { GUID Set = KSPROPSETID_Pin; ULONG Id = KSPROPERTY_PIN_CTYPES (1, ks.h); ULONG Flags = KSPROPERTY_TYPE_GET (1) }
$property = New-Object byte[] 24
[Guid]::new('8C134960-51AD-11CF-878A-94F801C10000').ToByteArray().CopyTo($property, 0)
[BitConverter]::GetBytes([uint32]1).CopyTo($property, 16)
[BitConverter]::GetBytes([uint32]1).CopyTo($property, 20)
$IOCTL_KS_PROPERTY = [uint32]0x002F0003   # CTL_CODE(FILE_DEVICE_KS, 0, METHOD_NEITHER, FILE_ANY_ACCESS)

$found = $false
foreach ($name in $categories.Keys) {
    $paths = & pnputil /enum-interfaces /class $categories[$name] /enabled |
        Select-String -Pattern 'Interface Path:\s+(\\\\\?\\Pattie#.*)$' |
        ForEach-Object { $_.Matches[0].Groups[1].Value.Trim() }
    foreach ($path in $paths) {
        $found = $true
        # GENERIC_READ | GENERIC_WRITE (PowerShell 5.1 reads 0xC0000000 as a negative Int32)
        $handle = [KsProperty]::CreateFile($path, [uint32]3221225472, 3, [IntPtr]::Zero, 3, 0, [IntPtr]::Zero)
        if ($handle.IsInvalid) {
            $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            "{0,-9} OPEN FAILED  win32 {1} ({2})  {3}" -f $name, $err, (New-Object ComponentModel.Win32Exception $err).Message, $path
            continue
        }
        $out = New-Object byte[] 4
        $returned = 0
        if ([KsProperty]::DeviceIoControl($handle, $IOCTL_KS_PROPERTY, $property, 24, $out, 4, [ref]$returned, [IntPtr]::Zero)) {
            "{0,-9} opened, pin types = {1}  {2}" -f $name, [BitConverter]::ToUInt32($out, 0), $path
        } else {
            $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            "{0,-9} opened, PIN_CTYPES FAILED win32 {1} ({2})  {3}" -f $name, $err, (New-Object ComponentModel.Win32Exception $err).Message, $path
        }
        $handle.Dispose()
    }
}
if (-not $found) { 'No enabled Pattie audio interfaces. Is pattie-cli running?' }
