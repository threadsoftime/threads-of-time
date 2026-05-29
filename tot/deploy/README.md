# tot/deploy/

Reference deployment stack for operators running Threads of Time.

## Status (as of Plan 1)

The `quadlet/` directory contains the current build-box-specific Quadlet units
as committed. They reference build-box-specific paths, ports, hostnames, and
credentials.

**Plan 5 generalizes these into operator-configurable stacks**: replacing
build-box specifics with `.env`-driven variables, adding a Compose-based alternative
for non-systemd operators, and producing the install script + first-boot
bootstrap.

DO NOT distribute these files as-is to operators. They are committed here
as the source from which Plan 5 derives the generalized stack.

Refs: spec §6, Plan 5.
