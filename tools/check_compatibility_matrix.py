#!/usr/bin/env python3
"""Check docs/compatibility.md: every `parity` row cites a harness, and every citation names a real one.

WHY THIS EXISTS.

`parity` is the output of stage 3 of the porting plan -- the claim that this product matches its authority, as
opposed to `native`, which claims only that an implementation is here. Until now nothing checked the matrix at
all in this repository. redrob-recall's validator refused EVERY parity claim outright, which was right while no
harness existed and useless once four did.

Two failures are possible and this catches both:

  * a row claiming parity with nothing to justify it -- a judgement dressed as a measurement;
  * a row citing a harness that does not exist, or no longer does -- a citation that cannot be followed, which
    is worse than none because it looks checked.

It also found the reason it was needed: the matrix had gone fifteen cycles stale. Dab shapes, GBR and ABR tips,
spacing, flood fill, tone curves, the downscale filter, the dirty-region projection and the latency tracker were
all built and none had a row. A matrix that does not describe the product cannot be the thing stage 3 is
measured by.
"""

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MATRIX = REPO / "docs" / "compatibility.md"
STATUSES = {"planned", "native", "adapter", "parity"}

# A citation key names a harness and the artefact within it that carries the comparison.
#   golden:<probe>  -> tools/golden/probes/<probe>.cpp AND tools/golden/expected/<probe>.txt
#   drift:<file>    -> tools/upstream-diff/recorded/<file>.diff
CITATION = re.compile(r"`(golden|drift):([A-Za-z0-9_.-]+)`")


def matrix_rows(text):
    """The feature rows only. The adapter table later in the file has its own shape."""
    for number, line in enumerate(text.split("\n"), 1):
        if not line.startswith("| ") or line.startswith("| Domain") or set(line) <= set("|- "):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) == 6 and cells[4] in STATUSES:
            yield number, cells


def main():
    if not MATRIX.exists():
        print(f"error: {MATRIX} is missing")
        return 1
    text = MATRIX.read_text(encoding="utf-8")
    rows = list(matrix_rows(text))
    if not rows:
        # A parser that finds nothing would pass every check vacuously, which is the one outcome this must
        # never report as success.
        print("error: no matrix rows parsed; the table shape changed and this guard is now blind")
        return 1

    problems = []
    counts = {status: 0 for status in STATUSES}
    cited = set()
    verified_not_parity = []

    for number, cells in rows:
        domain, feature, authority, route, status, evidence = cells
        counts[status] += 1
        keys = CITATION.findall(evidence)

        if status == "parity":
            if not keys:
                problems.append(
                    f"line {number}: {domain}/{feature[:40]} claims parity but cites no harness. "
                    f"`parity` is a citation, not a judgement"
                )
        elif keys:
            # NOT an error, and this was a correction. The first version of this guard rejected evidence on a
            # non-parity row, reasoning "promote it or drop the citation". redrob-recall then produced the case
            # that disproves it: its grammar harness verifies that our search grammar is bloop's REDUCED, with
            # three constructs that are ours and not bloop's at all. That is a real, checkable relationship and
            # it is not equivalence, so the row must stay `native` while carrying its evidence. Reported so
            # nothing hides, but a row citing a harness without claiming parity is legitimate.
            verified_not_parity.append(f"{domain}/{feature[:40]}")

        for kind, name in keys:
            cited.add((kind, name))
            if kind == "golden":
                for path in (
                    REPO / "tools" / "golden" / "probes" / f"{name}.cpp",
                    REPO / "tools" / "golden" / "expected" / f"{name}.txt",
                ):
                    if not path.exists():
                        problems.append(
                            f"line {number}: cites `golden:{name}` but {path.relative_to(REPO)} does not exist"
                        )
            elif kind == "drift":
                path = REPO / "tools" / "upstream-diff" / "recorded" / f"{name}.diff"
                if not path.exists():
                    problems.append(
                        f"line {number}: cites `drift:{name}` but {path.relative_to(REPO)} does not exist"
                    )

        if not authority:
            problems.append(f"line {number}: {domain}/{feature[:40]} names no authority")
        if not route:
            problems.append(f"line {number}: {domain}/{feature[:40]} describes no route")

    # The reverse direction. A recorded drift diff that no row cites is not an error -- a file may be copied
    # without its own matrix row -- but a golden PROBE nobody cites is worth reporting, because the probe exists
    # precisely to justify a claim and an uncited one means either a missing row or a dead probe.
    probes = sorted(p.stem for p in (REPO / "tools" / "golden" / "probes").glob("*.cpp"))
    uncited = [probe for probe in probes if ("golden", probe) not in cited]

    print(f"compatibility matrix: {len(rows)} rows")
    for status in ("parity", "native", "adapter", "planned"):
        print(f"  {status:<8} {counts[status]}")
    print(f"  citations {len(cited)} across {len(probes)} golden probes")
    if uncited:
        print(f"  golden probes no row cites: {', '.join(uncited)}")
    if verified_not_parity:
        # A harness can verify a relationship that is not equivalence, so these are legitimate.
        print(f"  cited without claiming parity: {', '.join(verified_not_parity)}")

    if problems:
        print()
        for problem in problems:
            print(f"error: {problem}")
        return 1
    print("every parity claim cites a harness that exists")
    return 0


if __name__ == "__main__":
    sys.exit(main())
