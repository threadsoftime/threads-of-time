# Threads of Time — Player Install Guide

**Version:** see the filename of the MPQ you downloaded  
**Realm:** Heimdal (private; invitation only)

---

## Requirements

| Requirement | Details |
|---|---|
| WoW client | 3.3.5a (build 12340) — any locale |
| Free disk space | ~5 MB for the patch file |
| Existing `Data/` patch files | Any `patch-A.MPQ` through `patch-Y.MPQ` are fine; do NOT rename one of those |

---

## Install (3 steps)

### Step 1 — Copy the patch file

Place the downloaded file into your WoW `Data/` directory and rename it to
`patch-ZZ.MPQ` (two capital Z's, exactly as shown):

```
WoW/
  Data/
    patch-ZZ.MPQ   ← put it here
    enUS/          ← locale folder, leave untouched
    ...
```

The `ZZ` suffix loads last among all patch files, ensuring ToT overrides take
precedence over any other addons or patches.

### Step 2 — Point your client at the Heimdal realm

Edit (or create) `WoW/Data/enUS/realmlist.wtf` and set:

```
set realmlist heimdal.example.com
```

Replace `heimdal.example.com` with the actual realm address provided by your
invitation.

### Step 3 — Launch the game

Start `WoW.exe` (or `Wow.app` on macOS). Log in normally. The login screen
should display a small disclaimer in the bottom-left corner indicating the
Threads of Time client patch is active.

---

## One-time: clear itemcache.wdb

If you have played on this account before with a different patch configuration,
the client may have cached stale item tooltips. Clear the cache once:

1. Exit WoW completely.
2. Delete (or move aside) `WoW/Cache/WDB/enUS/itemcache.wdb` (and
   `itemnamecache.wdb` if present).
3. Relaunch. WoW rebuilds the cache automatically on first login.

You only need to do this when upgrading from a pre-ToT patch or a different
version of the ToT MPQ.

---

## Verify the install

After logging in, confirm each of the following:

**Login-screen disclaimer**  
A small line of text in the bottom-left corner of the login screen reads
something like *"Threads of Time client patch active (v1.x.x)"*. If you do
not see this, the MPQ is not loading — double-check the filename and location.

**Tier tooltip bonus rows**  
Hover over any Bracket-set item in your inventory (items awarded inside ToT
dungeons). The tooltip should show set-bonus descriptions specific to your
class and spec, not generic Blizzard tier text.

**Warforged tag (~10% chance)**  
Items that dropped as Warforged (approximately 1 in 10 drops from ToT
bosses) display a gold *"Warforged"* suffix in their name and a short
enchantment line in the tooltip. If Warforged items show no special tooltip
text, the SpellItemEnchantment DBC is not loading correctly — clear
itemcache.wdb (see above) and relog.

---

## Upgrading to a newer version

1. Exit WoW.
2. Replace `Data/patch-ZZ.MPQ` with the new file (same name, same location).
3. Clear itemcache.wdb (see above).
4. Relaunch.

You do NOT need to uninstall anything; overwriting the file is sufficient.

---

## Common errors

| Symptom | Likely cause | Fix |
|---|---|---|
| No login-screen disclaimer | MPQ not loading | Confirm filename is exactly `patch-ZZ.MPQ` in `Data/`, not a subfolder |
| Stale/wrong set-bonus tooltips | Old itemcache | Delete `Cache/WDB/enUS/itemcache.wdb` and relog |
| Warforged items have no enchant line | Old itemcache or wrong patch version | Clear cache; confirm MPQ version matches realm version |
| Client crash on login | Conflicting `patch-ZZ.MPQ` from another source | Ensure only the official ToT MPQ uses the `ZZ` slot |
| Realm not found / cannot connect | Wrong realmlist | Re-check `realmlist.wtf`; ensure no extra spaces or quotes |

---

## Legal / Disclaimer

Threads of Time is a fan-made private-server project not affiliated with or
endorsed by Blizzard Entertainment. World of Warcraft® and all related assets
are the property of Blizzard Entertainment, Inc. This patch is provided for
educational and entertainment purposes on a private, invitation-only server.
Redistribution outside the Heimdal community is not permitted.
