<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Optional native backend boundaries

The babl and Little-CMS 2 adapters are OFF by default and the baseline Rust/Qt build requires neither. The GEGL and Krita adapters were removed in 0.5.4. Compilation intent, lifecycle state, and product readiness are separate capability facts; ABI version and CMake options must never be treated as operation support.

- **Dependency-free product report:** `redrob_adapter_capabilities_json` is always built and reports each adapter uncompiled/unready in an OFF build. The product FFI reports `adapters: {}`, because no adapter is routed into the product.

CTest always covers OFF truthfulness; each adapter's lifecycle test builds only when its dependency is enabled.
