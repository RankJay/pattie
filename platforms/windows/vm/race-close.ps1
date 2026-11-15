<#
.SYNOPSIS
    Drives the driver's control device directly, to check two fixes from the
    security review of 2026-11-14 (.ai/research/security-review-kernel-2026-11-14.md).

.DESCRIPTION
    -Mode Race (finding 1): opens \\.\Viaduct, says Hello, and has 6 threads
    send overlapped Create Device requests at once while another thread closes
    the handle, so some are still queued or in flight; repeats. (Threads,
    because the driver's sequential queue runs an idle queue's request on the
    caller's own thread: one thread alone never has two requests in flight.)
    Before the fix, a Create that ran after the close made a Device nobody
    would remove, with this process's
    memory still locked (and likely a bugcheck when the process exits). After
    it, those late Creates are refused, and no "Race" endpoint is left.

    -Mode Burst (finding 2): 40 threads send a Create Device request each, at
    once, on one handle. Only 16 may hold a pinned Ring at a time; requests past
    that are refused at once with ERROR_BUSY (170). Then it closes the handle.

    -Mode Gone (ADR 0024, VM-STEPS R5): a forced removal under a live Owner.
    Opens \\.\Viaduct, says Hello, creates a Device, then disables the root
    device with CM_DISABLE_ABSOLUTE ("don't ask the driver"), which skips the
    query-remove veto as no installer or pnputil does. On the still-open handle
    it then sends Hello and Create Device, which must fail with
    STATUS_DEVICE_REMOVED (Win32 1617, the SDK's DriverRemoved), closes it,
    re-enables the root and checks a new Owner can create a Device again.

    -Mode Names (finding 5): opening the device with a trailing name
    (\\.\Viaduct\x) must fail with ERROR_FILE_NOT_FOUND (2); opening
    \\.\Viaduct itself must still work.

    Run it in the guest with Driver Verifier on, then exit PowerShell (the
    process exit is when locked pages would bugcheck), and check
    `verifier /query`.

.EXAMPLE
    .\race-close.ps1 -Mode Race -Rounds 200
    .\race-close.ps1 -Mode Burst
    .\race-close.ps1 -Mode Names
#>
[CmdletBinding()]
param(
    [ValidateSet('Race', 'Burst', 'Names', 'Gone')][string]$Mode = 'Race',
    [ValidateRange(1, 10000)][int]$Rounds = 200,
    # Gone only: CM_DISABLE_HARDWARE instead of CM_DISABLE_ABSOLUTE (R5 found
    # ABSOLUTE still vetoed). Not restartable until a reboot if it takes.
    [switch]$Hard
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class ViaductRaw
{
    const uint GENERIC_READ = 0x80000000, GENERIC_WRITE = 0x40000000;
    const uint OPEN_EXISTING = 3, FILE_FLAG_OVERLAPPED = 0x40000000;
    const int ERROR_IO_PENDING = 997;

    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa,
                                     uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32", SetLastError = true)]
    static extern bool DeviceIoControl(IntPtr device, uint code, IntPtr input, uint inputLength,
                                       IntPtr output, uint outputLength, IntPtr returned,
                                       IntPtr overlapped);
    [DllImport("kernel32", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32", SetLastError = true)]
    static extern IntPtr CreateEventW(IntPtr sa, bool manual, bool initial, IntPtr name);
    [DllImport("kernel32", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr handle, uint ms);
    [DllImport("kernel32", SetLastError = true)]
    static extern IntPtr VirtualAlloc(IntPtr address, UIntPtr size, uint type, uint protect);

    static uint Ioctl(uint command) { return (0x22u << 16) | (3u << 14) | ((0x800u + command) << 2); }

    static IntPtr ring = IntPtr.Zero;
    static IntPtr doorbell = IntPtr.Zero;
    const int RingBytes = 192 + 4096 * 4;

    static IntPtr Open()
    {
        IntPtr h = CreateFileW(@"\\.\Viaduct", GENERIC_READ | GENERIC_WRITE, 0, IntPtr.Zero,
                               OPEN_EXISTING, FILE_FLAG_OVERLAPPED, IntPtr.Zero);
        if (h == new IntPtr(-1)) throw new Exception("open \\\\.\\Viaduct: " + Marshal.GetLastWin32Error());
        if (ring == IntPtr.Zero) {
            ring = VirtualAlloc(IntPtr.Zero, new UIntPtr(RingBytes), 0x3000, 0x04); // commit+reserve, RW
            Marshal.WriteInt32(ring, 128, 4096);                                  // capacity_frames
            doorbell = CreateEventW(IntPtr.Zero, false, false, IntPtr.Zero);
        }
        return h;
    }

    // One overlapped IOCTL, with its buffers and event kept until it completes.
    class Pending { public IntPtr Input, Output, Overlapped, Event; public int Error; }

    static Pending Send(IntPtr device, uint command, byte[] input)
    {
        var p = new Pending();
        p.Input = Marshal.AllocHGlobal(input.Length);
        Marshal.Copy(input, 0, p.Input, input.Length);
        p.Output = Marshal.AllocHGlobal(64);
        for (int i = 0; i < 64; i += 4) Marshal.WriteInt32(p.Output, i, -1);
        p.Overlapped = Marshal.AllocHGlobal(32);
        for (int i = 0; i < 32; i += 4) Marshal.WriteInt32(p.Overlapped, i, 0);
        p.Event = CreateEventW(IntPtr.Zero, true, false, IntPtr.Zero);
        Marshal.WriteIntPtr(p.Overlapped, 24, p.Event); // OVERLAPPED.hEvent (x64)
        bool done = DeviceIoControl(device, Ioctl(command), p.Input, (uint)input.Length,
                                    p.Output, 64, IntPtr.Zero, p.Overlapped);
        if (!done) {
            int error = Marshal.GetLastWin32Error();
            if (error != ERROR_IO_PENDING) p.Error = error; // failed at once: no completion to wait for
        }
        return p;
    }

    // What happened to a request:
    //   >= 0  it ran: the command's status from the reply envelope;
    //   -1    refused at once with ERROR_BUSY (the pin cap);
    //   -2    cancelled (STATUS_CANCELLED), e.g. by the framework at the close;
    //   -3    any other failure (the handle already closed, ...).
    static int Finish(Pending p)
    {
        int outcome;
        if (p.Error == 170) {
            outcome = -1;
        } else if (p.Error != 0) {
            outcome = -3;
        } else {
            WaitForSingleObject(p.Event, 10000);
            int status = Marshal.ReadInt32(p.Overlapped, 0); // OVERLAPPED.Internal: the NTSTATUS
            if (status == 0) outcome = Marshal.ReadInt32(p.Output, 0);
            else if (status == unchecked((int)0xC0000120)) outcome = -2;
            else if (status == unchecked((int)0x80000011)) outcome = -1;
            else outcome = -3;
        }
        CloseHandle(p.Event);
        Marshal.FreeHGlobal(p.Input); Marshal.FreeHGlobal(p.Output); Marshal.FreeHGlobal(p.Overlapped);
        return outcome;
    }

    static byte[] Hello() { return new byte[] { 1, 0, 0, 0 }; } // major 1, minor 0

    static byte[] Create(string name)
    {
        var r = new byte[160];
        BitConverter.GetBytes(1u).CopyTo(r, 0);                 // kind: Microphone
        BitConverter.GetBytes((uint)name.Length).CopyTo(r, 4);  // name_len
        System.Text.Encoding.Unicode.GetBytes(name).CopyTo(r, 8);
        BitConverter.GetBytes((ulong)ring.ToInt64()).CopyTo(r, 136);
        BitConverter.GetBytes((ulong)RingBytes).CopyTo(r, 144);
        BitConverter.GetBytes((ulong)doorbell.ToInt64()).CopyTo(r, 152);
        return r;
    }

    // Several threads send at once: the control device's queue is sequential and
    // runs an idle queue's request on the caller's thread, so only concurrent
    // callers ever have requests queued or in flight together.
    static int[] Concurrently(IntPtr device, int threads, Func<int, byte[]> request, Action whileSending)
    {
        var outcomes = new int[threads];
        var start = new System.Threading.Barrier(threads + 1);
        var workers = new System.Threading.Thread[threads];
        for (int i = 0; i < threads; i++) {
            int n = i;
            workers[n] = new System.Threading.Thread(() => {
                start.SignalAndWait();
                outcomes[n] = Finish(Send(device, 2, request(n)));
            });
            workers[n].Start();
        }
        start.SignalAndWait();
        whileSending();
        foreach (var w in workers) w.Join();
        return outcomes;
    }

    // Counts: created, refused after the close (status 4), busy, cancelled,
    // too many Devices (status 9), other.
    public static int[] Race(int rounds)
    {
        var counts = new int[6];
        for (int round = 0; round < rounds; round++) {
            IntPtr device = Open();
            Finish(Send(device, 1, Hello()));
            int r = round;
            var outcomes = Concurrently(device, 6, n => Create("Race " + r + "." + n), () => {
                System.Threading.Thread.SpinWait(20000 * (r % 10)); // vary where the close lands
                CloseHandle(device);
            });
            foreach (int o in outcomes) Tally(counts, o);
        }
        return counts;
    }

    public static int[] Burst()
    {
        var counts = new int[6];
        IntPtr device = Open();
        Finish(Send(device, 1, Hello()));
        var outcomes = Concurrently(device, 40, n => Create("Burst " + n), () => { });
        foreach (int o in outcomes) Tally(counts, o);
        CloseHandle(device);
        return counts;
    }

    // Win32 error of opening `path` (0: it opened, and was closed again).
    public static int TryOpen(string path)
    {
        IntPtr h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, 0, IntPtr.Zero,
                               OPEN_EXISTING, 0, IntPtr.Zero);
        if (h == new IntPtr(-1)) return Marshal.GetLastWin32Error();
        CloseHandle(h);
        return 0;
    }

    [DllImport("cfgmgr32", CharSet = CharSet.Unicode)]
    static extern int CM_Locate_DevNodeW(out uint devInst, string instanceId, uint flags);
    [DllImport("cfgmgr32")]
    static extern int CM_Disable_DevNode(uint devInst, uint flags);
    [DllImport("cfgmgr32")]
    static extern int CM_Enable_DevNode(uint devInst, uint flags);
    [DllImport("ntdll")]
    static extern int RtlNtStatusToDosError(int status);

    // The NTSTATUS a request completed with (0 when it ran; then the reply's own
    // status is in the envelope), or the Win32 error it failed with at once.
    static string Outcome(IntPtr device, uint command, byte[] input)
    {
        Pending p = Send(device, command, input);
        string text;
        if (p.Error != 0) {
            text = "failed at once, Win32 error " + p.Error;
        } else {
            WaitForSingleObject(p.Event, 10000);
            int status = Marshal.ReadInt32(p.Overlapped, 0);
            text = status == 0
                ? "ran (reply status " + Marshal.ReadInt32(p.Output, 0) + ")"
                : string.Format("NTSTATUS 0x{0:X8} (Win32 {1})", status, RtlNtStatusToDosError(status));
        }
        CloseHandle(p.Event);
        Marshal.FreeHGlobal(p.Input); Marshal.FreeHGlobal(p.Output); Marshal.FreeHGlobal(p.Overlapped);
        return text;
    }

    const uint CM_DISABLE_ABSOLUTE = 1, CM_DISABLE_HARDWARE = 2, CM_DISABLE_UI_NOT_OK = 4;

    static string Cr(int cr)
    {
        return cr == 0 ? "0 (CR_SUCCESS)" : cr == 0x17 ? "23 (CR_REMOVE_VETOED: the driver refused)" : cr.ToString();
    }

    // Each step's result, in order, for the script to print.
    public static string[] Gone(string rootInstance, bool hard)
    {
        var log = new System.Collections.Generic.List<string>();
        uint root;
        int cr = CM_Locate_DevNodeW(out root, rootInstance, 0);
        if (cr != 0) { log.Add("CM_Locate_DevNode(" + rootInstance + "): CONFIGRET " + cr); return log.ToArray(); }

        IntPtr device = Open();
        log.Add("Hello before:  " + Outcome(device, 1, Hello()));
        log.Add("Create before: " + Outcome(device, 2, Create("Gone A")));

        cr = CM_Disable_DevNode(root, (hard ? CM_DISABLE_HARDWARE : CM_DISABLE_ABSOLUTE) | CM_DISABLE_UI_NOT_OK);
        log.Add("CM_Disable_DevNode(" + (hard ? "HARDWARE" : "ABSOLUTE") + "): CONFIGRET " + Cr(cr));
        if (cr != 0) log.Add("  the root wasn't disabled: the commands below run as usual");
        System.Threading.Thread.Sleep(3000);

        log.Add("Hello after:   " + Outcome(device, 1, Hello()) + "   <- want NTSTATUS 0xC00002B6 (Win32 1617)");
        log.Add("Create after:  " + Outcome(device, 2, Create("Gone B")) + "   <- the same");
        CloseHandle(device);
        System.Threading.Thread.Sleep(2000);

        cr = CM_Enable_DevNode(root, 0);
        log.Add("CM_Enable_DevNode: CONFIGRET " + cr);
        System.Threading.Thread.Sleep(5000);
        int opened = TryOpen(@"\\.\Viaduct");
        log.Add(@"A new Owner opens \\.\Viaduct: error " + opened + "   <- want 0");
        if (opened == 0) {
            IntPtr fresh = Open();
            log.Add("  Hello:  " + Outcome(fresh, 1, Hello()));
            log.Add("  Create: " + Outcome(fresh, 2, Create("Gone C")) + "   <- want ran (reply status 0)");
            CloseHandle(fresh);
        }
        return log.ToArray();
    }

    static void Tally(int[] counts, int outcome)
    {
        if (outcome == 0) counts[0]++;
        else if (outcome == 4) counts[1]++;
        else if (outcome == -1) counts[2]++;
        else if (outcome == -2) counts[3]++;
        else if (outcome == 9) counts[4]++;
        else counts[5]++;
    }
}
'@

if ($Mode -eq 'Race') {
    $c = [ViaductRaw]::Race($Rounds)
    "Race, $Rounds rounds: 6 threads send Creates while another closes the handle"
    "  created: $($c[0]); refused after the close: $($c[1]); cancelled at the close: $($c[3])"
    "  busy: $($c[2]); too many Devices: $($c[4]); other (handle already closed, ...): $($c[5])"
    "  expect some refused or cancelled after the close: the race was reached"
    Start-Sleep -Seconds 3   # removals are asynchronous
    $left = @(Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue |
        Where-Object { $_.FriendlyName -like '*(Race *' -and $_.Status -eq 'OK' })
    "  'Race' endpoints still present: $($left.Count)   <- expect 0"
} elseif ($Mode -eq 'Gone') {
    $root = (Get-PnpDevice -FriendlyName 'Viaduct Virtual Audio' -ErrorAction Stop).InstanceId
    "Gone: a forced removal of $root under a live Owner"
    [ViaductRaw]::Gone($root, [bool]$Hard) | ForEach-Object { "  $_" }
    Start-Sleep -Seconds 2
    $after = Get-PnpDevice -InstanceId $root
    "  the root now: Status $($after.Status), Problem $($after.Problem)   <- want OK"
    $left = @(Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue |
        Where-Object { $_.FriendlyName -like '*(Gone *' -and $_.Status -eq 'OK' })
    "  'Gone' endpoints still present: $($left.Count)   <- expect 0"
} elseif ($Mode -eq 'Names') {
    $named = [ViaductRaw]::TryOpen('\\.\Viaduct\x')
    $plain = [ViaductRaw]::TryOpen('\\.\Viaduct')
    "Opening \\.\Viaduct\x: error $named   <- expect 2 (ERROR_FILE_NOT_FOUND)"
    "Opening \\.\Viaduct:   error $plain   <- expect 0 (it opens)"
    return
} else {
    $c = [ViaductRaw]::Burst()
    "Burst: 40 threads send a Create each, at once, on one handle"
    "  created: $($c[0]); busy (ERROR_BUSY): $($c[2]); too many Devices: $($c[4]); other: $($c[1] + $c[3] + $c[5])"
    "  expect: created at most 8, and busy above 0 (only 16 may hold a pinned Ring at once)"
}
'Now exit this PowerShell window (process exit is when locked pages would bugcheck), then run: verifier /query'
