# Security Policy

## Supported versions

Redrob Graphics is pre-1.0 and has no tagged release yet. Security fixes land on
`main`; there is no backport branch for earlier commits.

## Reporting a vulnerability

Do not open a public issue containing exploit details, credentials, or the
contents of a file that reproduces the problem.

Report privately through GitHub's **Report a vulnerability** button on the
repository's Security tab (private vulnerability reporting is enabled), or email
`security@redrob.ai`.

Include:

- the affected commit and operating system;
- the build configuration, including whether `REDROB_ENABLE_GEGL` or
  `REDROB_ENABLE_KRITA` was ON;
- reproduction steps using a synthetic document;
- the expected and observed behavior;
- impact and prerequisites; and
- any suggested mitigation.

Expect an acknowledgement within five business days.

## Scope

In scope: the Rust crates under `crates/`, the Qt/QML shell under `native/` and
`qml/`, the adapter seams under `native/adapters/`, and the build and
distribution tooling under `scripts/` and `tools/`.

Out of scope: vulnerabilities in the pinned upstream projects themselves (GIMP,
Krita, GEGL) — report those to their maintainers. If a pinned upstream
vulnerability is reachable *through* this project's build configuration, that is
in scope and we want to hear about it.
