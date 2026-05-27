--
-- Phase 1.1 Step 4 (V1, Heimdal-original) — SmartAI for 4 BFD raid bosses
--
-- V1 redesign: drop the SoD-port mechanics and ship original WotLK-style
-- mechanics, one identifying twist per boss on top of the standard
-- ability rotation. The 3 C++ bosses (Gelihast, Kelris, Aku'mai) are
-- redesigned in their .cpp files; the 4 here are SmartAI-only:
--
--   Ghamoo-ra (4887)            -- Patchwerk turtle: Hateful Strike on
--                                  random non-top + Sunder stacking
--                                  debuff drives tank-swaps + Cleave.
--   Lady Sarevess (4831)        -- Interruptible Frost Arrow (raid must
--                                  kick) + Forked Lightning chain
--                                  damage + add waves at 60/30% HP.
--   Old Serra'kis (4830, used   -- Frenzy shark: at 30% HP gain
--   as Baron Aquanis slot)        attack-speed + damage buff for the
--                                  last-stretch DPS race.
--   Lorgus Jett (207356)        -- Twilight shaman: Lightning Bolt
--                                  nuke + self Lightning Shield aura
--                                  + Corrupted Totem every 20s
--                                  (priority kill target).
--
-- SmartAI cheat sheet:
--   event_type 0  = SMART_EVENT_UPDATE_IC (in-combat cooldown)
--                   params (init_min, init_max, repeat_min, repeat_max) ms
--   event_type 2  = SMART_EVENT_HEALTH_PCT
--                   params (hp_min%, hp_max%, repeat_min, repeat_max)
--   action_type 11 = SMART_ACTION_CAST (param1=spell, param2=cast flags)
--   action_type 12 = SMART_ACTION_SUMMON_CREATURE
--   target_type 1  = SMART_TARGET_SELF
--   target_type 2  = SMART_TARGET_VICTIM
--   target_type 5  = SMART_TARGET_HOSTILE_RANDOM (all players in threat)
--   target_type 6  = SMART_TARGET_HOSTILE_RANDOM_NOT_TOP (not tank)
--

-- ============================================================================
-- 1. Ghamoo-ra (4887): Sunder + Hateful Strike + Cleave
-- ============================================================================
-- "Hateful Strike" is Mortal Strike R3 (12294) targeting a random non-tank.
-- It's not a true "highest-HP melee" pick (SmartAI can't express that
-- cleanly) -- close enough for V1.
DELETE FROM `smart_scripts` WHERE `entryorguid` = 4887 AND `source_type` = 0;
INSERT INTO `smart_scripts` (`entryorguid`, `source_type`, `id`, `link`,
    `event_type`, `event_phase_mask`, `event_chance`, `event_flags`,
    `event_param1`, `event_param2`, `event_param3`, `event_param4`,
    `action_type`, `action_param1`, `action_param2`, `target_type`, `comment`)
VALUES
    (4887, 0, 0, 0, 0, 0, 100, 0,  5000,  5000, 10000, 14000, 11,  7405, 0, 2,
        'Ghamoo-ra raid - Crushing Bite (Sunder Armor R3) stacks on tank'),
    (4887, 0, 1, 0, 0, 0, 100, 0, 12000, 12000, 12000, 16000, 11, 12294, 0, 6,
        'Ghamoo-ra raid - Hateful Strike (Mortal Strike R3) on random non-tank'),
    (4887, 0, 2, 0, 0, 0, 100, 0,  8000,  8000,  8000, 12000, 11,   845, 0, 2,
        'Ghamoo-ra raid - Cleave R1 on victim (frontal AoE; face away from raid)');

-- ============================================================================
-- 2. Lady Sarevess (4831): interruptible Frost Arrow + chain + adds
-- ============================================================================
-- Sea Witch adds: vanilla creature 4805 (Blackfathom Sea Witch), already
-- spawns as trash in BFD. Two adds per wave at 60% and 30% HP.
DELETE FROM `smart_scripts` WHERE `entryorguid` = 4831 AND `source_type` = 0;
INSERT INTO `smart_scripts` (`entryorguid`, `source_type`, `id`, `link`,
    `event_type`, `event_phase_mask`, `event_chance`, `event_flags`,
    `event_param1`, `event_param2`, `event_param3`, `event_param4`,
    `action_type`, `action_param1`, `action_param2`, `target_type`, `comment`)
