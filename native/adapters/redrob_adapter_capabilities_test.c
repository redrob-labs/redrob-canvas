/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_adapter_capabilities.h"

#include <assert.h>
#include <stdlib.h>
#include <string.h>

int main(void)
{
    const size_t required = redrob_adapter_capabilities_json(NULL, 0);
    assert(required > 0);
    char *json = (char *)malloc(required + 1);
    assert(json != NULL);
    assert(redrob_adapter_capabilities_json(json, required + 1) == required);
    assert(strlen(json) == required);
    assert(strstr(json, "\"ready\":false") != NULL);
    assert(strstr(json, "\"operations\":[]") != NULL);
    assert(strstr(json, "\"formats\":[]") != NULL);
#if !REDROB_GEGL_ADAPTER_COMPILED && !REDROB_KRITA_SCAFFOLD_COMPILED
    assert(strstr(json, "\"gegl\":{\"compiled\":false") != NULL);
    assert(strstr(json, "\"krita\":{\"compiled\":false,\"scaffold_compiled\":false") != NULL);
#endif
    free(json);
    return 0;
}
