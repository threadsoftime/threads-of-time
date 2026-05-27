# dbc-patch-builder

Builds `patch-Z.MPQ` for Bracket 1 tier-set UI. See
`docs/superpowers/specs/2026-05-23-bracket1-tier-set-ui-client-patch-design.md`.

## Quick start

```bash
python -m venv .venv && source .venv/bin/activate
pip install -e '.[test]'
pytest
python -m dbc_patch_builder.build
# -> build/patch-Z.MPQ
```

## Determinism

Same inputs MUST produce byte-identical MPQ. CI gate:
`python -m dbc_patch_builder.check` parses the committed artifact and
asserts no drift from the fixture sources.

## External dependencies

- `mpqeditor` from StormLib (install via `brew install stormlib` on macOS,
  `apt install libmpq-tools` on Debian). Fall back to `pip install python-mpq`
  if neither is available.
