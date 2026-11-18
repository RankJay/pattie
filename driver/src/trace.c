/*
 * Defines the "Pattie.Driver" TraceLogging provider (trace.h). Registered first
 * thing in DriverEntry and unregistered on every DriverEntry failure and on
 * unload (entry.c): an unload with the provider still registered crashes the
 * system the next time ETW calls its callback.
 */
#include "shell.h"

/* d99e0ac9-4341-56b5-5a61-1770f6c29080 = name hash of "Pattie.Driver". The VM
 * scripts derive it from the name (vm/trace-provider.ps1): rename both together. */
TRACELOGGING_DEFINE_PROVIDER(TraceProvider, "Pattie.Driver",
                             (0xd99e0ac9, 0x4341, 0x56b5, 0x5a, 0x61, 0x17, 0x70, 0xf6, 0xc2, 0x90,
                              0x80));
