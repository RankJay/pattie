#include "viaduct_core.h"

uint64_t vd_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
                         uint64_t tick_hz, uint32_t sample_rate)
{
    uint64_t elapsed = now_ticks - start_ticks; /* unsigned: survives tick wrap */
    return elapsed * sample_rate / tick_hz;
}
