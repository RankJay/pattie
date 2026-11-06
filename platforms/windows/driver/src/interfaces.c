/*
 * The audio interface values an INF would normally write (ADR 0010).
 *
 * ACX registers and enables a circuit's KS interfaces itself, but Windows' audio
 * endpoint builder only builds an endpoint from interfaces carrying the values an
 * audio INF writes with AddInterface ... AddReg: the KS proxy CLSID, a
 * FriendlyName, and on the wave interfaces the EP\0 endpoint values. A circuit
 * device is a raw child created at runtime, whose values depend on its Device's
 * kind and name, so the shell writes them before the circuit is added and the
 * interfaces are turned on. Values match Microsoft's ACX and SdcaVad sample INFs.
 */
#include "shell.h"

DECLARE_CONST_UNICODE_STRING(VdValueClsid, L"CLSID");
DECLARE_CONST_UNICODE_STRING(VdValueFriendlyName, L"FriendlyName");
DECLARE_CONST_UNICODE_STRING(VdKeyEp, L"EP");
DECLARE_CONST_UNICODE_STRING(VdKeyEp0, L"0");
/* PKEY_AudioEndpoint_Association and PKEY_AudioEndpoint_Supports_EventDriven_Mode. */
DECLARE_CONST_UNICODE_STRING(VdValueAssociation, L"{1DA5D803-D492-4EDD-8C23-E0C0FFEE7F0E},2");
DECLARE_CONST_UNICODE_STRING(VdValueEventDriven, L"{1DA5D803-D492-4EDD-8C23-E0C0FFEE7F0E},7");
static const WCHAR VdKsProxyClsid[] = L"{17CCA71B-ECD7-11D0-B908-00A0C9223196}";
static const WCHAR VdKsNodeTypeAny[] = L"{00000000-0000-0000-0000-000000000000}";

/* One interface the circuit exposes, and whether the endpoint itself lives there. */
typedef struct _VD_INTERFACE {
    const GUID *Category;
    BOOLEAN Endpoint; /* gets the EP\0 values */
} VD_INTERFACE;

static NTSTATUS VdCreateSubkey(HANDLE Parent, PCUNICODE_STRING Name, HANDLE *Key)
{
    OBJECT_ATTRIBUTES attributes;

    InitializeObjectAttributes(&attributes, (PUNICODE_STRING)Name,
                               OBJ_KERNEL_HANDLE | OBJ_CASE_INSENSITIVE, Parent, NULL);
    return ZwCreateKey(Key, KEY_WRITE, &attributes, 0, NULL, REG_OPTION_NON_VOLATILE, NULL);
}

static NTSTATUS VdSetString(HANDLE Key, PCUNICODE_STRING Value, const WCHAR *Data, ULONG Bytes)
{
    return ZwSetValueKey(Key, (PUNICODE_STRING)Value, 0, REG_SZ, (PVOID)Data, Bytes);
}

/* Writes the EP\0 endpoint values under an interface's Device Parameters key. */
static NTSTATUS VdWriteEndpointValues(HANDLE Key)
{
    HANDLE ep = NULL;
    HANDLE ep0 = NULL;
    ULONG one = 1;
    NTSTATUS status;

    status = VdCreateSubkey(Key, &VdKeyEp, &ep);
    if (NT_SUCCESS(status)) {
        status = VdCreateSubkey(ep, &VdKeyEp0, &ep0);
    }
    if (NT_SUCCESS(status)) {
        status = VdSetString(ep0, &VdValueAssociation, VdKsNodeTypeAny, sizeof VdKsNodeTypeAny);
    }
    if (NT_SUCCESS(status)) {
        /* Packet-completion notifications (stream.c) support event-driven mode. */
        status = ZwSetValueKey(ep0, (PUNICODE_STRING)&VdValueEventDriven, 0, REG_DWORD, &one, sizeof one);
    }
    if (ep0 != NULL) {
        ZwClose(ep0);
    }
    if (ep != NULL) {
        ZwClose(ep);
    }
    return status;
}

/* Writes one interface's values into its Device Parameters key. */
static NTSTATUS VdWriteInterfaceValues(PCUNICODE_STRING Link, const VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx,
                                       BOOLEAN Endpoint)
{
    HANDLE key = NULL;
    NTSTATUS status;

    status = IoOpenDeviceInterfaceRegistryKey((PUNICODE_STRING)Link, KEY_WRITE, &key);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = VdSetString(key, &VdValueClsid, VdKsProxyClsid, sizeof VdKsProxyClsid);
    if (NT_SUCCESS(status)) {
        /* NameBuffer is zero-filled past the name, so it is NUL-terminated. */
        status = VdSetString(key, &VdValueFriendlyName, DeviceCtx->NameBuffer,
                             DeviceCtx->Name.Length + sizeof(WCHAR));
    }
    if (NT_SUCCESS(status) && Endpoint) {
        status = VdWriteEndpointValues(key);
    }
    ZwClose(key);
    return status;
}

/*
 * Registers the circuit's audio interfaces and writes their values. Registering
 * an interface ACX also registers (same category, same reference string: the
 * circuit's name) just returns the existing one.
 *
 * The topology interface gets CLSID and FriendlyName but no EP\0, like the
 * topology interfaces in Microsoft's SdcaVad codec INF. ACX also registers the
 * opposite direction's category (RENDER on a capture circuit and vice versa); it
 * is left without values so Windows doesn't build a second, bogus endpoint.
 */
_Use_decl_annotations_
NTSTATUS VdEndpointInterfacesPrepare(WDFDEVICE Device, const VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx)
{
    BOOLEAN microphone = DeviceCtx->Kind == VD_KIND_MICROPHONE;
    const VD_INTERFACE interfaces[] = {
        { &KSCATEGORY_AUDIO, TRUE },
        { microphone ? &KSCATEGORY_CAPTURE : &KSCATEGORY_RENDER, TRUE },
        { &KSCATEGORY_REALTIME, TRUE },
        { &KSCATEGORY_TOPOLOGY, FALSE },
    };
    PDEVICE_OBJECT pdo = WdfDeviceWdmGetPhysicalDevice(Device);
    PCUNICODE_STRING reference = VdCircuitName(DeviceCtx->Kind);
    NTSTATUS status = STATUS_SUCCESS;

    for (ULONG i = 0; i < ARRAYSIZE(interfaces) && NT_SUCCESS(status); i++) {
        UNICODE_STRING link = { 0 };

        status = IoRegisterDeviceInterface(pdo, interfaces[i].Category, (PUNICODE_STRING)reference, &link);
        if (NT_SUCCESS(status)) {
            status = VdWriteInterfaceValues(&link, DeviceCtx, interfaces[i].Endpoint);
            RtlFreeUnicodeString(&link);
        }
    }
    return status;
}
