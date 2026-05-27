--
-- mod-bracket-sets — Bracket 1 (level 25-34) bonus seed
--
-- 54 rows: 9 classes × 3 specs × 2 thresholds (2pc, 4pc).
--
-- DESIGN NOTE (2026-05-13): Bracket 1 uses ONE itemset (90101) that covers all
-- wearable armor types — Cloth, Leather, Mail, Plate, Misc (rings/necks/
-- cloaks/trinkets), and Buckler. Reason: our Phase 1.0 SoD import shipped
-- 0 plate items (BFD plate drops imported as mail due to a classification
-- bug in sod-import.py), so a 4-itemset Plate/Mail/Leather/Cloth design
-- would leave Warrior/Paladin with no earnable bonuses. The single-itemset
-- model lets every class earn their bonuses by wearing any 2-4 Bracket 1
-- pieces, regardless of armor type.
--
-- DESIGN NOTE (2026-05-13 catalog refit): Original-catalog bonuses targeted
-- abilities that don't unlock until L40-60+ (Mortal Strike L40, Mangle L50,
-- Crusader Strike L50, Stormstrike L40, Bestial Wrath L40, Vampiric Touch
-- L40+, Lifebloom L70, Adrenaline Rush L60, Mutilate L60, Shadowstep L60,
-- Lava Burst L60). At Bracket 1 (L25-34) those triggers can't fire, so the
-- catalog was refitted to L1-25 abilities while preserving spec identity.
-- Full refit proposal: docs/superpowers/specs/2026-05-13-phase-1-5-bracket-1-bonus-catalog-refit.md
--
-- For future brackets, the itemset ID encodes the bracket window:
--   90101 = Bracket 1 (levels 25-34) — this file
--   90201 = Bracket 2 (levels 35-44)
--   90301 = Bracket 3, etc.
--
-- Marker spell IDs are existing WotLK retail T7-T10 tier-bonus auras whose
-- behavior is fully overridden by mod-bracket-sets' AuraScript registrations
-- (in BracketSetsBonusEffects.cpp). All 54 are bound to
-- spell_bracketsets_placeholder in spell_script_names by SQL pack 04 until
-- per-bonus implementations are authored.
--
-- Curated from Spell.dbc via tools/spell_dbc_extract.py + cross-checked
-- against src/server/scripts/Spells/spell_*.cpp for binding conflicts on
-- 2026-05-13. Conflicted IDs avoided: 64736, 64869, 64890, 64895, 64928,
-- 67151, 67191, 67201, 67228, 70650, 70656, 70718, 70723, 70726, 70752,
-- 70770, 70803, 70805, 70811, 70817, 70832, 70847.
--
-- See:
--   docs/superpowers/plans/2026-05-13-phase-1-5-bracket-1-tier-sets-implementation.md
--   docs/superpowers/specs/2026-05-13-phase-1-5-bracket-1-bonus-catalog-refit.md
--   kb_c50ce934 spike findings (set-bonus mechanism)
--   kb_0d42f4e5 catalog refit proposal
--

DELETE FROM `bracket_set_bonus_map` WHERE `itemset_id` BETWEEN 90101 AND 90199;

INSERT INTO `bracket_set_bonus_map`
  (`itemset_id`, `threshold`, `class_id`, `spec_id`, `spell_id`, `bracket_min`, `bracket_max`, `display_name`) VALUES

-- =========================================================================
-- Itemset 90101 — Vestiges of Blackfathom (Bracket 1, all wearable armor)
-- =========================================================================

-- Warrior (class 1) — Arms (1) / Fury (2) / Protection (3)
(90101, 2, 1, 1, 64938, 25, 34, 'Sundered Resolve'),               -- Rend damage +20%
(90101, 4, 1, 1, 64939, 25, 34, 'Overpowering Strike'),            -- After crit, next Overpower rage refunded (8s window)
(90101, 2, 1, 2, 67234, 25, 34, "Berserker's Cadence"),            -- Cleave damage +15%
(90101, 4, 1, 2, 67268, 25, 34, 'Wrathful Strike'),                -- Heroic Strike 15% chance no rage
(90101, 2, 1, 3, 64933, 25, 34, 'Bulwark Surge'),                  -- Shield Block grants +5% block value (6s)
(90101, 4, 1, 3, 64936, 25, 34, 'Devastating Cycle'),              -- After Revenge, Thunder Clap -50% rage (5s window)

