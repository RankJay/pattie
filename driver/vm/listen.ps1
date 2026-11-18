<#
.SYNOPSIS
    Records from a capture endpoint the way a meeting app does (WASAPI shared,
    event-driven), keeping nothing: it only counts. Stands in for Sound Recorder.

.DESCRIPTION
    -Name: part of the endpoint's friendly name ("Agent Mic"). Waits up to 60 s
    for it to appear (a new Device's endpoint takes a few seconds).
    -List: prints the active capture endpoints and exits.

    Once capturing it prints "listening: ...". Then, on stdin:
      "mark" prints one line of counts since the previous mark and keeps going;
      "stop" (or end of input) prints the last counts and exits.
    Counts: packets, frames, WASAPI discontinuities (the engine saw a glitch),
    silent packets (flagged silent by the engine), and the peak level heard.
#>
[CmdletBinding()]
param(
    [string]$Name = 'Agent Mic',
    [switch]$List
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

namespace PattieListen {
    [StructLayout(LayoutKind.Sequential)] public struct PropertyKey { public Guid fmtid; public int pid; }
    [StructLayout(LayoutKind.Sequential)] public struct PropVariant { public ushort vt; ushort r1, r2, r3; public IntPtr p; IntPtr p2; }

    [ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceEnumerator { void EnumAudioEndpoints(int flow, uint mask, out IMMDeviceCollection devices); }
    [ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceCollection { void GetCount(out uint count); void Item(uint index, out IMMDevice device); }
    [ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDevice {
        void Activate(ref Guid iid, uint ctx, IntPtr p, [MarshalAs(UnmanagedType.IUnknown)] out object o);
        void OpenPropertyStore(uint access, out IPropertyStore store);
    }
    [ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IPropertyStore {
        void GetCount(out uint n); void GetAt(uint i, out PropertyKey k); void GetValue(ref PropertyKey k, out PropVariant v);
    }
    [ComImport, Guid("1CB9AD4C-DBFA-4C32-B178-C2F568A703B2"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioClient {
        void Initialize(int share, uint flags, long buffer, long period, IntPtr format, IntPtr session);
        void GetBufferSize(out uint frames);
        void GetStreamLatency(out long latency);
        void GetCurrentPadding(out uint padding);
        void IsFormatSupported(int share, IntPtr format, out IntPtr closest);
        void GetMixFormat(out IntPtr format);
        void GetDevicePeriod(out long def, out long min);
        void Start(); void Stop(); void Reset();
        void SetEventHandle(IntPtr handle);
        void GetService(ref Guid iid, [MarshalAs(UnmanagedType.IUnknown)] out object o);
    }
    [ComImport, Guid("C8ADBD64-E71E-48A0-A4DE-185C395CD317"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioCaptureClient {
        void GetBuffer(out IntPtr data, out uint frames, out uint flags, out ulong position, out ulong qpc);
        void ReleaseBuffer(uint frames);
        void GetNextPacketSize(out uint frames);
    }
    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumerator { }

    public class Counts { public long Packets, Frames, Discontinuities, SilentPackets; public float Peak; }

    public class Listener {
        static readonly PropertyKey FriendlyName = new PropertyKey { fmtid = new Guid("A45C254E-DF1C-4EFD-8020-67D146A850E0"), pid = 14 };
        [DllImport("ole32.dll")] static extern int PropVariantClear(ref PropVariant v);

        static List<KeyValuePair<string, IMMDevice>> Captures() {
            var e = (IMMDeviceEnumerator)new MMDeviceEnumerator();
            IMMDeviceCollection all; e.EnumAudioEndpoints(1 /* eCapture */, 1 /* ACTIVE */, out all);
            uint n; all.GetCount(out n);
            var found = new List<KeyValuePair<string, IMMDevice>>();
            for (uint i = 0; i < n; i++) {
                IMMDevice d; all.Item(i, out d);
                IPropertyStore s; d.OpenPropertyStore(0 /* STGM_READ */, out s);
                var key = FriendlyName; PropVariant v; s.GetValue(ref key, out v);
                string name = v.vt == 31 ? Marshal.PtrToStringUni(v.p) : "?";
                PropVariantClear(ref v);
                found.Add(new KeyValuePair<string, IMMDevice>(name, d));
            }
            return found;
        }

        public static string[] List() {
            string[] names = null;
            var t = new Thread(() => { names = Captures().ConvertAll(p => p.Key).ToArray(); });
            t.SetApartmentState(ApartmentState.MTA); t.Start(); t.Join();
            return names;
        }

        readonly object gate = new object();
        Counts counts = new Counts();
        volatile bool stop;
        Thread thread;
        Exception failure;
        public string Endpoint, Format;
        public readonly ManualResetEvent Ready = new ManualResetEvent(false);

        public Counts Take() { lock (gate) { var c = counts; counts = new Counts(); return c; } }
        public Exception Failure { get { return failure; } }

        public void Start(string wanted) {
            thread = new Thread(() => { try { Run(wanted); } catch (Exception x) { failure = x; Ready.Set(); } });
            thread.SetApartmentState(ApartmentState.MTA);
            thread.Start();
        }
        public void Stop() { stop = true; thread.Join(); }

        void Run(string wanted) {
            IMMDevice device = null;
            for (int tries = 0; device == null; tries++) {
                foreach (var p in Captures())
                    if (p.Key.IndexOf(wanted, StringComparison.OrdinalIgnoreCase) >= 0) { Endpoint = p.Key; device = p.Value; break; }
                if (device == null) {
                    if (tries >= 120 || stop) throw new Exception("no active capture endpoint named like \"" + wanted + "\"");
                    Thread.Sleep(500);
                }
            }
            var iid = typeof(IAudioClient).GUID; object o;
            device.Activate(ref iid, 23 /* CLSCTX_ALL */, IntPtr.Zero, out o);
            var client = (IAudioClient)o;
            IntPtr mix; client.GetMixFormat(out mix);
            int channels = Marshal.ReadInt16(mix, 2), rate = Marshal.ReadInt32(mix, 4), bits = Marshal.ReadInt16(mix, 14);
            Format = rate + " Hz, " + channels + " ch, " + bits + "-bit";
            // Shared, event-driven, 20 ms buffer: as a meeting app opens it.
            client.Initialize(0, 0x00040000 /* EVENTCALLBACK */, 200000, 0, mix, IntPtr.Zero);
            Marshal.FreeCoTaskMem(mix);
            var wake = new AutoResetEvent(false);
            client.SetEventHandle(wake.SafeWaitHandle.DangerousGetHandle());
            iid = typeof(IAudioCaptureClient).GUID;
            client.GetService(ref iid, out o);
            var capture = (IAudioCaptureClient)o;
            client.Start();
            Ready.Set();
            while (!stop) {
                wake.WaitOne(200);
                uint next; capture.GetNextPacketSize(out next);
                while (next > 0) {
                    IntPtr data; uint frames, flags; ulong pos, qpc;
                    capture.GetBuffer(out data, out frames, out flags, out pos, out qpc);
                    float peak = 0;
                    if ((flags & 2) == 0 && bits == 32)
                        for (int i = 0; i < frames * channels; i++) {
                            float s = Math.Abs(BitConverter.ToSingle(BitConverter.GetBytes(Marshal.ReadInt32(data, i * 4)), 0));
                            if (s > peak) peak = s;
                        }
                    capture.ReleaseBuffer(frames);
                    lock (gate) {
                        counts.Packets++; counts.Frames += frames;
                        if ((flags & 1) != 0) counts.Discontinuities++;
                        if ((flags & 2) != 0) counts.SilentPackets++;
                        if (peak > counts.Peak) counts.Peak = peak;
                    }
                    capture.GetNextPacketSize(out next);
                }
            }
            client.Stop();
        }
    }
}
'@

if ($List) {
    [PattieListen.Listener]::List() | ForEach-Object { "  $_" }
    return
}

$listener = New-Object PattieListen.Listener
$listener.Start($Name)
[void]$listener.Ready.WaitOne()
if ($listener.Failure) { [Console]::Out.WriteLine("listen failed: $($listener.Failure.Message)"); exit 1 }
[Console]::Out.WriteLine("listening: `"$($listener.Endpoint)`", $($listener.Format)")
[Console]::Out.Flush()

function Write-Counts([string]$Label) {
    $c = $listener.Take()
    [Console]::Out.WriteLine(('{0}: packets {1}, frames {2}, discontinuities {3}, silent packets {4}, peak {5:N3}' -f
        $Label, $c.Packets, $c.Frames, $c.Discontinuities, $c.SilentPackets, $c.Peak))
    [Console]::Out.Flush()
}

while ($true) {
    $line = [Console]::In.ReadLine()
    if ($null -eq $line -or $line -eq 'stop') { break }
    if ($line -eq 'mark') { Write-Counts 'counts' }
}
Write-Counts 'final'
$listener.Stop()
if ($listener.Failure) { [Console]::Out.WriteLine("listen failed: $($listener.Failure.Message)"); exit 1 }
