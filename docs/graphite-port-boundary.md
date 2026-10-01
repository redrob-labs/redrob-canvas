# What is portable from Graphite, and the boundary the plan had wrong

Item 1b.1. Measured before copying anything, because the four Krita measurements
each removed planned work that looked necessary until it was counted.

Reproduce against a sparse checkout of `node-graph` and `editor` at pinned commit
`d10ccbedac06497ee4766a7061d29281ee5df811`.

## The finding: canvas is layer-based, Graphite is node-based

The porting plan says "canvas ← Graphite: start with the `node-graph` core". That
instruction cannot be followed as written, and the reason is architectural rather
than technical.

Measured in `crates/redrob-core/src/document.rs`:

- **Zero** occurrences of "graph".
- `layers: Vec<Layer>` — an ordered stack.
- `NodeKind` is `Raster | Group | Text | Vector` — those are *layer types*, not graph
  operations.
- Two incidental mentions of edge/connect vocabulary, and no edge type.

Graphite's document is a directed graph of nodes with typed inputs and outputs,
executed by an interpreter. This product's document is a layer stack with a command
bus and an undo history.

**Porting `node-graph` would not add a feature. It would replace this product's
document model, its command system, its undo history, its `.rrg` file format, and
everything built on those.** That is a different product, and it is not what stage 1-b
was scoped to do.

So the boundary moves: **take the libraries, not the graph.**

## Where the coupling actually is

Every crate under `node-graph/libraries/` was checked for references to
`graph_craft`, `NodeNetwork`, `DocumentNode` or the `#[node_macro]` attribute:

| Crate | Files | Lines | Graph-coupled files |
|---|---|---|---|
| core-types | 24 | 5,481 | **0** |
| vector-types | 21 | 10,588 | **0** |
| no-std-types | 15 | 2,426 | **0** |
| graphic-types | 4 | 1,895 | **0** |
| raster-types | 3 | 792 | **0** |
| brush-types | 2 | 384 | **0** |
| graphene-hash | 2 | 367 | **0** |
| resources | 1 | 366 | **0** |
| application-io | 1 | 161 | **0** |
| rendering | 5 | 3,982 | 0 — excluded for wgpu |
| wgpu-executor | 8 | 1,001 | 0 — excluded for wgpu |
| canvas-utils | 2 | 221 | 0 — excluded, wasm |
| graph-craft | 13 | 4,869 | 9 |
| interpreted-executor | 13 | 2,391 | 9 |
| nodes | 67 | 22,571 | 35 |
| node-macro | 9 | 4,956 | 2 |
| preprocessor | 1 | 420 | 1 |

**The library layer is entirely free of the graph.** The coupling is confined to five
crates, and those five are exactly the graph architecture.

| Verdict | Files | Lines | Share |
|---|---|---|---|
| **portable — architecture-neutral libraries** | **73** | **22,460** | **35.4%** |
| graph architecture — needs Graphite's document model | 105 | 35,727 | 56.4% |
| excluded by the renderer and platform boundary | 15 | 5,204 | 8.2% |
| **total `node-graph`** | **193** | **63,391** | |

## The wgpu boundary is a Cargo feature, not a cut line

The plan assumed wgpu lives in the renderer and porting could "stop before" it. wgpu
is referenced in **43 files**, including `raster-types`, `graphic-types`,
`graph-craft` and `interpreted-executor` — so a file-level cut was never going to
work.

It does not need to. wgpu is an **optional Cargo feature**:

- `raster-types`: `wgpu = ["dep:wgpu"]`, and `default = ["serde"]` — off by default.
- `graph-craft`: `default = ["dealloc_nodes", "wgpu", "loading"]` — on by default and
  disableable.
- `graphic-types`: pulls `raster-types` with `features = ["wgpu"]` hardcoded. The one
  place it is forced, and the one place to patch.

And the fallback is real rather than a `compile_error!`. With the feature off,
`raster_types.rs` compiles a `mod gpu` in which `Raster<GPU>` is a zero-sized type
that reports `is_empty() == true` always. The CPU path is untouched.

**So the boundary is drawn by feature selection, and the renderer never has to be
read.**

## What to take first, and why it is the biggest single win in the programme

`vector-types`: **21 files, 10,588 lines, zero graph coupling.** Bezier path
algorithms, intersection, offsetting, splines, Poisson disk sampling, shape
construction, gradients. It depends on `kurbo`, which is the Rust ecosystem's Bezier
library rather than anything Graphite-specific.

It maps directly onto rows this product's compatibility matrix already lists as
`planned`:

> Vector | general SVG, transforms, gradients, dashes, CSS, scripts, external data,
> clipping, filters, animation, variable joins/caps, **path booleans**

A Bezier path is a Bezier path whether a graph node or a layer owns it. This is
architecture-neutral geometry, it is the largest coherent portable block found in
either upstream, and nothing in this product currently does any of it.

`gradient.rs` alone is 3,057 lines, and the matrix lists gradients as planned.

## The graph-uncoupled files inside `nodes/`

32 of the 67 files in `nodes/` do not touch the graph, 4,285 lines, and they are more
interesting than their size suggests:

| File | Lines | What it is |
|---|---|---|
| `brush/src/basic_brush/pipeline.rs` | 543 | brush stroke pipeline |
| `vector/src/voronoi.rs` | 472 | Voronoi diagram |
| `brush/src/basic_brush/render.rs` | 450 | dab rendering |
| `raster/src/color_lookup_table.rs` | 442 | LUT application |
| `raster/src/color_lookup_table/format_icc.rs` | 362 | ICC LUT parsing |
| `text/src/path_builder.rs` | 284 | text to path |
| `brush/src/basic_brush/stroke.rs` | 222 | stroke geometry |

The brush pipeline (1,378 lines across four files) is worth noting beside the Krita
measurement: `plugins/paintops` was 316 files to translate, and Graphite has a
working brush in Rust already. Whether it is a substitute or a starting point is a
comparison stage 2 should make rather than a guess here.

The ICC LUT parsing overlaps the lcms2 adapter already wired; the colour work should
check there first.

## Revised order for 1b.1

1. **`vector-types`** (10,588 lines) — architecture-neutral, fills existing `planned`
   rows, largest single win. Depends only on `core-types`, `graphene-hash` and
   ordinary crates.
2. **`core-types`** (5,481) and **`no-std-types`** (2,426) — whatever `vector-types`
   actually needs from them, taken by use rather than wholesale.
3. **`brush-types`** (384) and the graph-uncoupled brush pipeline in `nodes/` (1,378)
   — compared against the Krita paintops plan before either is chosen.
4. **`raster-types`** and **`graphic-types`** with `wgpu` off — only if something
   above needs them.
5. **Not `graph-craft`, `interpreted-executor`, `nodes/` proper, or `node-macro`.**
   Those are the graph, and adopting them is a product decision rather than a port.

## What this does to the plan's framing

The plan measured Graphite at 264k lines and treated the `node-graph` crate as the
first tranche. The measurement says:

- `node-graph` is 63,391 lines, not 264k — the rest is `editor` (273 files),
  `desktop`, `document` and `frontend`.
- Of those 63,391, **22,460 lines across 73 files are portable**, 35.4%.
- The 56.4% that is the graph is not a porting backlog. It is an architecture this
  product deliberately does not have.

**Nothing has been copied yet.** This cycle establishes the boundary; the copying
starts with `vector-types` in the next.
