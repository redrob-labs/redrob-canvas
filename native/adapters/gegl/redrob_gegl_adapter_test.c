/* SPDX-License-Identifier: GPL-3.0-or-later */
#include "redrob_gegl_adapter.h"

#include <assert.h>

int main(void)
{
    RedrobGeglAdapterCapabilities capabilities = redrob_gegl_adapter_capabilities();
    assert(capabilities.compiled);
    assert(!capabilities.initialized);
    assert(!capabilities.ready);
    assert(capabilities.operation_count == 0);
    assert(capabilities.format_count == 0);

    redrob_gegl_adapter_shutdown();
    assert(redrob_gegl_adapter_initialize());
    assert(redrob_gegl_adapter_initialize());
    capabilities = redrob_gegl_adapter_capabilities();
    assert(capabilities.initialized);
    assert(!capabilities.ready);

    uint8_t output[] = {7, 8, 9, 10};
    assert(!redrob_gegl_adapter_apply_rgba("unsupported", NULL, 0, 1, 1, output, sizeof(output)));
    assert(output[0] == 7 && output[1] == 8 && output[2] == 9 && output[3] == 10);

    redrob_gegl_adapter_shutdown();
    redrob_gegl_adapter_shutdown();
    assert(!redrob_gegl_adapter_capabilities().initialized);
    return 0;
}
