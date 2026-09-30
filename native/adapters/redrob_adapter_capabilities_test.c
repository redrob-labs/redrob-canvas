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

    /* babl is present whether or not it is compiled, and it is the one entry that
     * names formats: those strings are compile-time constants, so this
     * dependency-free report can state them without linking babl. */
    assert(strstr(json, "\"babl\":{\"compiled\":") != NULL);
    assert(strstr(json, "\"R'G'B'A u8\"") != NULL);
    assert(strstr(json, "\"RGBA float\"") != NULL);
    assert(strstr(json, "\"operations\":[\"convert\"]") != NULL);
#if REDROB_BABL_ADAPTER_COMPILED
    assert(strstr(json, "\"babl\":{\"compiled\":true") != NULL);
#else
    assert(strstr(json, "\"babl\":{\"compiled\":false") != NULL);
#endif

    assert(strstr(json, "\"lcms\":{\"compiled\":") != NULL);
    assert(strstr(json, "\"open_profile\"") != NULL);
    assert(strstr(json, "\"rgba8->rgba_f32\"") != NULL);
#if REDROB_LCMS_ADAPTER_COMPILED
    assert(strstr(json, "\"lcms\":{\"compiled\":true") != NULL);
#else
    assert(strstr(json, "\"lcms\":{\"compiled\":false") != NULL);
#endif
    /* Whatever else is true, this report never claims a runtime state it cannot
     * observe: it does not link any adapter, so nothing here may say ready. */
    assert(strstr(json, "\"ready\":true") == NULL);
    assert(strstr(json, "\"initialized\":true") == NULL);

    /* Truncation stays NUL-terminated and never overruns. The report grew by an
     * entry in this change, and a short destination is exactly where a
     * length-versus-capacity slip would first show. */
    char small[16];
    memset(small, 'x', sizeof small);
    assert(redrob_adapter_capabilities_json(small, sizeof small) == required);
    assert(strlen(small) == sizeof small - 1);

    free(json);
    return 0;
}