-- Paladin (class 2) — Holy (1) / Protection (2) / Retribution (3)
(90101, 2, 2, 1, 70755, 25, 34, "Light's Compact"),                -- Flash of Light 10% chance instant cast
(90101, 4, 2, 1, 70756, 25, 34, 'Mending Spark'),                  -- Holy Light 25% chance to apply small HoT
(90101, 2, 2, 2, 64881, 25, 34, 'Righteous Provocation'),          -- Hand of Reckoning +50% threat
(90101, 4, 2, 2, 64882, 25, 34, 'Sanctified Ground'),              -- Consecration tick damage +20% (V1; was BoS DR, redesigned 2026-05-14)
(90101, 2, 2, 3, 64878, 25, 34, "Crusader's Fervor"),              -- Seal of Righteousness +20% damage
(90101, 4, 2, 3, 64879, 25, 34, 'Verdict of Light'),               -- Judgement +15% damage + 2% mana refund on crit

-- Hunter (class 3) — Beast Mastery (1) / Marksmanship (2) / Survival (3)
(90101, 2, 3, 1, 64854, 25, 34, 'Tide-Born Beast'),                -- Pet damage +5%
(90101, 4, 3, 1, 64860, 25, 34, 'Primal Frenzy'),                  -- Pet OOC HP regen +50%
(90101, 2, 3, 2, 67150, 25, 34, 'Piercing Shot'),                  -- Arcane Shot reduces target armor 5% (8s)
(90101, 4, 3, 2, 70724, 25, 34, 'Arcane Cadence'),                 -- Auto Shot crit -> next Arcane Shot guaranteed crit (8s); marker borrowed from Druid T10 Feral 2P spell
(90101, 2, 3, 3, 70727, 25, 34, 'Venomous Tide'),                  -- Serpent Sting +15% damage
(90101, 4, 3, 3, 70730, 25, 34, 'Detonation'),                     -- Immolation Trap initial damage +25%

-- Rogue (class 4) — Assassination (1) / Combat (2) / Subtlety (3)
(90101, 2, 4, 1, 64914, 25, 34, 'Vile Toxins'),                    -- Garrote damage +20%
(90101, 4, 4, 1, 64915, 25, 34, "Envenom's Wake"),                 -- Rupture 10%/tick chance to refund 1 energy
(90101, 2, 4, 2, 67209, 25, 34, 'Sinister Edge'),                  -- Sinister Strike 10% chance +1 energy
(90101, 4, 4, 2, 67211, 25, 34, "Adrenaline's Edge"),              -- Eviscerate +10% per combo point above 3
(90101, 2, 4, 3, 67186, 25, 34, "Backstab's Bite"),                -- Backstab +10% damage when behind target; marker borrowed from Paladin T9 Tank 2P spell
(90101, 4, 4, 3, 67187, 25, 34, 'Stepping Shadow'),                -- Vanish cooldown -30s; marker borrowed from Paladin T9 Tank 4P spell

-- Priest (class 5) — Discipline (1) / Holy (2) / Shadow (3)
(90101, 2, 5, 1, 67202, 25, 34, 'Reinforced Shield'),              -- Power Word: Shield absorb +10%
(90101, 4, 5, 1, 70798, 25, 34, "Suppression's Echo"),             -- After PW:Shield, target healing received +5% (8s)
(90101, 2, 5, 2, 64910, 25, 34, 'Renewed Light'),                  -- Renew ticks 15% chance to be doubled
(90101, 4, 5, 2, 64912, 25, 34, 'Fire of the Fold'),               -- Holy Fire periodic damage +25%
(90101, 2, 5, 3, 67193, 25, 34, 'Mind Crack'),                     -- Mind Blast reduces target shadow resistance 10 (8s)
(90101, 4, 5, 3, 67198, 25, 34, "Vampire's Renewal"),              -- SW:Pain periodic ticks heal you 1% maximum HP

