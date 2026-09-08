<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Optional native backend boundaries

Both adapters are OFF by default and the baseline Rust/Qt build requires neither dependency. Compilation intent, lifecycle state, and product readiness are separate capability facts; ABI version and CMake options must never be treated as operation support.

- **Dependency-free product report:** `redrob_adapter_capabilities_json` is always built. OFF/OFF reports both routes uncompiled/unready with `operations:[]` and `formats:[]`. The product FFI exposes the equivalent default-build truth through `redrob_ffi_capabilities_json`.
- **GEGL C:** `-DREDROB_ENABLE_GEGL=ON` requires a system pkg-config module `gegl-0.4 >= 0.4.66`, matching the pinned GIMP floor. Missing pkg-config or an old/missing module is a configure error. Initialization/shutdown are mutex-guarded and idempotent. An enabled library may report `compiled=true` and `initialized=true`, but `ready=false`, operation/format counts zero, and `redrob_gegl_adapter_apply_rgba` still returns false without modifying output. GEGL is a native C library, not a Rust crate.
- **Krita C++:** `-DREDROB_ENABLE_KRITA=ON -DREDROB_KRITA_SOURCE_DIR=/path/to/krita` requires Git-verifiable source exactly at `fdbf33b2146735465bb8aa59928fbc1890ceb160`. This is a pointer-only source-tree scaffold, not linked Krita product integration. It reports `scaffoldCompiled` and `attached` separately, always reports `ready=false` with zero operations/formats, and `applyPaintStroke` remains false. Null attachment detaches cleanly; no KRA or Krita paint support is claimed.

CTest always covers OFF/OFF truthfulness. GEGL lifecycle tests build only when its dependency is available and explicitly enabled; Krita scaffold lifecycle tests build only with the exact source pin. These seams remain `planned` in the compatibility matrix until pinned fixtures demonstrate measurable operations.
