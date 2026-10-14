#include "clash_verge_native_core.h"
#include <stddef.h>
_Static_assert(sizeof(CvnBuffer) == 32, "C/Rust buffer size");
_Static_assert(offsetof(CvnBuffer, status) == 24, "C/Rust status offset");
/* An intentionally incompatible library, rejected before any handle is created. */
uint32_t cvn_abi_version(void) { return 1; }
