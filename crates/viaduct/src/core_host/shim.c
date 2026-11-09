/*
 * Core host (ADR 0018; architecture review 2026-11-09): allocates the embedded
 * core's state for the SDK, so
 * the Rust side treats vd_core and vd_owner as opaque and never mirrors their
 * sizes. The core itself never allocates.
 */
#include <stdlib.h>

#include "viaduct_core.h"

vd_core *vd_host_core_new(void)
{
    vd_core *core = calloc(1, sizeof *core);

    if (core != NULL) {
        vd_core_init(core);
    }
    return core;
}

void vd_host_core_free(vd_core *core)
{
    free(core);
}

vd_owner *vd_host_owner_new(void)
{
    vd_owner *owner = calloc(1, sizeof *owner);

    if (owner != NULL) {
        vd_owner_init(owner);
    }
    return owner;
}

void vd_host_owner_free(vd_owner *owner)
{
    free(owner);
}
