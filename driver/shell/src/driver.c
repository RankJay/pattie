/*
 * Kernel-mode compile probe: proves the pure core builds under the WDK.
 * Replaced by the real shell once this builds.
 */
#include <ntddk.h>
#include <wdf.h>
#include "viaduct_core.h"

DRIVER_INITIALIZE DriverEntry;

NTSTATUS DriverEntry(PDRIVER_OBJECT DriverObject, PUNICODE_STRING RegistryPath)
{
    UNREFERENCED_PARAMETER(DriverObject);
    UNREFERENCED_PARAMETER(RegistryPath);
    return STATUS_NOT_IMPLEMENTED;
}
