# Branding — login-screen watermark + §10.4 disclaimer

`addon-contrib/ToTBranding.lua` sets the login-screen version watermark and
the mandatory §10.4 fan-project disclaimer when the WoW 3.3.5a client loads.

## Pipeline

- **Stage A** (`compose-tot-addon.py`): discovers `addon-contrib/manifest.toml`,
  copies `ToTBranding.lua` into the unified `ThreadsOfTime` AddOn output under
  `build/tot-addon/ThreadsOfTime/` with the priority-prefixed filename
  `05-TotBranding-01-ToTBranding.lua`. The `@TOT_VERSION@` token is **not**
  substituted here — it is carried through as a literal token.

- **Stage B** (`pack-mpq.py`): substitutes `@TOT_VERSION@` with the release
  version before packing the final `patch-ZZ-tot-<version>.MPQ`.

## Module status

This is a **synthetic non-AC "module"** — it lives under `tot/client-patch/`
rather than `modules/` because it has no C++ server-side component. The composer
discovers it via an additional `SCRIPT_DIR / "branding" / "addon-contrib" /
"manifest.toml"` lookup alongside the normal `modules/*/data/addon-contrib/`
glob.

## Priority

**priority = 5** (core band, loads before feature mods at 10-89).

## Scope for 1.0.0

Text-only watermark. Splash imagery (custom login screen background) is out of
scope for 1.0.0 and deferred to a future release.

## Pre-flight checklist (verify on real 3.3.5a client)

1. Confirm `VersionLabel` is the correct FontString name in the 3.3.5a GlueXML.
   If absent, the hook no-ops harmlessly; if the name differs, update `ToTBranding.lua`.
2. Confirm glue-screen AddOns load on this build. If they do not, ship the
   branding via the FrameXML-override rail instead (design §5.3).
