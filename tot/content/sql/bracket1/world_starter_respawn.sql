-- sql/bracket1/world_starter_respawn.sql
-- Drop respawn time for non-vendor, non-elite, low-level creatures in racial starter zones.
-- Companion to bracket-1's "1000 bots all start at level 1" launch event.
--
-- Reverted by sql/bracket2/world_starter_respawn_revert.sql when bracket cap moves to 35.
--
-- NOTE: Most creature rows have zoneId=0 (computed at runtime from DBC/map data, not persisted).
-- We therefore filter by coordinate bounding boxes for each starter zone instead of zoneId.
-- Approximate bounds sourced from WoW map geometry and verified against game_tele reference points.

-- Backup the original spawntimesecs so revert can restore exactly.
CREATE TABLE IF NOT EXISTS acore_world.creature_spawntime_backup_bracket1 (
    guid INT UNSIGNED NOT NULL PRIMARY KEY,
    spawntimesecs INT UNSIGNED NOT NULL
) ENGINE=InnoDB;

-- Take backup of current values for affected rows (idempotent: only inserts unbacked-up rows).
INSERT IGNORE INTO acore_world.creature_spawntime_backup_bracket1 (guid, spawntimesecs)
SELECT c.guid, c.spawntimesecs
FROM acore_world.creature c
JOIN acore_world.creature_template ct ON c.id1 = ct.entry
WHERE ct.minlevel <= 10
  AND ct.npcflag = 0
  AND ct.rank = 0
  AND c.spawntimesecs > 60
  AND (
    -- Elwynn Forest (Alliance Human starter) map=0
    (c.map = 0 AND c.position_x BETWEEN -10800 AND -8300 AND c.position_y BETWEEN -700 AND 800)
    OR
    -- Dun Morogh (Alliance Dwarf/Gnome starter) map=0
    (c.map = 0 AND c.position_x BETWEEN -6700 AND -4000 AND c.position_y BETWEEN -1400 AND 1600)
    OR
    -- Tirisfal Glades (Horde Undead starter) map=0
    (c.map = 0 AND c.position_x BETWEEN 1500 AND 3700 AND c.position_y BETWEEN -700 AND 1800)
    OR
    -- Mulgore (Horde Tauren starter) map=1
    (c.map = 1 AND c.position_x BETWEEN -3000 AND -500 AND c.position_y BETWEEN -2400 AND 800)
    OR
    -- Durotar (Horde Orc/Troll starter) map=1
    (c.map = 1 AND c.position_x BETWEEN -300 AND 2500 AND c.position_y BETWEEN -5600 AND -3000)
    OR
    -- Teldrassil (Alliance Night Elf starter) map=1
    (c.map = 1 AND c.position_x BETWEEN 9200 AND 10900 AND c.position_y BETWEEN 1000 AND 2900)
    OR
    -- Azuremyst Isle (Alliance Draenei starter) map=530
    (c.map = 530 AND c.position_x BETWEEN -5100 AND -3200 AND c.position_y BETWEEN -13600 AND -11400)
    OR
    -- Eversong Woods (Horde Blood Elf starter) map=530
    (c.map = 530 AND c.position_x BETWEEN 8300 AND 10300 AND c.position_y BETWEEN -8000 AND -5400)
  );

-- Apply the boost.
UPDATE acore_world.creature c
JOIN acore_world.creature_template ct ON c.id1 = ct.entry
SET c.spawntimesecs = 30
WHERE ct.minlevel <= 10
  AND ct.npcflag = 0
  AND ct.rank = 0
  AND c.spawntimesecs > 60
  AND (
    -- Elwynn Forest (Alliance Human starter) map=0
    (c.map = 0 AND c.position_x BETWEEN -10800 AND -8300 AND c.position_y BETWEEN -700 AND 800)
    OR
    -- Dun Morogh (Alliance Dwarf/Gnome starter) map=0
    (c.map = 0 AND c.position_x BETWEEN -6700 AND -4000 AND c.position_y BETWEEN -1400 AND 1600)
    OR
    -- Tirisfal Glades (Horde Undead starter) map=0
    (c.map = 0 AND c.position_x BETWEEN 1500 AND 3700 AND c.position_y BETWEEN -700 AND 1800)
    OR
    -- Mulgore (Horde Tauren starter) map=1
    (c.map = 1 AND c.position_x BETWEEN -3000 AND -500 AND c.position_y BETWEEN -2400 AND 800)
    OR
    -- Durotar (Horde Orc/Troll starter) map=1
    (c.map = 1 AND c.position_x BETWEEN -300 AND 2500 AND c.position_y BETWEEN -5600 AND -3000)
    OR
    -- Teldrassil (Alliance Night Elf starter) map=1
    (c.map = 1 AND c.position_x BETWEEN 9200 AND 10900 AND c.position_y BETWEEN 1000 AND 2900)
    OR
    -- Azuremyst Isle (Alliance Draenei starter) map=530
    (c.map = 530 AND c.position_x BETWEEN -5100 AND -3200 AND c.position_y BETWEEN -13600 AND -11400)
    OR
    -- Eversong Woods (Horde Blood Elf starter) map=530
    (c.map = 530 AND c.position_x BETWEEN 8300 AND 10300 AND c.position_y BETWEEN -8000 AND -5400)
  );
