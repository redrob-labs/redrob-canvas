# File formats and how a file reaches a mode

Each mode's codecs are its editor's, built from source under `crafts/`. This page records what the
shell adds on top: which mode a file opens in, and which tables are authoritative for the rest.

## Routing

A file given on the command line, dropped on the window or chosen in Open is routed by extension to
the mode whose editor owns that format; the editor then does its own content detection and refuses a
renamed file the way its upstream does.

| Extensions | Mode | Notes |
| --- | --- | --- |
| `.rphoto`, `.psd`, `.psb`, `.ora`, `.kra`, `.pdn`, `.tif`/`.tiff`, `.png`, `.jpg`/`.jpeg`, `.webp`, `.gif`, `.bmp`, `.tga`, `.ico`, `.qoi`, `.pnm`, `.exr`, `.hdr`, `.fits`, `.heic`/`.heif`, camera RAW extensions, `.af`/`.afphoto`/`.afdesign`/`.afpub` | Photo | the Photo editor's codec crate; raster formats default here even though Vector and Layout can also place them |
| `.rvector`, `.svg`, `.svgz`, `.ai`, `.eps`, `.pdf`, `.dxf`, `.emf`, `.wmf` | Vector | PDF opens as editable vector pages here; Layout places PDFs as assets instead |
| `.rlayout`, `.idml` | Layout | the Layout editor's native format and InDesign interchange |

Routing is the shell's (`apps/redrob-canvas`); a mode can still open or place any format its editor
supports from its own menus, so the table above is the default, not a restriction. Routing by
extension at launch is 0.7 work; until it lands, open the mode first and use its File menu.

## Authoritative tables

The upstream docs and READMEs are not in this tree (`tools/prune_upstream.py` removes them after
every pull); the tables below are the code.

- Photo: `editors/photo/crates/codecs/README.md` (the capability matrix per format and bit
  depth) and the `OPENABLE` list in `editors/photo/crates/engine/src/file_cmds.rs`; the
  upstream project's docs on ORA, KRA, PDN and PSD are at
  [storytold/photocraft](https://github.com/storytold/photocraft) under `docs/`.
- Vector: `FORMATS`, `OPEN_EXTS` and `PLACE_EXTS` in
  `editors/vector/crates/engine/src/cmd/fileio/mod.rs` (SVG, PDF and `.ai` with PDF/X, EPS,
  DXF, EMF/WMF, Export for Screens, Print, Package).
- Layout: `opens_as_document` in `editors/layout/crates/ui-egui/src/lib.rs` and the `file.*`
  commands of `editors/layout/crates/engine` (IDML in and out, PDF/X-4 and PDF/A-2b, editable
  PPTX, EPUB).

## Loss policy

Each editor warns rather than silently degrades: the Photo editor reports depth reduction on 8-bit targets
and lists what an Affinity import could not model; the Vector editor and the Layout editor report what a PDF or
IDML round trip drops. Redrob Canvas adds no conversion step of its own between modes. Moving work
from one mode to another is an export from the first and an open or place in the second, so the
formats in the table are the contract.

## Native formats

`.rphoto` (Photo), `.rvector` (Vector, JSON) and `.rlayout` (Layout) are each editor's own
documented format under a Redrob extension; the bytes are the upstream format, versioned by the
upstreams, and the JSON header keeps the upstream `"format"` value so a file written here opens in
an upstream build. Redrob Canvas has no project format of its own; the GPL-era `.rrg` format was
retired with the Qt shell in 0.7.0 and nothing reads it.

### Old extensions

The upstream extensions still open, everywhere a file is routed or chosen (`routing.rs`, the Open
filters, each editor's open path, its recovery scan and template list): `.pcraft` opens as Photo
(by name and by content), `.vectorcraft` and `.drawcraft` as Vector, `.designcraft` as Layout.
Saving such a document suggests the Redrob extension; nothing writes an old one. In the code every
such line is marked `// legacy:`, which `tools/rename_identifiers.py` skips and
`scripts/check_upstream_ids.py` allows.
