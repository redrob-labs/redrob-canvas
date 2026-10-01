# Krita translation boundary — what may be translated, and under what duty

Pin: `fdbf33b2146735465bb8aa59928fbc1890ceb160` (the commit registered in `docs/upstream-sources.toml`).
Working checkout: `libs/` + `plugins/paintops`, sparse, **377 MB** against 10.2 GB for the full clone.
Survey scripts: `research/krita-licences.py`, `research/krita-licence-blockers.py` (in the loop workspace,
not in this repository).

This document exists because item 1c.2 intends to translate 619 files, and a translation is a derivative
work. The licence question had to be settled before the first line, not after 619 of them.

## The direction is fine, and that is not automatic

This product is **GPL-3.0-or-later**. Krita is predominantly **GPL-2.0-or-later**, which upgrades. Measured
across 4,478 C++ sources in the checkout:

| Licence | Files |
|---|---|
| GPL-2.0-or-later | 3,076 |
| LGPL-2.0-or-later | 904 |
| LGPL-2.1-or-later | 226 |
| GPL-3.0-or-later | 122 |
| **LGPL-2.0-only** | **53** |
| GPL, version stated in prose only | 33 |
| no licence statement at all | 32 |
| BSD-3-Clause | 16 |
| **LGPL-2.1-only** | **6** |
| multi-licence disjunctions, MIT, MIT OR BSL-1.0 | 5 |

## 59 files cannot come in, and none of them is a target

The `-only` variants are the hard edge. LGPL-2.0-only and LGPL-2.1-only permit conversion to **GPL version
2**, and GPL-2.0-only cannot become GPL-3.0 — a one-way door in the direction we need. 59 files carry one.

Where they live, and why that is lucky rather than clever:

| Directory | Files |
|---|---|
| `libs/widgetutils/xmlgui` | 21 |
| `libs/ui/widgets` | 10 |
| `libs/ui` | 8 |
| `libs/widgets` | 7 |
| `libs/widgetutils/config` | 6 |
| `libs/widgetutils` | 3 |
| `libs/global` | 2 |
| `libs/ui/animation` | 2 |

Every one is Qt/KDE UI plumbing — an RSS reader, an FFmpeg wrapper, XML GUI builders, a backup helper. This
product has its own Qt layer and translates none of it. **Checked rather than assumed**: each of the eight
areas 1c.2 names was matched against the blocked set by symbol name, and all eight came back with zero.

## Two files looked unlicensed and are not

- `libs/global/KisRollingSumAccumulatorWrapper.h` carries **no statement**, and sits in the FIRST area
  1c.2 translates (the latency tracker). Its own `.cpp` and its sibling
  `KisRollingMeanAccumulatorWrapper.h` are both GPL-2.0-or-later, so it is part of that work and comes in.
- `plugins/paintops/libpaintop/kritapaintop_export_instance.h` writes **`SPDX-License-Ref:`**, which is not
  a valid SPDX tag — the tag is `SPDX-License-Identifier`, and `LicenseRef-` is a prefix for custom licence
  ids, not a tag name. An upstream typo, so the file is invisible to every SPDX scanner including ours. It
  is GPL-2.0-or-later by intent, and its contents are MSVC/MinGW export macros with nothing to translate.

## The duty: a new `translated` kind

`kind = "algorithm"` says behaviour is the authority and **no file is reused**. That is true today and stops
being true the moment a translated file lands. Rather than let the classification go stale — which is how
`library` became a lie in the query repository once plugin files started shipping — `verify-upstream.sh`
gained a fifth kind, `translated`, validated exactly like `code`:

- an attribution section in `UPSTREAM_NOTICES.md`,
- the pin recorded there,
- a declared `boundary` saying where translating stops,
- an inbound-compatible licence, with `-only` variants refused by name.

It is a separate kind from `code` because the two differ in what a reader must then do: for `code` the
re-sync method is a diff against the upstream file, and for `translated` there is no file to diff — only
behaviour can be compared. Calling both `code` would send the next reader looking for a diff that cannot
exist.

Krita stays `algorithm` until the first translated file exists. Flipping it now would have the registry
claim a derivative work that has not happened.

The `-only` check was reverse-verified across seven licence strings, and its first form was **wrong**: an
unanchored `[A-Za-z0-9.]-only` matched the prose word "algorithm-only" already in Krita's own licence field
and refused a correct entry. It is now anchored to a version number, which is how SPDX spells these.
