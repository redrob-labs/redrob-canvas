# Golden-output harness

Stage 2 of the porting plan, for the Krita translations in `crates/redrob-core`.

## The problem it fixes

Every reference constant those translations assert came from Krita's own algorithm, extracted with its
arithmetic untouched, compiled and run. That was the rule from the first translation: **verify a translation by
running the upstream, not by reading it.** Seven such probes were written, one per block.

All seven lived in a scratch directory that is reclaimed between sessions. So while every number in the Rust
was obtained honestly, **no one could regenerate one, check one, or notice if the pinned upstream had changed
underneath them.** Repairing that was the first item of stage 2, before any new comparison was built.

Recovering them also showed why this could not wait: one probe had been damaged by a later edit into a form
that no longer compiled, and nothing would ever have reported it.

## The two halves

Both are needed, and neither is sufficient.

| | What it proves | Needs the upstream? |
|---|---|---|
| `tools/golden/run.sh` | Krita still prints what we recorded | yes |
| `crates/redrob-core/tests/golden_upstream.rs` | our translation agrees with that record | no |

`run.sh` on its own only proves Krita is deterministic — it diffs the upstream against itself. The Rust test
closes the loop from the upstream's numbers to ours. A translation that drifted, or a constant someone
"tidied", fails in the test; a pinned upstream that moved fails in the script.

The transcripts are **committed**, so the test runs on a machine with neither Krita nor boost nor lcms2. That
is deliberate: a contributor who cannot build the upstream must still be able to check the values. What they
cannot do is regenerate them.

## Running it

```bash
./tools/golden/run.sh --list      # which probes can build here
./tools/golden/run.sh             # compile, run, diff against expected/
./tools/golden/run.sh --update    # re-record, for a deliberate pin bump
cargo test -p redrob-core --test golden_upstream
```

## It never passes silently

The same rule that keeps the database dialect harness out of `npm test`: **a test that passes when its subject
is absent is worse than no test.** Reverse-verified, all three failing as they must:

| Provoked | Result |
|---|---|
| one recorded digit changed | `DIFFERS`, exit 1 |
| `REDROB_LCMS_PREFIX` pointed at nothing | `UNRESOLVED -- lcms2 is not available`, exit 1 |
| a transcript deleted | the Rust test panics naming the file and the command to restore it |

An unresolved dependency is a FAILURE, not a skip. Nothing compared means nothing verified, and the script says
so and exits 1.

## The probes

| Probe | Upstream | Needs | Checked by the Rust test |
|---|---|---|---|
| `krita-mask-probe` | `kis_circle_mask_generator.cpp` | — | yes, the x-axis sweep at fade 0.5 |
| `krita-spacing-probe` | `kis_paintop_utils`, `kis_distance_information.cpp` | — | yes, the exact dab positions |
| `krita-gbr-probe` | `kis_gbr_brush.cpp` | — | yes, the coverage rows |
| `krita-antialias-probe` | `kis_circle_mask_generator.cpp` | — | transcript diff only |
| `krita-latency-probe` | `kis_latency_tracker.h`, `KisFilteredRollingMean` | boost | transcript diff only |
| `krita-spline-probe` | `kis_cubic_curve.cpp` | Eigen | transcript diff only |
| `lab-difference-probe` | `LcmsColorSpace.h` via real lcms2 | lcms2 | the self-comparison row |

The four marked "transcript diff only" print prose around their numbers instead of a parseable table. Their
values are asserted inline by the tests written alongside each translation, and `run.sh` still diffs their
whole output, so a change cannot pass unnoticed. Giving each a machine-readable block is the next step and was
deliberately not done here: **editing a probe invalidates the transcript it produced**, and the two would have
to be re-recorded in the same commit that changed what they mean.

## Fetching the dependencies

No root is needed. Each is an `apt-get download` plus `dpkg-deb -x` into a private prefix under
`$KIROCREW_SCRATCH`; the script looks there by default and each path is overridable:

| Variable | Default |
|---|---|
| `REDROB_BOOST_PREFIX` | `$KIROCREW_SCRATCH/boostprefix/usr/include` |
| `REDROB_EIGEN_PREFIX` | `$KIROCREW_SCRATCH/eigenprefix/usr/include/eigen3` |
| `REDROB_LCMS_PREFIX` | `$KIROCREW_SCRATCH/lcmsprefix/usr/include` |
| `REDROB_LCMS_LIB` | `$KIROCREW_SCRATCH/lcmsprefix/usr/lib/x86_64-linux-gnu` |

```bash
apt-get download libboost1.83-dev libeigen3-dev liblcms2-dev liblcms2-2
dpkg-deb -x libboost1.83-dev_*.deb "$KIROCREW_SCRATCH/boostprefix/"
dpkg-deb -x libeigen3-dev_*.deb    "$KIROCREW_SCRATCH/eigenprefix/"
dpkg-deb -x liblcms2-dev_*.deb     "$KIROCREW_SCRATCH/lcmsprefix/"
dpkg-deb -x liblcms2-2_*.deb       "$KIROCREW_SCRATCH/lcmsprefix/"
```

The probes do not need a Krita checkout: each carries the extracted algorithm as committed source, with the
upstream file and licence named in its header. The checkout is needed only to write a NEW probe, and
`UPSTREAM_NOTICES.md` records the pin it must be taken at.

## What is not here yet

The plan's other three harnesses — canvas vector against a Graphite render, query against a headless Beekeeper,
recall against a bloop index — are separate stage-2 items. No matrix row may be promoted to `parity` until the
harness covering it exists and runs; `scripts/verify-upstream.sh` enforces that no row claims `parity` today.
