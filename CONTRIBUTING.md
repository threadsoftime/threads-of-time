# Contributing to Threads of Time

Thank you for considering a contribution. This document covers the contributor license and the basic workflow.

## License of contributions

By submitting a pull request to this repository, you certify that you have the right to license your contribution under GPL-2.0-or-later and you agree to do so. This is the "inbound = outbound" model — your contribution ships under the same license as the project (matching AzerothCore's license).

No CLA, no DCO sign-off, no copyright assignment required.

## Workflow

1. Open an issue describing what you want to change before writing code, unless the change is small and obvious.
2. Fork the repo, create a feature branch off `dev`.
3. Make changes following the project conventions (see `docs/superpowers/specs/` for architecture context).
4. Open a PR back to `dev`. CI must be green.
5. Maintainers review and merge.

## Adding a new SPDX header

All new source files require an SPDX header on line 1:

```
// SPDX-License-Identifier: GPL-2.0-or-later
```

(Use `# SPDX-License-Identifier: GPL-2.0-or-later` for Python and shell.)

## Where to get help

See [README.md](README.md) for architecture overview. For implementation questions, the specs under `docs/superpowers/specs/` and `docs/superpowers/plans/` are the authoritative source.