VALUES
    (4831, 0, 0, 0, 0, 0, 100, 0,  4000,  4000,  7000, 10000, 11, 8407, 0, 2,
        'Lady Sarevess raid - Frost Arrow (Frostbolt R4, interruptible)'),
    (4831, 0, 1, 0, 0, 0, 100, 0,  9000,  9000, 11000, 14000, 11,  939, 0, 2,
        'Lady Sarevess raid - Forked Lightning (Chain Lightning R2)'),
    (4831, 0, 2, 0, 2, 0, 100, 0,    59,    61,     0,     0, 12, 4805, 8, 1,
        'Lady Sarevess raid - At 60% HP: summon Blackfathom Sea Witch (wave 1)'),
    (4831, 0, 3, 0, 2, 0, 100, 0,    59,    61,     0,     0, 12, 4805, 8, 1,
        'Lady Sarevess raid - At 60% HP: summon Blackfathom Sea Witch (wave 1)'),
    (4831, 0, 4, 0, 2, 0, 100, 0,    29,    31,     0,     0, 12, 4805, 8, 1,
        'Lady Sarevess raid - At 30% HP: summon Blackfathom Sea Witch (wave 2)'),
    (4831, 0, 5, 0, 2, 0, 100, 0,    29,    31,     0,     0, 12, 4805, 8, 1,
        'Lady Sarevess raid - At 30% HP: summon Blackfathom Sea Witch (wave 2)');

-- ============================================================================
-- 3. Old Serra'kis (4830, used as Baron Aquanis slot): Frenzy at 30%
-- ============================================================================
-- Thrash (3391) gives the boss extra melee attacks. Frenzy (8269) gives
-- +30% melee haste and acts as the soft-enrage at 30% HP.
DELETE FROM `smart_scripts` WHERE `entryorguid` = 4830 AND `source_type` = 0;
INSERT INTO `smart_scripts` (`entryorguid`, `source_type`, `id`, `link`,
    `event_type`, `event_phase_mask`, `event_chance`, `event_flags`,
    `event_param1`, `event_param2`, `event_param3`, `event_param4`,
    `action_type`, `action_param1`, `action_param2`, `target_type`, `comment`)
VALUES
    (4830, 0, 0, 0, 0, 0, 100, 0,  6000,  6000, 10000, 14000, 11, 3391, 0, 1,
        'Old Serra''kis raid - Thrash (extra melee attacks)'),
    (4830, 0, 1, 0, 2, 0, 100, 0,    29,    31,     0,     0, 11, 8269, 0, 1,
        'Old Serra''kis raid - At 30% HP: Blood Frenzy self-buff (soft enrage)');

-- ============================================================================
-- 4. Lorgus Jett (207356): nuke + self shield + corrupted totem
-- ============================================================================
-- Lightning Shield (324) reflects a small amount of damage taken back at
-- attackers. In retail bots would pause DPS while it's up; ours probably
-- won't, so the reflect is mostly flavor in V1. The Corrupted Totem
-- (Searing Totem 5929) is the priority-kill mechanic -- it's a low-HP
-- mob that the raid should burn before it stacks too much damage.
DELETE FROM `smart_scripts` WHERE `entryorguid` = 207356 AND `source_type` = 0;
INSERT INTO `smart_scripts` (`entryorguid`, `source_type`, `id`, `link`,
    `event_type`, `event_phase_mask`, `event_chance`, `event_flags`,
    `event_param1`, `event_param2`, `event_param3`, `event_param4`,
    `action_type`, `action_param1`, `action_param2`, `target_type`, `comment`)
VALUES
    (207356, 0, 0, 0, 0, 0, 100, 0,  3000,  3000,  3500,  5000, 11,  529, 0, 2,
        'Lorgus Jett raid - Lightning Bolt R3 on victim'),
    (207356, 0, 1, 0, 0, 0, 100, 0,  1000,  1000, 60000, 60000, 11,  324, 0, 1,
        'Lorgus Jett raid - Self Lightning Shield (reflect; up almost always)'),
    (207356, 0, 2, 0, 0, 0, 100, 0, 20000, 20000, 25000, 30000, 12, 5929, 8, 1,
        'Lorgus Jett raid - Summon Corrupted Totem (Searing Totem; priority kill)');

-- ============================================================================
-- Bind: SmartAI on, ScriptName off for these 4 (C++ bosses keep ScriptName)
-- ============================================================================
UPDATE `creature_template`
SET `AIName` = 'SmartAI', `ScriptName` = ''
WHERE `entry` IN (4887, 4831, 4830, 207356);
