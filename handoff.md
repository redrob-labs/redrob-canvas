# Redrob Graphics handoff

## Project state

Redrob Graphics is a Rust 1.92 + Qt 6/QML graphics editor with a product-owned typed document model, command bus, native UI, and approval-gated Agent workflow. It is not a Krita or GIMP UI fork.

Behavioral sources of truth are:

- Krita: painting, brush/stroke behavior, layer projection, and animation.
- GIMP/GEGL: filters, selections, compositing, and color behavior.
- Redrob Code: API, authentication, and streaming protocol behavior.
- Redrob Graphics: Rust domain model, typed `CommandBus`, modern Qt UX, and Agent proposal/apply workflow.

Pinned upstream revisions and the GEGL floor are recorded in `docs/upstream-sources.toml`. “Native” in `docs/compatibility.md` means implemented locally; it does not imply full upstream parity unless a named fixture and acceptance criterion exist.

## Architecture and ownership

- `crates/redrob-core/`: authoritative document, layers/groups/masks, timeline, selections, raster and semantic rendering, file codecs, command execution, and history.
- `crates/redrob-agent/`: bounded Redrob HTTP/SSE client and provider DTOs.
- `crates/redrob-ffi/`: stable ABI v2 with opaque editor ownership, owned buffers, coherent projections, format APIs, and Agent proposal APIs.
- `native/qt/`: `EditorBridge`, Qt models, immutable canvas snapshots, playback, file workflows, proposal queue, and native smoke validation.
- `native/adapters/`: optional GEGL seam and Krita source-boundary scaffold. Both are OFF by default and currently advertise no product operations or formats.
- `qml/`, `resources/`: product UI and embedded assets.
- `tools/`: deterministic third-party notices and corresponding-source generation, plus source-integrity tests.
- `docs/`: architecture, compatibility, format, licensing, and upstream policy.
- `semantic-review/`: independent review records; these are evidence, not runtime source of truth.

Rust owns all mutable editor state. Qt consumes generation-numbered immutable render/selection snapshots and coherent JSON projections through the C ABI. Rust allocations, Qt containers, C++ templates, and GObject pointers do not cross the ABI without explicit ownership rules.

## Mutation and Agent invariants

QML actions and Agent tools use the same serde-tagged `Command` model and `CommandBus`. Command execution clones the document, applies the edit and playback stop to a detached candidate, validates the complete candidate, and then commits exactly one generation and history transition. Rejected commands preserve the document, current frame, playback, generation, undo/redo, and projection. Undo/redo likewise validate detached targets before committing.

Frame selection and playback ticks are navigation, not history edits. Navigation advances generation so frame-dependent proposals and snapshots become stale, but it does not create history or clear redo.

Agent mutations are inert proposals until explicit Apply. Every proposal is bound to both the Rust generation and a Qt document epoch. Freshness is checked before dispatch; stale, malformed, rejected, or unsupported proposals do not mutate the editor and remain available for inspection/rejection. A current proposal can apply while playback is active because playback stops only inside the successful core transaction. Read-only Agent inspection never creates a mutation proposal.

## Implemented surface

The initial editor includes:

- Raster layers, isolated groups, raster masks, ordering, visibility, opacity, and Normal/Multiply/Screen/Overlay/Add compositing.
- Bounded undo/redo and transactional grouped edits.
- Pressure-aware round brush, moving-average smoothing, X/Y mirroring, fill/clear, linear/radial gradients, crop/resize, flips, 90-degree rotation, and affine transforms.
- Grayscale selections with rectangle/ellipse/all/invert and replace/add/subtract/intersect, feather, grow, and shrink.
- Typed filters including invert, grayscale, brightness/contrast, Gaussian/box blur, threshold, posterize, levels, hue/saturation, and sharpen.
- Sparse raster animation cels, stable frame IDs, copy-on-write duplication, FPS/range/loop controls, and native Qt playback.
- Deterministic text limited to printable ASCII/newline with embedded `font8x8` rendering.
- Deterministic bounded vector paths with move/line/cubic/close, non-zero/even-odd fill, solid fill/stroke, 1/256-pixel quantization, fixed cubic subdivision, and 4×4 coverage.
- `.rrg` v1 import and v2 project import/export.
- Strict content-detected PNG, JPEG, deterministic lossless WebP, bounded OpenRaster, and a restricted deterministic SVG subset.
- Explicit loss policy. JPEG alpha requires verified opacity or an explicit opaque matte; unsupported ORA/SVG semantics reject or require typed allow-loss handling.

Important planned gaps are documented in `docs/compatibility.md`: onion skin, pass-through groups, full Krita brush presets/sensors, operational GEGL/Krita adapters, color-profile/display conversion, Unicode shaping and OS fonts, broader vector/SVG semantics, KRA/XCF/PSD, unrestricted codecs, and lossy WebP.

## Build prerequisites

