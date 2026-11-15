#include "pattie_core.h"

uint64_t pattie_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
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

uint64_t pattie_clock_ns(uint64_t ticks, uint64_t tick_hz)
{
    /* Whole seconds and a remainder, as in pattie_clock_frames: exact, no overflow. */
    return (ticks / tick_hz) * 1000000000u + (ticks % tick_hz) * 1000000000u / tick_hz;
}

uint64_t pattie_clock_ticks_for(uint64_t frames, uint64_t tick_hz, uint32_t sample_rate)
{
    /*
     * Exactly ceil(frames * tick_hz / sample_rate), split into whole seconds and
     * a remainder so it can't overflow; rounding up makes it the first tick at
     * which pattie_clock_frames reaches `frames`.
     */
    uint64_t seconds = frames / sample_rate;
    uint64_t remainder = frames % sample_rate;

    return seconds * tick_hz + (remainder * tick_hz + sample_rate - 1) / sample_rate;
}

bool pattie_clock_next_pass(uint64_t *start_ticks, uint64_t packets_done, uint32_t packet_frames,
                        uint32_t max_behind, uint64_t now_ticks, uint64_t tick_hz,
                        uint32_t sample_rate, uint64_t *due_ticks)
{
    uint64_t next_frames = (packets_done + 1u) * packet_frames;
    uint64_t next_ticks = pattie_clock_ticks_for(next_frames, tick_hz, sample_rate);
    /* The pass `max_behind` packets after this one: if even it is due, more than
     * `max_behind` passes are, and the stream was stalled. */
    uint64_t last_catchable_ticks =
        pattie_clock_ticks_for(next_frames + (uint64_t)max_behind * packet_frames, tick_hz, sample_rate);
    uint64_t elapsed = now_ticks - *start_ticks; /* unsigned: survives tick wrap */
    bool slipped = elapsed >= last_catchable_ticks;

    if (slipped) {
        *start_ticks = now_ticks - next_ticks;
    }
    *due_ticks = *start_ticks + next_ticks;
    return slipped;
}
