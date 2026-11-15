/*
 * Float samples to and from 16-bit PCM, Windows' device format (ADR 0005).
 *
 * Integer arithmetic on IEEE-754 single-precision bit patterns only, like the
 * rest of the core: no floating-point instruction, so no FPU state at raised IRQL
 * and no _fltused. A float is sign, 8-bit exponent e, 23-bit mantissa m; a normal
 * number is (2^23 + m) * 2^(e - 150).
 */
#include "pattie_core.h"

#define PATTIE_PCM16_MAX 32767
#define PATTIE_PCM16_MIN (-32767 - 1)

int16_t pattie_pcm16_from_sample(uint32_t sample_bits)
{
    uint32_t negative = sample_bits >> 31;
    uint32_t exponent = (sample_bits >> 23) & 0xFFu;
    uint32_t mantissa = sample_bits & 0x7FFFFFu;
    uint32_t magnitude; /* round(|sample| * 32768), or anything >= 32768 when clamped */

    if (exponent == 0xFFu) {
        if (mantissa != 0) {
            return 0; /* NaN */
        }
        magnitude = 0x10000u; /* infinity */
    } else if (exponent == 0) {
        return 0; /* zero or denormal: far below one PCM step */
    } else if (exponent >= 135) {
        magnitude = 0x10000u; /* |sample| * 32768 >= 2^23 */
    } else {
        /* |sample| * 32768 = (2^23 + m) * 2^(e - 135), so shift right by 135 - e. */
        uint32_t significand = mantissa | 0x800000u;
        uint32_t shift = 135 - exponent;

        if (shift > 24) {
            return 0; /* below half a step */
        }
        magnitude = (significand + (1u << (shift - 1))) >> shift; /* half away from zero */
    }

    if (negative) {
        return magnitude >= 32768u ? (int16_t)PATTIE_PCM16_MIN : (int16_t)-(int32_t)magnitude;
    }
    return magnitude >= (uint32_t)PATTIE_PCM16_MAX ? (int16_t)PATTIE_PCM16_MAX : (int16_t)magnitude;
}

uint32_t pattie_sample_from_pcm16(int16_t pcm)
{
    uint32_t sign = pcm < 0 ? 0x80000000u : 0;
    uint32_t magnitude = pcm < 0 ? (uint32_t)(-(int32_t)pcm) : (uint32_t)pcm; /* 0..32768 */
    uint32_t top = 15;

    if (magnitude == 0) {
        return 0;
    }
    while ((magnitude & (1u << top)) == 0) {
        top--;
    }
    /*
     * pcm / 32768 = magnitude * 2^-15 = 2^(top - 15) * (1 + fraction): exponent
     * top - 15 + 127, and the bits below the top one become the mantissa. Exact,
     * since a 16-bit magnitude fits in the 24-bit significand.
     */
    return sign | ((top + 112u) << 23) | ((magnitude << (23 - top)) & 0x7FFFFFu);
}
