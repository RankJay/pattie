/*
 * The driver's ETW provider (TraceLogging): "Viaduct.Driver".
 *
 * Costs nothing while no trace session listens, so it stays in production builds.
 * Events are written only from lifecycle callbacks (stream create, packets,
 * run, pause, release; circuit device added and prepared), never per audio pass:
 * the timer pass only updates counters, reported when the stream stops. Recording
 * them: platforms/windows/vm/trace-run.ps1 (.ai/VM-STEPS.md, section G).
 *
 * The GUID is the standard name hash of "Viaduct.Driver" (as EventSource and
 * tracelog's "*Name" syntax compute it), so tools can enable it by name.
 */
#ifndef VIADUCT_TRACE_H
#define VIADUCT_TRACE_H

#include <TraceLoggingProvider.h>

TRACELOGGING_DECLARE_PROVIDER(VdTraceProvider);

/* Event levels used: lifecycle events are informational. */
#define VD_TRACE_LEVEL_INFO 4

#endif /* VIADUCT_TRACE_H */