-- Shaman (class 7) — Elemental (1) / Enhancement (2) / Restoration (3)
(90101, 2, 7, 1, 67227, 25, 34, "Stormcaller's Insight"),          -- Lightning Bolt crits grant +2% haste (6s)
(90101, 4, 7, 1, 64925, 25, 34, "Lightning's Reach"),              -- Lightning Bolt damage +5%, range +5 yards
(90101, 2, 7, 2, 67220, 25, 34, "Stormstrike's Echo"),             -- Lightning Shield damage +15%
(90101, 4, 7, 2, 67221, 25, 34, 'Searing Lash'),                   -- Searing Totem damage +25%
(90101, 2, 7, 3, 67225, 25, 34, 'Cascading Tide'),                 -- Healing Wave mana cost -10%
(90101, 4, 7, 3, 67226, 25, 34, 'Quick Mend'),                     -- After LHW, next HW cast time -0.5s (5s window)

-- Mage (class 8) — Arcane (1) / Fire (2) / Frost (3)
(90101, 2, 8, 1, 64867, 25, 34, 'Final Missile'),                  -- Arcane Missiles last tick +30% crit chance
(90101, 4, 8, 1, 67188, 25, 34, 'Sustained Power'),                -- Arcane Explosion -> next Arcane Missiles +10% damage (8s window); marker borrowed from Paladin T9 Ret 2P spell
(90101, 2, 8, 2, 67164, 25, 34, 'Incandescent Burn'),              -- Fireball/Ignite periodic +25%
(90101, 4, 8, 2, 67185, 25, 34, 'Pyroblast Surge'),                -- Fire Blast cooldown -1s
(90101, 2, 8, 3, 67189, 25, 34, 'Frozen Tide'),                    -- Frost Nova freeze duration +1s; marker borrowed from Paladin T9 Ret 4P spell
(90101, 4, 8, 3, 70748, 25, 34, 'Glacial Lance'),                  -- Frostbolt vs frozen +30% damage

-- Warlock (class 9) — Affliction (1) / Demonology (2) / Destruction (3)
(90101, 2, 9, 1, 64931, 25, 34, 'Corrupted Flesh'),                -- Corruption damage +15%
(90101, 4, 9, 1, 67231, 25, 34, 'Soul Drain'),                     -- Drain Soul kill refunds 30% maximum mana
(90101, 2, 9, 2, 64932, 25, 34, 'Demonic Augmentation'),           -- Pet damage +10%
(90101, 4, 9, 2, 67230, 25, 34, 'Soulbound'),                      -- Health Funnel transfers +25%
(90101, 2, 9, 3, 70839, 25, 34, 'Shadow Cadence'),                 -- After Shadow Bolt crit, next SB cast time -0.5s (6s window)
(90101, 4, 9, 3, 70841, 25, 34, 'Lingering Flame'),                -- Immolate periodic duration +6s

-- Druid (class 11) — Balance (1) / Feral (2) / Restoration (3)
(90101, 2, 11, 1, 67125, 25, 34, 'Lunar Cycle'),                   -- Moonfire periodic +15% crit chance
(90101, 4, 11, 1, 67126, 25, 34, 'Solar Cadence'),                 -- Wrath 10% chance to grant Nature's Grace (next spell instant, 8s)
(90101, 2, 11, 2, 67121, 25, 34, 'Primal Economy'),                -- Claw -5 energy, Maul -5 rage
(90101, 4, 11, 2, 67123, 25, 34, "Beast's Refund"),                -- Maul 15% chance to refund full rage cost
(90101, 2, 11, 3, 67127, 25, 34, 'Rejuvenated'),                   -- Rejuvenation first periodic tick doubled
(90101, 4, 11, 3, 67128, 25, 34, 'Wake of Bloom');                 -- Regrowth bloom 25% chance to splash 2 nearby allies (8 yard) for 30%

-- Verification: should return 54
-- SELECT COUNT(*) FROM bracket_set_bonus_map WHERE itemset_id = 90101;
