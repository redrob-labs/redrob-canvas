# Parity inventories (clean room)

Clean-room feature inventories of each Redrob Canvas mode against its reference application. Every row
was written from the reference's public manual and from running the installed binary as a black box;
no reference source was read. The `manual` field is the page consulted (all verified reachable), the
`behaviour` text is our own wording, and `ours` / `ours_where` come from reading our crates.

Schema (one `[[feature]]` table per row):

| field | meaning |
|---|---|
| `id` | `<mode>-<group>-<nnn>`, stable once published |
| `group` | coarse area used for the counts below |
| `name` | feature as a user would name it |
| `reference` | reference application the behaviour was observed in |
| `manual` | manual page read (HTTP 200 at inventory time) |
| `behaviour` | what the feature does, in our words |
| `ours` | `none`, `partial` or `full` |
| `ours_where` | our crate paths (and command ids) implementing it, empty when none |
| `gap` | what is missing when `partial` or `none` |
| `priority` | 1 daily tool, 2 common, 3 rare |
| `evidence` | optional screenshot path under `docs/parity/evidence/` |

## Files

| File | Reference | Rows | none / partial / full |
|---|---|---:|---|
| `photo.toml` | GIMP 3 (Photo mode: selection, paint, transform, colour, filters, layers, channels, image, formats, dialogs, scripting) | 450 | 119 / 85 / 246 |
| `paint.toml` | Krita 5.3 (paint depth: brush engines, settings, sensors, presets, resources, tools, assistants, colour, layers, filters, animation, colour management, workflow) | 334 | 116 / 89 / 129 |

## photo.toml -- Photo mode (450 rows)

| group | rows | none | partial | full |
|---|---:|---:|---:|---:|
| selection | 24 | 3 | 9 | 12 |
| paint | 19 | 2 | 7 | 10 |
| transform | 14 | 1 | 3 | 10 |
| tools | 6 | 1 | 0 | 5 |
| color | 48 | 13 | 13 | 22 |
| filter | 149 | 75 | 26 | 48 |
| layer-mode | 39 | 9 | 5 | 25 |
| layer | 29 | 2 | 2 | 25 |
| channels | 7 | 0 | 3 | 4 |
| image | 23 | 1 | 2 | 20 |
| file | 10 | 2 | 1 | 7 |
| format | 32 | 4 | 5 | 23 |
| edit | 8 | 2 | 1 | 5 |
| view | 12 | 1 | 2 | 9 |
| dialogs | 23 | 2 | 5 | 16 |
| scripting | 7 | 1 | 1 | 5 |
| **total** | **450** | **119** | **85** | **246** |

Reading the Photo numbers: the daily surface (selection, paint, transform, layers, image, file, formats,
colour adjustments) is largely covered; the `none` column is dominated by the reference's long tail of
generator and script filters (render, decor, animation, legacy artistic) and by its LCh/grain layer
modes. Priority-1 rows with `partial` are the first targets.

## paint.toml -- paint depth (334 rows)

By group: engine 19, tip 12, settings 35, sensor 18, preset 14, resource 16, tool 47, assistant 17,
color 17, layer 46, filter 51, animation 12, colormgmt 10, workflow 20. Totals: none 116, partial 89, full 129.
