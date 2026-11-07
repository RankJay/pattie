/*
 * Defines the "Viaduct.Driver" TraceLogging provider (trace.h). Registered first
 * thing in DriverEntry and unregistered on every DriverEntry failure and on
 * unload (driver.c): an unload with the provider still registered crashes the
 * system the next time ETW calls its callback.
 */
#include "shell.h"

/* a97790c8-3349-54e2-c7f3-9ad24f60a4ce = name hash of "Viaduct.Driver". */
TRACELOGGING_DEFINE_PROVIDER(VdTraceProvider, "Viaduct.Driver",
                             (0xa97790c8, 0x3349, 0x54e2, 0xc7, 0xf3, 0x9a, 0xd2, 0x4f, 0x60, 0xa4, 0xce));
