# Redrob Canvas

**English** · [한국어](./README.ko.md)

Redrob Canvas is a modern, cross-platform, agentic graphics editor built with Rust and Qt Quick.

redrob-canvas is a new product architecture, not a UI fork. Krita and GIMP remain pinned, executable sources of truth while their proven behavior is moved behind a typed Rust command system and a modern QML experience. Painting behavior follows Krita; image processing behavior follows GIMP/GEGL; product UX and agent workflows belong to redrob-canvas.

## Project structure

```text
crates/
  redrob-core/       Document, layers, raster engine, selection, commands, history
  redrob-agent/      Redrob streaming API, tool loop, approvals
  redrob-ffi/        Stable C ABI consumed by the Qt host
native/qt/            Qt 6 host, QObject models, canvas bridge
qml/                  Modern Qt Quick interface
docs/                 Architecture, compatibility matrix, pinned upstreams
```

## Architecture rules

1. QML and agents submit the same typed commands.
2. Every mutating command is transactional, auditable, and undoable.
3. The render thread consumes immutable snapshots; it never owns document state.
4. Upstream behavior is pinned and tracked feature-by-feature.
5. Krita/GIMP code is integrated only behind explicit adapters; no upstream UI is carried forward.
6. Destructive and agent-generated work requires preview or an explicit commit boundary.

## Build

Core and protocol checks do not require Qt:

```bash
cargo test --workspace
```

The desktop shell requires Qt 6.8+ (validated with Qt 6.12) with Quick, Quick Controls 2, and SVG:

```bash
cmake -S native -B build/qt -DCMAKE_BUILD_TYPE=Release \
  -DREDROB_ENABLE_GEGL=OFF -DREDROB_ENABLE_KRITA=OFF
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
./build/qt/qt/redrob-canvas
```

The baseline build is dependency-free with respect to GEGL and Krita. Adapter intent is explicit: `REDROB_ENABLE_GEGL=ON` requires a system `gegl-0.4 >= 0.4.66`, while `REDROB_ENABLE_KRITA=ON` requires `REDROB_KRITA_SOURCE_DIR` exactly at commit `fdbf33b2146735465bb8aa59928fbc1890ceb160`. Enabling either scaffold does not create product operations; capability reports remain authoritative. A headless capability-only check is:

```bash
cmake -S native -B build/adapters -DREDROB_BUILD_QT=OFF
cmake --build build/adapters
ctest --test-dir build/adapters --output-on-failure
```

## Core file formats

`redrob-core` exposes a content-detected, typed format boundary for RRG, PNG, JPEG, lossless WebP, OpenRaster, and a deliberately limited deterministic SVG subset. The ABI-v2-compatible `redrob_editor_import_file` and `redrob_editor_export_file` symbols route those formats through strict, bounded JSON options and return effective metadata plus machine-readable warnings. Default options reject representational loss. JPEG alpha removal requires an explicit opaque matte or a verified-opaque image, and ORA/SVG adapters reject unsupported semantics rather than guessing. Existing `save_project`/`load_project`, `import_png`/`export_png`, and their C ABI wrappers remain available with their compatibility behavior.

The Qt/QML shell separates editable RRG project identity from interchange. Open Project and Save Project are RRG-only; Import and Export Current Frame advertise exactly PNG, JPG/JPEG, lossless WebP, ORA, and limited SVG. Import clears the project path, export never changes it or navigates the timeline, output publication uses `QSaveFile`, and explicit allow-loss/JPEG quality/opaque-matte controls prevent silent degradation. See `docs/formats.md` for exact routes and limits.

## Redrob

The client uses the hosted OpenAI-compatible streaming API at `https://console.redrob.ai/api/backend/v1`, defaults to model `auto`, and supports function tools. Set `REDROB_API_KEY`, or connect through the device authorization flow in the application.

## Distribution artifacts

The complete lockfile notice inventory and source offer are generated only from `Cargo.lock`, locked offline Cargo metadata, and local crate source/license files. The corresponding-source archive includes checksum-verified copies of every locked registry package plus an offline Cargo source-replacement configuration:

```bash
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF
cmake --build build/qt --target redrob_source_bundle
cmake --install build/qt --prefix /desired/prefix
```

`redrob_distribution_check` is available as a standalone CMake target. Installation reruns the deterministic check and requires the source bundle, `THIRD_PARTY_NOTICES.md`, and `SOURCE_OFFER.md`; see `docs/licensing.md` for the dependency-status policy.

## License

GPL-3.0-or-later. `LICENSE` is the verbatim GNU General Public License version 3 as required by section 4; `COPYRIGHT` carries this project's copyright notice and warranty disclaimer. See also `docs/licensing.md`, `THIRD_PARTY_NOTICES.md`, and `SOURCE_OFFER.md`.