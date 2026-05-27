--
-- Phase 1.1 Step 3 — Blackfathom Deeps raid creature wiring
--
-- Two changes:
-- 1. UPDATE the 6 vanilla BFD bosses to (a) bind to our stub raid scripts
--    (see patches/ac-bfd-raid/overlays/.../boss_*.cpp) and (b) bump their
--    HealthModifier so they survive a 10-player burn (5-man tuned values
--    were 5-10x base; raid is ~25x).
-- 2. INSERT a new creature_template for Lorgus Jett (entry 207356, matches
--    SoD ID) since he's not in vanilla BFD. Spawn him at a logical spot
--    in the Twilight Pillar area between Gelihast and Lord Kelris.
--
-- DamageModifier left at vanilla 1.7x for V1 -- mod-autobalance still
-- runs on top and will scale further at run time. Step 4-7's mechanics
-- patches will tune per-boss when the mechanics actually fire.
--
-- For V1 we treat Old Serra'kis (entry 4830) as the Baron Aquanis script
-- placeholder. The vanilla creature stays named "Old Serra'kis" in DB
-- but its AI runs the boss_baron_aquanis_raid stub. Real Baron Aquanis
-- as a separate creature is a follow-up if/when we want both.
--

-- ============================================================================
-- Vanilla boss raid binding + HP bump
-- ============================================================================
UPDATE `creature_template`
SET `ScriptName` = 'boss_ghamoo_ra_raid', `HealthModifier` = 25.0, `DamageModifier` = 2.0
WHERE `entry` = 4887;

UPDATE `creature_template`
SET `ScriptName` = 'boss_lady_sarevess_raid', `HealthModifier` = 25.0, `DamageModifier` = 2.0
WHERE `entry` = 4831;

UPDATE `creature_template`
SET `ScriptName` = 'boss_baron_aquanis_raid', `HealthModifier` = 25.0, `DamageModifier` = 2.0
WHERE `entry` = 4830;  -- vanilla "Old Serra'kis" doubles as Baron Aquanis script slot for V1

UPDATE `creature_template`
SET `ScriptName` = 'boss_gelihast_raid', `HealthModifier` = 25.0, `DamageModifier` = 2.0
WHERE `entry` = 6243;

UPDATE `creature_template`
SET `ScriptName` = 'boss_twilight_lord_kelris_raid', `HealthModifier` = 25.0, `DamageModifier` = 2.0
WHERE `entry` = 4832;

UPDATE `creature_template`
SET `ScriptName` = 'boss_akumai_raid', `HealthModifier` = 30.0, `DamageModifier` = 2.0
WHERE `entry` = 4829;  -- final boss, slightly larger HP pool

-- ============================================================================
-- Lorgus Jett — new creature, not in vanilla BFD
-- ============================================================================
-- Entry 207356 = SoD ID per kb_4dbe8db8 research. ScriptName binds to our
-- step-2 stub; mechanics ship in step 4. Stats mirror Twilight Lord
-- Kelris (he's a Twilight Cult shaman, similar power level).
--
-- Faction 14 (HOSTILE) so any player aggros him on sight.
-- DamageSchool 0 (Normal); raid takes Twilight Cult flavor damage from
-- his totems rather than from his melee.

DELETE FROM `creature_template` WHERE `entry` = 207356;
INSERT INTO `creature_template` (`entry`, `name`, `subname`,
    `minlevel`, `maxlevel`, `faction`,
    `npcflag`, `speed_walk`, `speed_run`,
    `rank`, `dmgschool`, `DamageModifier`, `BaseAttackTime`, `RangeAttackTime`,
    `BaseVariance`, `RangeVariance`, `unit_class`,
    `unit_flags`, `dynamicflags`, `family`, `type`, `type_flags`,
    `AIName`, `MovementType`, `HealthModifier`, `ManaModifier`,
    `ArmorModifier`, `ExperienceModifier`, `RegenHealth`, `RacialLeader`,
    `ScriptName`, `VerifiedBuild`)
VALUES (207356, "Lorgus Jett", "Twilight Cult Shaman",
    25, 25, 14,
    0, 1, 1.14286,
    1, 0, 2.0, 2000, 2000,
    1, 1, 1,
    32768, 0, 0, 7, 4136,
    '', 0, 25.0, 1,
    1, 1, 1, 0,
    'boss_lorgus_jett_raid', 0);

DELETE FROM `creature_template_model` WHERE `CreatureID` = 207356;
INSERT INTO `creature_template_model` (`CreatureID`, `Idx`, `CreatureDisplayID`, `DisplayScale`, `Probability`)
VALUES (207356, 0, 4939, 1.0, 1.0);  -- Reuse Kelris's Twilight cultist model

-- ============================================================================
-- Spawn Lorgus Jett in BFD
-- ============================================================================
-- Position picked between Gelihast (-412, 40) and Kelris (-818, -155),
-- roughly halfway up the main path. spawntimesecs 86400 = 24h respawn
-- (raid bosses don't fast-respawn).

DELETE FROM `creature` WHERE `id1` = 207356;
INSERT INTO `creature` (`id1`, `map`, `zoneId`, `areaId`, `spawnMask`, `phaseMask`,
    `equipment_id`, `position_x`, `position_y`, `position_z`, `orientation`,
    `spawntimesecs`, `wander_distance`, `currentwaypoint`, `curhealth`, `curmana`,
    `MovementType`, `npcflag`, `unit_flags`, `dynamicflags`)
VALUES (207356, 48, 0, 0, 1, 1,
    0, -615.0, -55.0, -50.0, 0.0,
    86400, 0, 0, 1000000, 0,
    0, 0, 0, 0);
