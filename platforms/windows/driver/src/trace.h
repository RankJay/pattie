/*
 * The driver's ETW provider (TraceLogging): "Pattie.Driver".
 *
 * Costs nothing while no trace session listens, so it stays in production builds.
 * Events are written only from lifecycle callbacks (stream create, packets,
 * run, pause, release; circuit device added and prepared), never per audio pass:
 * the timer pass only updates counters, reported when the stream stops. Recording
 * them: platforms/windows/vm/trace-run.ps1 (.ai/ops/vm-steps.md, section G).
 *
 * The GUID is the standard name hash of "Pattie.Driver" (as EventSource and
 * tracelog's "*Name" syntax compute it), so tools can enable it by name.
 */
#ifndef PATTIE_TRACE_H
#define PATTIE_TRACE_H

#include <TraceLoggingProvider.h>

TRACELOGGING_DECLARE_PROVIDER(PattieTraceProvider);

/* Event levels used: lifecycle events are informational. */
#define PATTIE_TRACE_LEVEL_INFO 4

#endif /* PATTIE_TRACE_H */
