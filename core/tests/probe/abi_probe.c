#include "abi_probe.h"

uint32_t vd_probe_ioctl(uint32_t command)
{
    return VD_IOCTL(command);
}
