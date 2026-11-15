/*
 * Core host (ADR 0018; architecture review 2026-11-09): allocates the embedded
 * core's state for the SDK, so
 * the Rust side treats pattie_core and pattie_owner as opaque and never mirrors their
 * sizes. The core itself never allocates.
 */
#include <stdlib.h>

#include "pattie_core.h"

pattie_core *pattie_host_core_new(void)
{
    pattie_core *core = calloc(1, sizeof *core);

    if (core != NULL) {
        pattie_core_init(core);
    }
    return core;
}

void pattie_host_core_free(pattie_core *core)
{
    free(core);
}

pattie_owner *pattie_host_owner_new(void)
{
    pattie_owner *owner = calloc(1, sizeof *owner);

    if (owner != NULL) {
        pattie_owner_init(owner);
    }
    return owner;
}

void pattie_host_owner_free(pattie_owner *owner)
{
    free(owner);
}
