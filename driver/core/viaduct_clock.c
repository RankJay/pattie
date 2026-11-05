#include "viaduct_core.h"

uint64_t vd_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
                         uint64_t tick_hz, uint32_t sample_rate)
{
    uint64_t elapsed = now_ticks - start_ticks; /* unsigned: survives tick wrap */

    /*
     * Exactly floor(elapsed * sample_rate / tick_hz), split into whole seconds
     * and a remainder so the product can't overflow 64 bits. Always computed
     * from the stream's start, so no rounding error accumulates.
     */
    return (elapsed / tick_hz) * sample_rate
         + (elapsed % tick_hz) * sample_rate / tick_hz;
}