- Rust/Cargo 1.92.0 with rustfmt and Clippy.
- Python 3.
- CMake 3.21+ at project level; use the minimum required by the selected Qt package (Qt 6.12 officially requires CMake 3.25).
- C/C++20 toolchain.
- Qt 6.8+ components: Core, Concurrent, Gui, Qml, Quick, QuickControls2, and Svg, plus Qt build tools.
- Unix system libraries `dl`, `pthread`, and `m`.
- GEGL only when enabled: pkg-config and `gegl-0.4 >= 0.4.66`.

## Canonical validation and build

Run from the repository root with adapters OFF unless intentionally validating an enabled adapter:

```sh
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF
python3 -m unittest discover -s tools/tests -p 'test_*.py'
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked --offline
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings

cmake -S native -B build/qt \
  -DCMAKE_BUILD_TYPE=Release \
  -DREDROB_BUILD_QT=ON \
  -DREDROB_ENABLE_GEGL=OFF \
  -DREDROB_ENABLE_KRITA=OFF
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
cmake --install build/qt --prefix /desired/prefix
```

If Qt is installed outside the system prefix, add `-DCMAKE_PREFIX_PATH=/path/to/qt`. The application binary is normally `build/qt/qt/redrob-graphics`.

Qt-independent native adapter/distribution validation:

```sh
cmake -S native -B build/adapters -DREDROB_BUILD_QT=OFF -DBUILD_TESTING=ON
cmake --build build/adapters
ctest --test-dir build/adapters --output-on-failure
```

`REDROB_ENABLE_KRITA=ON` additionally requires `REDROB_KRITA_SOURCE_DIR` at the exact pinned commit. It remains a source-verified, non-linking scaffold. Do not describe either adapter as product-ready until its capability operation/format arrays are non-empty and compatibility fixtures pass.

## Distribution and compliance

The project is GPL-3.0-or-later. A distributable build must include:

- `LICENSE`
- generated `THIRD_PARTY_NOTICES.md`
- generated, Git-state-stable `SOURCE_OFFER.md`
- `redrob-graphics-corresponding-source.tar.gz`

Refresh tracked artifacts after changing `Cargo.lock`, source policy, declared repository, or adapter selection:

```sh
python3 tools/generate_distribution_artifacts.py --generate --gegl OFF --krita OFF
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF
```

Build and verify corresponding source:

```sh
cmake --build build/qt --target redrob_source_bundle
python3 tools/generate_distribution_artifacts.py \
  --check-source-bundle build/qt/redrob-graphics-corresponding-source.tar.gz \
  --gegl OFF --krita OFF
```

CMake configure, build, and install fail closed on stale or missing compliance artifacts. Cached registry `.crate` archives are checked against `Cargo.lock`; archive paths/types are validated; unpacked Cargo trees must exactly match the verified archive; notices and bundled vendor bytes come from the archive-authoritative snapshot.

`SOURCE_OFFER.md` intentionally excludes live Git identity so the first commit does not create a self-referential tracked hash. Each generated source archive instead contains `.redrob-source-identity.json` with the generation-time branch, actual commit when one exists, public repository-local origin, and declared repository. Generate the release archive from the final merged commit.

## Agent configuration

The desktop bridge currently consumes `REDROB_API_KEY`. Without a key it makes no network request and uses a bounded deterministic local proposal parser; this is not a general offline LLM. `REDROB_BASE_URL`, `REDROB_MODEL`, and desktop device authorization are not fully wired even though placeholders/primitives exist. Either implement and document that contract or remove unsupported configuration promises before claiming those flows.

Never log API keys, raw authorization codes, unredacted provider payloads, or raw user text from network-safe Agent context.

## Latest validation before handoff

The final local verification before this handoff passed:

- Rust formatting.
- 180 Rust/core/Agent/FFI tests.
- strict Clippy over workspace, all targets, and all features.
- 18 Python distribution/source-integrity tests.
- fresh Qt 6.12 Release build.
- CTest 5/5: distribution integrity, adapter capability, restricted QML lint, offscreen native smoke, and minimal-platform smoke.
- install verification for executable, GPL license, notices, source offer, and corresponding-source archive.
- deterministic source-bundle regeneration/check with 9,455 archive entries.
- native smoke screenshot at 1440×900.

Validation hashes identify only that exact pre-commit snapshot and are not permanent release constants. Re-run the full matrix after merge and generate the release source archive from merged `main` so its identity contains the real commit.

## Immediate continuation priorities

1. Re-run the complete clean validation matrix on merged `main` and generate/check the corresponding-source archive from that commit.
2. Add CI for Rust fmt/test/Clippy, Python source-integrity tests, Qt 6.8 minimum and validated Qt 6.12, QML lint/smoke, install checks, and OFF/OFF adapter defaults. CI workflow changes must use review rather than direct pushes.
3. Resolve desktop Agent configuration/device-auth behavior and document only implemented flows.
4. Add compatibility fixtures before expanding Krita/GEGL adapters or claiming parity.
5. Prioritize onion skin, color management, text shaping, and additional file formats without weakening typed-command atomicity, resource bounds, strict loss policy, or release gates.
