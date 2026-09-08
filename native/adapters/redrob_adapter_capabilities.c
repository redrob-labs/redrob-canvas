/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_adapter_capabilities.h"

#include <string.h>

#ifndef REDROB_GEGL_ADAPTER_COMPILED
#define REDROB_GEGL_ADAPTER_COMPILED 0
#endif
#ifndef REDROB_KRITA_SCAFFOLD_COMPILED
#define REDROB_KRITA_SCAFFOLD_COMPILED 0
#endif

size_t redrob_adapter_capabilities_json(char *destination, size_t capacity)
{
    static const char json[] =
        "{\"schema_version\":1,\"gegl\":{"
#if REDROB_GEGL_ADAPTER_COMPILED
        "\"compiled\":true,"
#else
        "\"compiled\":false,"
#endif
        "\"initialized\":false,\"ready\":false,\"operations\":[],\"formats\":[]},"
        "\"krita\":{\"compiled\":false,"
#if REDROB_KRITA_SCAFFOLD_COMPILED
        "\"scaffold_compiled\":true,"
#else
        "\"scaffold_compiled\":false,"
#endif
        "\"attached\":false,\"ready\":false,\"operations\":[],\"formats\":[]}}";
    const size_t required = sizeof(json) - 1;
    if (destination != NULL && capacity > 0) {
        const size_t copied = required < capacity - 1 ? required : capacity - 1;
        memcpy(destination, json, copied);
        destination[copied] = '\0';
    }
    return required;
}
