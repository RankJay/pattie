/* Gives bindgen the value of PATTIE_IOCTL, a macro it cannot read on its own. */
#include "abi_probe.h"

uint32_t pattie_probe_ioctl(uint32_t command)
{
    return PATTIE_IOCTL(command);
}
