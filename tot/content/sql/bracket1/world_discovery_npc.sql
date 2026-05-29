-- sql/bracket1/world_discovery_npc.sql
-- Discovery NPC creature template + 8 capital city spawns.
-- Gossip menu handled by lua/discovery-npc-gossip.lua at runtime.
--
-- Schema notes vs plan template:
--   - modelid1 lives in creature_template_model (Idx=0), not creature_template itself.
--   - creature_template has no 'scale' or 'mechanic_immune_mask' columns in this build.
--   - creature spawn rows have no 'modelid' column; display is driven by creature_template_model.
--   - Non-combat flag uses flags_extra=2 (CREATURE_FLAG_EXTRA_NO_AGGRO_RANGE) + unit_flags=768.

-- -----------------------------------------------------------------------
-- 1. Creature template
-- -----------------------------------------------------------------------
INSERT IGNORE INTO tot_world.creature_template
    (entry, name, subname, gossip_menu_id,
     minlevel, maxlevel, faction,
     npcflag, speed_walk, speed_run,
     `rank`, dmgschool, DamageModifier,
     BaseAttackTime, RangeAttackTime,
     BaseVariance, RangeVariance,
     unit_class, unit_flags, unit_flags2, dynamicflags,
     family, type, type_flags,
     lootid, pickpocketloot, skinloot,
     PetSpellDataId, VehicleId,
     mingold, maxgold,
     AIName, MovementType, HoverHeight,
     HealthModifier, ManaModifier, ArmorModifier, ExperienceModifier,
     RacialLeader, RegenHealth,
     CreatureImmunitiesId, flags_extra, ScriptName)
VALUES
    (90100, 'Master of Discovery', 'Rune Engraver', 0,
     25, 25, 35,
     1, 1.0, 1.14286,
     0, 0, 1.0,
     2000, 0,
     1.0, 1.0,
     1, 768, 0, 0,
     0, 7, 0,
     0, 0, 0,
     0, 0,
     0, 0,
     '', 0, 1.0,
     1.0, 1.0, 1.0, 1.0,
     0, 1,
     0, 2, '');

-- -----------------------------------------------------------------------
-- 2. Model assignment (modelid1 = 15351 via creature_template_model)
-- -----------------------------------------------------------------------
INSERT IGNORE INTO tot_world.creature_template_model
    (CreatureID, Idx, CreatureDisplayID, DisplayScale, Probability)
VALUES
    (90100, 0, 15351, 1.0, 1.0);

-- -----------------------------------------------------------------------
-- 3. Spawns: 8 capitals (guid range 9010000-9010007, high custom range)
-- -----------------------------------------------------------------------
INSERT IGNORE INTO tot_world.creature
    (guid, id1, id2, id3, map, zoneId, areaId, spawnMask, phaseMask,
     equipment_id, position_x, position_y, position_z, orientation,
     spawntimesecs, wander_distance, currentwaypoint, curhealth, curmana,
     MovementType, npcflag, unit_flags, dynamicflags, ScriptName)
VALUES
    -- Stormwind (Alliance)
    (9010000, 90100, 0, 0, 0, 1519, 0, 1, 1,
     0, -8830.0, 625.0, 94.0, 5.5,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Ironforge (Alliance)
    (9010001, 90100, 0, 0, 0, 1537, 0, 1, 1,
     0, -4925.0, -961.0, 502.0, 4.7,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Darnassus (Alliance)
    (9010002, 90100, 0, 0, 1, 1657, 0, 1, 1,
     0, 9947.0, 2483.0, 1316.0, 0.0,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Exodar (Alliance)
    (9010003, 90100, 0, 0, 530, 3557, 0, 1, 1,
     0, -3961.0, -11899.0, -1.4, 5.0,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Orgrimmar (Horde)
    (9010004, 90100, 0, 0, 1, 1637, 0, 1, 1,
     0, 1633.0, -4438.0, 16.0, 5.5,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Thunder Bluff (Horde)
    (9010005, 90100, 0, 0, 1, 1638, 0, 1, 1,
     0, -1297.0, 134.0, 132.0, 5.0,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Undercity (Horde)
    (9010006, 90100, 0, 0, 0, 1497, 0, 1, 1,
     0, 1633.0, 240.0, -43.0, 6.0,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, ''),
    -- Silvermoon (Horde)
    (9010007, 90100, 0, 0, 530, 3487, 0, 1, 1,
     0, 9462.0, -7369.0, 14.0, 6.0,
     300, 0, 0, 100, 0,
     0, 1, 0, 0, '');
