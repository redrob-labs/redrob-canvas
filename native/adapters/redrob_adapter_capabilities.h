/* SPDX-License-Identifier: GPL-3.0-or-later */
#pragma once

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Side-effect-free, dependency-free product adapter capability report.
 * Returns required UTF-8 JSON bytes excluding NUL. When destination is non-null
 * and capacity > 0, writes a NUL-terminated, possibly truncated copy. */
size_t redrob_adapter_capabilities_json(char *destination, size_t capacity);

#ifdef __cplusplus
}
#endif
