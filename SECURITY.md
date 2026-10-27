# Security Policy

## Supported versions

Redrob Canvas is pre-1.0 and has no tagged release yet. Security fixes land on
`main`; there is no backport branch for earlier commits.

## Reporting a vulnerability

Do not open a public issue containing exploit details, credentials, or the
contents of a file that reproduces the problem.

Report privately through GitHub's **Report a vulnerability** button on the
repository's Security tab (private vulnerability reporting is enabled), or email
`security@redrob.ai`.

Include:

- the affected commit and operating system;
- the build configuration, including whether `REDROB_ENABLE_BABL` or
  `REDROB_ENABLE_LCMS` was ON;
- reproduction steps using a synthetic document;
- the expected and observed behavior;
- impact and prerequisites; and
- any suggested mitigation.

Expect an acknowledgement within five business days.

## Scope

In scope: the Rust crates under `crates/`, the egui shell and command line under `apps/redrob-canvas`,
the local AI engine bridge in `crates/redrob-ai`, and the build and distribution
tooling under `scripts/` and `tools/`.

The editors under `editors/` (Photo, Vector, Layout and their shared crates) are
in scope too: they are developed here. Out of scope: vulnerabilities in IOPaint --
report those to its maintainers. If one is reachable *through* this project's
integration or build configuration, that is in scope and we want to hear about it.
