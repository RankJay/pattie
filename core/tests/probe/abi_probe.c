#include "abi_probe.h"

uint32_t pattie_probe_ioctl(uint32_t command)
{
    return PATTIE_IOCTL(command);
}
