--
-- mod-bracket-sets — Batch 1: passive damage modifier bindings
--
-- Binds 3 bonus implementation scripts to their TARGET spell IDs (the spell
-- whose damage we want to modify), NOT to our marker spell IDs. AC's
-- spell_script_names supports multiple bindings per spell ID, so our scripts
-- coexist with the engine's default per-spell logic (e.g., spell_warr_rend).
--
-- The 3 bonuses bound here:
--   spell_bracketsets_warrior_arms_2pc_rend          - Sundered Resolve
--   spell_bracketsets_warlock_aff_2pc_corruption     - Corrupted Flesh
--   spell_bracketsets_priest_holy_4pc_holy_fire      - Fire of the Fold
--
-- Each script's CalculateBonus hook checks whether the caster has the
-- corresponding bracket-set marker aura and, if so, scales the target
-- spell's periodic damage by the bonus percent.
--
-- Markers (still bound to placeholder for chat notification) are unchanged:
--   64938 Warrior Arms 2pc      -> placeholder + (target Rend) Sundered Resolve
--   64931 Warlock Aff 2pc       -> placeholder + (target Corruption) Corrupted Flesh
--   64912 Priest Holy 4pc       -> placeholder + (target Holy Fire) Fire of the Fold
--
-- To add another bonus in this pattern: pick the target spell's ranks
-- (Wowhead is the easiest source) and add rows here pointing to your new
-- script class.
--

-- Idempotent: remove any prior bindings for these script names so re-runs
-- are safe.
DELETE FROM `spell_script_names` WHERE `ScriptName` IN (
    -- Batch 1
    'spell_bracketsets_warrior_arms_2pc_rend',
    'spell_bracketsets_warlock_aff_2pc_corruption',
    'spell_bracketsets_priest_holy_4pc_holy_fire',
    -- Batch 2
    'spell_bracketsets_hunter_surv_2pc_serpent_sting',
    'spell_bracketsets_rogue_assn_2pc_garrote',
    'spell_bracketsets_mage_fire_2pc_ignite',
    'spell_bracketsets_warrior_fury_2pc_cleave',
    -- Batch 3
    'spell_bracketsets_shaman_ele_4pc_lightning_bolt',
    'spell_bracketsets_hunter_surv_4pc_immolation_trap',
    'spell_bracketsets_priest_disc_2pc_power_word_shield',
    'spell_bracketsets_druid_resto_2pc_rejuvenation',
    'spell_bracketsets_mage_frost_4pc_frostbolt',
    -- Batch 4
    'spell_bracketsets_mage_fire_4pc_fire_blast',
    'spell_bracketsets_rogue_sub_4pc_vanish',
    'spell_bracketsets_warlock_destro_4pc_immolate',
    'spell_bracketsets_mage_frost_2pc_frost_nova',
    -- Batch 5
    'spell_bracketsets_rogue_combat_2pc_sinister_strike',
    'spell_bracketsets_hunter_bm_2pc_pet_damage',
    'spell_bracketsets_warlock_demo_2pc_pet_damage',
    -- Batch 6
    'spell_bracketsets_druid_feral_2pc_claw',
    'spell_bracketsets_druid_feral_2pc_maul',
    'spell_bracketsets_shaman_resto_2pc_healing_wave',
    'spell_bracketsets_paladin_ret_4pc_judgement',
    -- Batch 7
    'spell_bracketsets_warrior_fury_4pc_heroic_strike',
    'spell_bracketsets_druid_feral_4pc_maul',
    'spell_bracketsets_rogue_sub_2pc_backstab',
    'spell_bracketsets_hunter_mm_2pc_arcane_shot',
    'spell_bracketsets_priest_shadow_2pc_mind_blast',
    -- Batch 8
    'spell_bracketsets_hunter_bm_4pc_pet_hp',
    'spell_bracketsets_paladin_prot_2pc_hand_of_reckoning',
    'spell_bracketsets_paladin_prot_4pc_blessing_of_sanctuary',  -- legacy name, kept so re-runs purge prior rows
    'spell_bracketsets_paladin_prot_4pc_consecration',
    'spell_bracketsets_warrior_prot_2pc_shield_block',
    'spell_bracketsets_mage_arc_2pc_arcane_missile',
    -- Batch 9
    'spell_bracketsets_druid_balance_2pc_moonfire',
    'spell_bracketsets_priest_holy_2pc_renew',
    'spell_bracketsets_priest_shadow_4pc_sw_pain',
    'spell_bracketsets_rogue_assn_4pc_rupture',
    'spell_bracketsets_warlock_demo_4pc_health_funnel',
    -- Batch 10
    'spell_bracketsets_warrior_prot_4pc_revenge',
    'spell_bracketsets_warrior_prot_4pc_thunder_clap',
    'spell_bracketsets_mage_arc_4pc_arcane_explosion',
    'spell_bracketsets_mage_arc_4pc_arc_missile_bonus',
    'spell_bracketsets_shaman_resto_4pc_lhw',
    'spell_bracketsets_shaman_resto_4pc_hw',
    'spell_bracketsets_paladin_holy_2pc_flash_of_light',
    'spell_bracketsets_druid_balance_4pc_wrath',
    -- Batch 11
    'spell_bracketsets_warrior_arms_4pc_overpower',
    'spell_bracketsets_hunter_mm_4pc_multi_shot',
    'spell_bracketsets_priest_disc_4pc_pw_shield_duration',
    'spell_bracketsets_shaman_ele_2pc_lightning_bolt',
    'spell_bracketsets_warlock_destro_2pc_shadow_bolt',
    'spell_bracketsets_druid_resto_4pc_regrowth',
    -- Batch 12 (final)
    'spell_bracketsets_paladin_holy_4pc_holy_light',
    'spell_bracketsets_paladin_ret_2pc_sor',
    'spell_bracketsets_shaman_enh_2pc_lightning_shield',
    'spell_bracketsets_shaman_enh_4pc_searing_totem',
    'spell_bracketsets_warlock_aff_4pc_drain_soul',
    'spell_bracketsets_rogue_combat_4pc_eviscerate'
);

-- ============================================================================
-- Warrior Arms 2pc — Sundered Resolve: Rend damage +20%
-- ============================================================================
-- Rend spell ranks in WotLK 3.3.5a:
--   772    rank 1 (level 4)
--   6546   rank 2 (level 10)
--   6547   rank 3 (level 18)
--   6548   rank 4 (level 26)   <- Bracket 1 wearer most likely has this
--   11572  rank 5 (level 34)   <- top end of Bracket 1
--   11573  rank 6 (level 42)
--   11574  rank 7 (level 50)
--   25208  rank 8 (level 60)
--   47465  rank 9 (level 73)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (772,   'spell_bracketsets_warrior_arms_2pc_rend'),
    (6546,  'spell_bracketsets_warrior_arms_2pc_rend'),
    (6547,  'spell_bracketsets_warrior_arms_2pc_rend'),
    (6548,  'spell_bracketsets_warrior_arms_2pc_rend'),
    (11572, 'spell_bracketsets_warrior_arms_2pc_rend'),
    (11573, 'spell_bracketsets_warrior_arms_2pc_rend'),
    (11574, 'spell_bracketsets_warrior_arms_2pc_rend'),
    (25208, 'spell_bracketsets_warrior_arms_2pc_rend'),
    (47465, 'spell_bracketsets_warrior_arms_2pc_rend');

-- ============================================================================
-- Warlock Affliction 2pc — Corrupted Flesh: Corruption damage +15%
-- ============================================================================
-- Corruption spell ranks in WotLK 3.3.5a:
--   172    rank 1 (level 4)
--   6222   rank 2 (level 12)
--   6223   rank 3 (level 20)   <- Bracket 1 lower bound
--   7648   rank 4 (level 28)   <- Bracket 1 mid
--   11671  rank 5 (level 36)
--   11672  rank 6 (level 44)
--   25311  rank 7 (level 54)
--   27216  rank 8 (level 60)
--   47812  rank 9 (level 69)
--   47813  rank 10 (level 77)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (172,   'spell_bracketsets_warlock_aff_2pc_corruption'),
    (6222,  'spell_bracketsets_warlock_aff_2pc_corruption'),
    (6223,  'spell_bracketsets_warlock_aff_2pc_corruption'),
    (7648,  'spell_bracketsets_warlock_aff_2pc_corruption'),
    (11671, 'spell_bracketsets_warlock_aff_2pc_corruption'),
    (11672, 'spell_bracketsets_warlock_aff_2pc_corruption'),
    (25311, 'spell_bracketsets_warlock_aff_2pc_corruption'),
    (27216, 'spell_bracketsets_warlock_aff_2pc_corruption'),
    (47812, 'spell_bracketsets_warlock_aff_2pc_corruption'),
    (47813, 'spell_bracketsets_warlock_aff_2pc_corruption');

-- ============================================================================
-- Priest Holy 4pc — Fire of the Fold: Holy Fire periodic damage +25%
-- ============================================================================
-- Holy Fire spell ranks in WotLK 3.3.5a:
--   14914  rank 1 (level 20)   <- Bracket 1 lower bound
--   15262  rank 2 (level 24)
--   15263  rank 3 (level 30)   <- Bracket 1 mid
--   15264  rank 4 (level 36)
--   15265  rank 5 (level 42)
--   15266  rank 6 (level 48)
--   15267  rank 7 (level 54)
--   25384  rank 8 (level 60)
--   48134  rank 9 (level 71)
--   48135  rank 10 (level 79)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (14914, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (15262, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (15263, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (15264, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (15265, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (15266, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (15267, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (25384, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (48134, 'spell_bracketsets_priest_holy_4pc_holy_fire'),
    (48135, 'spell_bracketsets_priest_holy_4pc_holy_fire');

-- ============================================================================
-- Batch 2 bindings
-- ============================================================================

-- ============================================================================
-- Hunter Survival 2pc — Venomous Tide: Serpent Sting damage +15%
-- ============================================================================
-- Serpent Sting spell ranks in WotLK 3.3.5a:
--   1978   rank 1  (level 4)
--   13549  rank 2  (level 10)
--   13550  rank 3  (level 18)
--   13551  rank 4  (level 26)   <- Bracket 1 most-likely
--   13552  rank 5  (level 34)   <- top of Bracket 1
--   13553  rank 6  (level 42)
--   13554  rank 7  (level 50)
--   13555  rank 8  (level 58)
--   25295  rank 9  (level 60)
--   27016  rank 10 (level 70)
--   49000  rank 11 (level 76)
--   49001  rank 12 (level 80)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1978,  'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13549, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13550, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13551, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13552, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13553, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13554, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (13555, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (25295, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (27016, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (49000, 'spell_bracketsets_hunter_surv_2pc_serpent_sting'),
    (49001, 'spell_bracketsets_hunter_surv_2pc_serpent_sting');

-- ============================================================================
-- Rogue Assassination 2pc — Vile Toxins: Garrote damage +20%
-- ============================================================================
-- Garrote spell ranks in WotLK 3.3.5a (stealth opener, periodic physical):
--   703    rank 1 (level 14)
--   8631   rank 2 (level 24)
--   8632   rank 3 (level 34)   <- Bracket 1 top
--   8633   rank 4 (level 44)
--   11289  rank 5 (level 54)
--   11290  rank 6 (level 60)
--   26839  rank 7 (level 70)
--   26884  rank 8 (level 77)
--   48675  rank 9 (level 80)
--   48676  rank 10 (level 80)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (703,   'spell_bracketsets_rogue_assn_2pc_garrote'),
    (8631,  'spell_bracketsets_rogue_assn_2pc_garrote'),
    (8632,  'spell_bracketsets_rogue_assn_2pc_garrote'),
    (8633,  'spell_bracketsets_rogue_assn_2pc_garrote'),
    (11289, 'spell_bracketsets_rogue_assn_2pc_garrote'),
    (11290, 'spell_bracketsets_rogue_assn_2pc_garrote'),
    (26839, 'spell_bracketsets_rogue_assn_2pc_garrote'),
    (26884, 'spell_bracketsets_rogue_assn_2pc_garrote'),
    (48675, 'spell_bracketsets_rogue_assn_2pc_garrote'),
    (48676, 'spell_bracketsets_rogue_assn_2pc_garrote');

-- ============================================================================
-- Mage Fire 2pc — Incandescent Burn: Ignite periodic damage +25%
-- ============================================================================
-- Ignite is the Fire talent. The proc spell that deals the periodic damage
-- is 12654. Only one spell ID — the talent ranks affect the percent, not
-- the periodic spell.
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (12654, 'spell_bracketsets_mage_fire_2pc_ignite');

-- ============================================================================
-- Warrior Fury 2pc — Berserker's Cadence: Cleave damage +15%
-- ============================================================================
-- Cleave spell ranks in WotLK 3.3.5a (instant weapon damage to nearby):
--   845    rank 1 (level 20)
--   7369   rank 2 (level 30)   <- Bracket 1 sweet spot
--   11608  rank 3 (level 40)
--   11609  rank 4 (level 50)
--   20569  rank 5 (level 60)
--   25231  rank 6 (level 65)
--   47519  rank 7 (level 73)
--   47520  rank 8 (level 79)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (845,   'spell_bracketsets_warrior_fury_2pc_cleave'),
    (7369,  'spell_bracketsets_warrior_fury_2pc_cleave'),
    (11608, 'spell_bracketsets_warrior_fury_2pc_cleave'),
    (11609, 'spell_bracketsets_warrior_fury_2pc_cleave'),
    (20569, 'spell_bracketsets_warrior_fury_2pc_cleave'),
    (25231, 'spell_bracketsets_warrior_fury_2pc_cleave'),
    (47519, 'spell_bracketsets_warrior_fury_2pc_cleave'),
    (47520, 'spell_bracketsets_warrior_fury_2pc_cleave');

-- ============================================================================
-- Batch 3 bindings
-- ============================================================================

-- ============================================================================
-- Shaman Elemental 4pc — Lightning's Reach: Lightning Bolt damage +5%
-- ============================================================================
-- Lightning Bolt spell ranks in WotLK 3.3.5a:
--   403    rank 1  (level 1)
--   529    rank 2  (level 8)
--   548    rank 3  (level 14)
--   915    rank 4  (level 20)   <- Bracket 1 entry
--   943    rank 5  (level 26)
--   6041   rank 6  (level 32)   <- top of Bracket 1
--   10391  rank 7  (level 40)
--   10392  rank 8  (level 48)
--   15207  rank 9  (level 56)
--   15208  rank 10 (level 60)
--   25448  rank 11 (level 66)
--   25449  rank 12 (level 72)
--   49237  rank 13 (level 74)
--   49238  rank 14 (level 80)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (403,   'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (529,   'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (548,   'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (915,   'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (943,   'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (6041,  'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (10391, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (10392, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (15207, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (15208, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (25448, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (25449, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (49237, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt'),
    (49238, 'spell_bracketsets_shaman_ele_4pc_lightning_bolt');

-- ============================================================================
-- Hunter Survival 4pc — Detonation: Immolation Trap initial damage +25%
-- ============================================================================
-- Bound to the trap-effect damage spell (NOT the trap-creation spell):
--   13797  rank 1 (level 16)
--   14299  rank 2 (level 26)   <- Bracket 1 mid
--   14300  rank 3 (level 36)
--   14301  rank 4 (level 46)
--   27024  rank 5 (level 56)
--   49053  rank 6 (level 66)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (13797, 'spell_bracketsets_hunter_surv_4pc_immolation_trap'),
    (14299, 'spell_bracketsets_hunter_surv_4pc_immolation_trap'),
    (14300, 'spell_bracketsets_hunter_surv_4pc_immolation_trap'),
    (14301, 'spell_bracketsets_hunter_surv_4pc_immolation_trap'),
    (27024, 'spell_bracketsets_hunter_surv_4pc_immolation_trap'),
    (49053, 'spell_bracketsets_hunter_surv_4pc_immolation_trap');

-- ============================================================================
-- Priest Discipline 2pc — Reinforced Shield: Power Word: Shield absorb +10%
-- ============================================================================
-- PW:Shield spell ranks in WotLK 3.3.5a:
--   17     rank 1  (level 6)
--   592    rank 2  (level 12)
--   600    rank 3  (level 18)
--   3747   rank 4  (level 24)
--   6065   rank 5  (level 30)   <- Bracket 1 mid
--   6066   rank 6  (level 36)
--   10898  rank 7  (level 42)
--   10899  rank 8  (level 48)
--   10900  rank 9  (level 54)
--   10901  rank 10 (level 60)
--   25217  rank 11 (level 66)
--   25218  rank 12 (level 70)
--   48065  rank 13 (level 75)
--   48066  rank 14 (level 80)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (17,    'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (592,   'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (600,   'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (3747,  'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (6065,  'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (6066,  'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (10898, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (10899, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (10900, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (10901, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (25217, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (25218, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (48065, 'spell_bracketsets_priest_disc_2pc_power_word_shield'),
    (48066, 'spell_bracketsets_priest_disc_2pc_power_word_shield');

-- ============================================================================
-- Druid Restoration 2pc — Rejuvenated: Rejuvenation periodic heal +10%
-- ============================================================================
-- Rejuvenation ranks in WotLK 3.3.5a:
--   774    rank 1  (level 4)
--   1058   rank 2  (level 10)
--   1430   rank 3  (level 16)
--   2090   rank 4  (level 22)   <- Bracket 1 entry
--   2091   rank 5  (level 28)
--   3627   rank 6  (level 34)   <- top of Bracket 1
--   8910   rank 7  (level 40)
--   9839   rank 8  (level 46)
--   9840   rank 9  (level 52)
--   9841   rank 10 (level 58)
--   25299  rank 11 (level 60)
--   26982  rank 12 (level 65)
--   48440  rank 13 (level 73)
--   48441  rank 14 (level 79)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (774,   'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (1058,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (1430,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (2090,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (2091,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (3627,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (8910,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (9839,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (9840,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (9841,  'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (25299, 'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (26982, 'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (48440, 'spell_bracketsets_druid_resto_2pc_rejuvenation'),
    (48441, 'spell_bracketsets_druid_resto_2pc_rejuvenation');

-- ============================================================================
-- Mage Frost 4pc — Glacial Lance: Frostbolt vs frozen target damage +30%
-- ============================================================================
-- Frostbolt ranks in WotLK 3.3.5a:
--   116    rank 1  (level 1)
--   205    rank 2  (level 6)
--   837    rank 3  (level 12)
--   7322   rank 4  (level 18)
--   8406   rank 5  (level 24)   <- Bracket 1 mid
--   8407   rank 6  (level 30)
--   8408   rank 7  (level 36)
--   10179  rank 8  (level 42)
--   10180  rank 9  (level 48)
--   10181  rank 10 (level 54)
--   25304  rank 11 (level 60)
--   27071  rank 12 (level 66)
--   27072  rank 13 (level 72)
--   38697  alt rank
--   42841  rank 14 (level 74)
--   42842  rank 15 (level 79)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (116,   'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (205,   'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (837,   'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (7322,  'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (8406,  'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (8407,  'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (8408,  'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (10179, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (10180, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (10181, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (25304, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (27071, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (27072, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (42841, 'spell_bracketsets_mage_frost_4pc_frostbolt'),
    (42842, 'spell_bracketsets_mage_frost_4pc_frostbolt');

-- ============================================================================
-- Batch 4 bindings
-- ============================================================================

-- ============================================================================
-- Mage Fire 4pc — Pyroblast Surge: Fire Blast cooldown -1s
-- ============================================================================
-- Fire Blast spell ranks in WotLK 3.3.5a:
--   2136   rank 1  (level 6)
--   2137   rank 2  (level 14)
--   2138   rank 3  (level 22)   <- Bracket 1 entry
--   8412   rank 4  (level 30)   <- Bracket 1 mid
--   8413   rank 5  (level 38)
--   10197  rank 6  (level 46)
--   10199  rank 7  (level 54)
--   27078  rank 8  (level 60)
--   27079  rank 9  (level 66)
--   42872  rank 10 (level 73)
--   42873  rank 11 (level 79)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (2136,  'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (2137,  'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (2138,  'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (8412,  'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (8413,  'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (10197, 'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (10199, 'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (27078, 'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (27079, 'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (42872, 'spell_bracketsets_mage_fire_4pc_fire_blast'),
    (42873, 'spell_bracketsets_mage_fire_4pc_fire_blast');

-- ============================================================================
-- Rogue Subtlety 4pc — Stepping Shadow: Vanish cooldown -30s
-- ============================================================================
-- Vanish ranks in WotLK 3.3.5a:
--   1856   rank 1 (level 26)   <- Bracket 1 (29s base CD * 0.75 talent? -> ~3min in WotLK actually)
--   1857   rank 2 (level 44)
--   26889  rank 3 (level 62)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1856,  'spell_bracketsets_rogue_sub_4pc_vanish'),
    (1857,  'spell_bracketsets_rogue_sub_4pc_vanish'),
    (26889, 'spell_bracketsets_rogue_sub_4pc_vanish');

-- ============================================================================
-- Warlock Destruction 4pc — Lingering Flame: Immolate duration +6s
-- ============================================================================
-- Immolate ranks (periodic Fire DoT):
--   348    rank 1  (level 1)
--   707    rank 2  (level 10)
--   1094   rank 3  (level 20)   <- Bracket 1 entry
--   2941   rank 4  (level 30)   <- Bracket 1 mid
--   11665  rank 5  (level 40)
--   11667  rank 6  (level 50)
--   11668  rank 7  (level 60)
--   25309  rank 8  (level 66)
--   27215  rank 9  (level 70)
--   47810  rank 10 (level 75)
--   47811  rank 11 (level 80)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (348,   'spell_bracketsets_warlock_destro_4pc_immolate'),
    (707,   'spell_bracketsets_warlock_destro_4pc_immolate'),
    (1094,  'spell_bracketsets_warlock_destro_4pc_immolate'),
    (2941,  'spell_bracketsets_warlock_destro_4pc_immolate'),
    (11665, 'spell_bracketsets_warlock_destro_4pc_immolate'),
    (11667, 'spell_bracketsets_warlock_destro_4pc_immolate'),
    (11668, 'spell_bracketsets_warlock_destro_4pc_immolate'),
    (25309, 'spell_bracketsets_warlock_destro_4pc_immolate'),
    (27215, 'spell_bracketsets_warlock_destro_4pc_immolate'),
    (47810, 'spell_bracketsets_warlock_destro_4pc_immolate'),
    (47811, 'spell_bracketsets_warlock_destro_4pc_immolate');

-- ============================================================================
-- Mage Frost 2pc — Frozen Tide: Frost Nova freeze duration +1s
-- ============================================================================
-- Frost Nova ranks (WotLK Spell.dbc, verified):
--   122    rank 1 (level 10)
--   865    rank 2 (level 20)   <- Bracket 1 entry
--   6131   rank 3 (level 34)   <- top of Bracket 1
--   10230  rank 4 (level 48)
--   27088  rank 5 (level 60) -- last real rank in 3.3.5a
-- (42939 and 42940 are NOT Frost Nova -- they're Blizzard R7/R8)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (122,   'spell_bracketsets_mage_frost_2pc_frost_nova'),
    (865,   'spell_bracketsets_mage_frost_2pc_frost_nova'),
    (6131,  'spell_bracketsets_mage_frost_2pc_frost_nova'),
    (10230, 'spell_bracketsets_mage_frost_2pc_frost_nova'),
    (27088, 'spell_bracketsets_mage_frost_2pc_frost_nova');

-- ============================================================================
-- Batch 5 bindings
-- ============================================================================

-- ============================================================================
-- Rogue Combat 2pc — Sinister Edge: Sinister Strike 10% chance +1 energy
-- ============================================================================
-- Sinister Strike ranks in WotLK 3.3.5a:
--   1752   rank 1  (level 1)
--   1757   rank 2  (level 6)
--   1758   rank 3  (level 12)
--   1759   rank 4  (level 20)   <- Bracket 1 entry
--   1760   rank 5  (level 28)   <- Bracket 1 mid
--   8621   rank 6  (level 36)
--   11293  rank 7  (level 44)
--   11294  rank 8  (level 52)
--   26861  rank 9  (level 60)
--   26862  rank 10 (level 68)
--   48637  rank 11 (level 74)
--   48638  rank 12 (level 79)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1752,  'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (1757,  'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (1758,  'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (1759,  'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (1760,  'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (8621,  'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (11293, 'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (11294, 'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (26861, 'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (26862, 'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (48637, 'spell_bracketsets_rogue_combat_2pc_sinister_strike'),
    (48638, 'spell_bracketsets_rogue_combat_2pc_sinister_strike');

-- ============================================================================
-- Hunter BM 2pc — Tide-Born Beast: Pet damage +5% (Pattern J: marker-bound)
-- ============================================================================
-- IMPORTANT: this script binds to the MARKER spell ID, not a target ability.
-- The marker is already bound to spell_bracketsets_placeholder in pack 04
-- (for chat notification). AC supports multiple bindings per spell ID, so
-- both scripts fire on marker apply/remove: placeholder sends chat AND this
-- script modifies pet stats.
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (64854, 'spell_bracketsets_hunter_bm_2pc_pet_damage');

-- ============================================================================
-- Warlock Demo 2pc — Demonic Augmentation: Pet damage +10% (Pattern J)
-- ============================================================================
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (64932, 'spell_bracketsets_warlock_demo_2pc_pet_damage');

-- ============================================================================
-- Batch 6 bindings
-- ============================================================================

-- ============================================================================
-- Druid Feral 2pc — Primal Economy (Claw): Claw -5 energy
-- ============================================================================
-- Claw (Cat form) ranks in WotLK 3.3.5a:
--   1082   rank 1 (level 20)   <- Bracket 1 entry
--   3029   rank 2 (level 26)
--   5201   rank 3 (level 34)   <- top of Bracket 1
--   9849   rank 4 (level 42)
--   9850   rank 5 (level 50)
--   27000  rank 6 (level 58)
--   48569  rank 7 (level 67)
--   48570  rank 8 (level 75)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1082,  'spell_bracketsets_druid_feral_2pc_claw'),
    (3029,  'spell_bracketsets_druid_feral_2pc_claw'),
    (5201,  'spell_bracketsets_druid_feral_2pc_claw'),
    (9849,  'spell_bracketsets_druid_feral_2pc_claw'),
    (9850,  'spell_bracketsets_druid_feral_2pc_claw'),
    (27000, 'spell_bracketsets_druid_feral_2pc_claw'),
    (48569, 'spell_bracketsets_druid_feral_2pc_claw'),
    (48570, 'spell_bracketsets_druid_feral_2pc_claw');

-- ============================================================================
-- Druid Feral 2pc — Primal Economy (Maul): Maul -5 rage
-- ============================================================================
-- Maul (Bear form) ranks in WotLK 3.3.5a:
--   6807   rank 1  (level 10)
--   6808   rank 2  (level 18)
--   6809   rank 3  (level 26)   <- Bracket 1 mid
--   8972   rank 4  (level 34)   <- top of Bracket 1
--   9745   rank 5  (level 42)
--   9880   rank 6  (level 50)
--   9881   rank 7  (level 58)
--   26996  rank 8  (level 60)
--   48479  rank 9  (level 67)
--   48480  rank 10 (level 75)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (6807,  'spell_bracketsets_druid_feral_2pc_maul'),
    (6808,  'spell_bracketsets_druid_feral_2pc_maul'),
    (6809,  'spell_bracketsets_druid_feral_2pc_maul'),
    (8972,  'spell_bracketsets_druid_feral_2pc_maul'),
    (9745,  'spell_bracketsets_druid_feral_2pc_maul'),
    (9880,  'spell_bracketsets_druid_feral_2pc_maul'),
    (9881,  'spell_bracketsets_druid_feral_2pc_maul'),
    (26996, 'spell_bracketsets_druid_feral_2pc_maul'),
    (48479, 'spell_bracketsets_druid_feral_2pc_maul'),
    (48480, 'spell_bracketsets_druid_feral_2pc_maul');

-- ============================================================================
-- Shaman Restoration 2pc — Cascading Tide: Healing Wave mana cost -10%
-- ============================================================================
-- Healing Wave ranks in WotLK 3.3.5a:
--   331    rank 1  (level 1)
--   332    rank 2  (level 6)
--   547    rank 3  (level 12)
--   913    rank 4  (level 18)
--   939    rank 5  (level 24)   <- Bracket 1 mid
--   959    rank 6  (level 32)
--   8005   rank 7  (level 40)
--   10395  rank 8  (level 48)
--   10396  rank 9  (level 56)
--   25357  rank 10 (level 60)
--   25391  rank 11 (level 66)
--   25396  rank 12 (level 72)
--   49272  rank 13 (level 74)
--   49273  rank 14 (level 80)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (331,   'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (332,   'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (547,   'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (913,   'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (939,   'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (959,   'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (8005,  'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (10395, 'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (10396, 'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (25357, 'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (25391, 'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (25396, 'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (49272, 'spell_bracketsets_shaman_resto_2pc_healing_wave'),
    (49273, 'spell_bracketsets_shaman_resto_2pc_healing_wave');

-- ============================================================================
-- Paladin Retribution 4pc — Verdict of Light: Judgement damage +15%
-- ============================================================================
-- In WotLK 3.3.5a, Judgement is a unified spell (20271 "Judgement of Light"
-- is the primary cast that triggers seal-specific judgement effects).
-- Bind to all Judgement variants for coverage.
--   20271  Judgement (the cast itself, all WotLK Pally judgements unified)
--   20184  Judgement of Justice (debuff component)
--   20185  Judgement of Light (effect on target)
--   20186  Judgement of Wisdom (effect on target)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (20271, 'spell_bracketsets_paladin_ret_4pc_judgement'),
    (20184, 'spell_bracketsets_paladin_ret_4pc_judgement'),
    (20185, 'spell_bracketsets_paladin_ret_4pc_judgement'),
    (20186, 'spell_bracketsets_paladin_ret_4pc_judgement');

-- ============================================================================
-- Batch 7 bindings
-- ============================================================================

-- Warrior Fury 4pc — Wrathful Strike: Heroic Strike 15% chance no rage
-- Heroic Strike ranks: 78, 284, 285, 1608, 11564-11567, 25286, 29707, 30324, 47449-47450
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (78,    'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (284,   'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (285,   'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (1608,  'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (11564, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (11565, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (11566, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (11567, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (25286, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (29707, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (30324, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (47449, 'spell_bracketsets_warrior_fury_4pc_heroic_strike'),
    (47450, 'spell_bracketsets_warrior_fury_4pc_heroic_strike');

-- Druid Feral 4pc — Beast's Refund: Maul 15% chance refund full rage
-- Maul ranks (same as batch 6 Primal Economy Maul script — stack additively)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (6807,  'spell_bracketsets_druid_feral_4pc_maul'),
    (6808,  'spell_bracketsets_druid_feral_4pc_maul'),
    (6809,  'spell_bracketsets_druid_feral_4pc_maul'),
    (8972,  'spell_bracketsets_druid_feral_4pc_maul'),
    (9745,  'spell_bracketsets_druid_feral_4pc_maul'),
    (9880,  'spell_bracketsets_druid_feral_4pc_maul'),
    (9881,  'spell_bracketsets_druid_feral_4pc_maul'),
    (26996, 'spell_bracketsets_druid_feral_4pc_maul'),
    (48479, 'spell_bracketsets_druid_feral_4pc_maul'),
    (48480, 'spell_bracketsets_druid_feral_4pc_maul');

-- Rogue Subtlety 2pc — Backstab's Bite: Backstab +10% damage when behind target
-- Backstab ranks: 53, 2589-2591, 8721, 11279-11281, 25300, 26863, 48656-48657
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (53,    'spell_bracketsets_rogue_sub_2pc_backstab'),
    (2589,  'spell_bracketsets_rogue_sub_2pc_backstab'),
    (2590,  'spell_bracketsets_rogue_sub_2pc_backstab'),
    (2591,  'spell_bracketsets_rogue_sub_2pc_backstab'),
    (8721,  'spell_bracketsets_rogue_sub_2pc_backstab'),
    (11279, 'spell_bracketsets_rogue_sub_2pc_backstab'),
    (11280, 'spell_bracketsets_rogue_sub_2pc_backstab'),
    (11281, 'spell_bracketsets_rogue_sub_2pc_backstab'),
    (25300, 'spell_bracketsets_rogue_sub_2pc_backstab'),
    (26863, 'spell_bracketsets_rogue_sub_2pc_backstab'),
    (48656, 'spell_bracketsets_rogue_sub_2pc_backstab'),
    (48657, 'spell_bracketsets_rogue_sub_2pc_backstab');

-- Hunter MM 2pc — Piercing Shot (simplified): Arcane Shot damage +10%
-- Arcane Shot ranks: 3044, 14281-14287, 27019, 49044-49045
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (3044,  'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14281, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14282, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14283, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14284, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14285, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14286, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (14287, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (27019, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (49044, 'spell_bracketsets_hunter_mm_2pc_arcane_shot'),
    (49045, 'spell_bracketsets_hunter_mm_2pc_arcane_shot');

-- Priest Shadow 2pc — Mind Crack (simplified): Mind Blast damage +15%
-- Mind Blast ranks: 8092, 8102-8106, 10945-10947, 25372, 25375, 48126-48127
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (8092,  'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (8102,  'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (8103,  'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (8104,  'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (8105,  'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (8106,  'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (10945, 'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (10946, 'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (10947, 'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (25372, 'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (25375, 'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (48126, 'spell_bracketsets_priest_shadow_2pc_mind_blast'),
    (48127, 'spell_bracketsets_priest_shadow_2pc_mind_blast');

-- ============================================================================
-- Batch 8 bindings
-- ============================================================================

-- Hunter BM 4pc Primal Frenzy: pet HP +10% — bound to MARKER (not target)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (64860, 'spell_bracketsets_hunter_bm_4pc_pet_hp');

-- Paladin Prot 2pc Righteous Provocation: Hand of Reckoning +50% dmg
-- Hand of Reckoning spell: 62124
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (62124, 'spell_bracketsets_paladin_prot_2pc_hand_of_reckoning');

-- Paladin Prot 4pc Sanctified Ground: Consecration tick damage +20% (V1)
-- Was "Blessing of Sanctuary +2% DR" -- redesigned because WotLK BoS uses
-- a dummy aura that AC handles via custom damage code, not via a real
-- percent-taken aura, so Pattern A had nothing to hook. Consecration's
-- DBC layout (verified via .bracketsets diag, all 8 ranks identical):
-- EFFECT_0 = PersistentAreaAura + PERIODIC_DAMAGE @ 1s tick.
-- Consecration ranks: 26573 (R1 L20), 20116 (R2 L30), 20922 (R3 L40),
--   20923 (R4 L48), 20924 (R5 L56), 27173 (R6 L64), 48818 (R7 L70),
--   48819 (R8 L75). R1 and R2 cover the Bracket 1 25-34 range natively.
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (26573, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (20116, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (20922, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (20923, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (20924, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (27173, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (48818, 'spell_bracketsets_paladin_prot_4pc_consecration'),
    (48819, 'spell_bracketsets_paladin_prot_4pc_consecration');

-- Warrior Prot 2pc Bulwark Surge: Shield Block +5% block value
-- Shield Block in WotLK 3.3.5a: 2565
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (2565, 'spell_bracketsets_warrior_prot_2pc_shield_block');

-- Mage Arcane 2pc Final Missile: Arcane Missile damage +15% (simplified)
-- Arcane Missile damage spells: 7268 (r1), 7269, 7270, 8418, 8419, 10273,
-- 10274, 25346, 27076, 38703, 38704, 42832, 42833
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (7268,  'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (7269,  'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (7270,  'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (8418,  'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (8419,  'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (10273, 'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (10274, 'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (25346, 'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (27076, 'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (38704, 'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (42832, 'spell_bracketsets_mage_arc_2pc_arcane_missile'),
    (42833, 'spell_bracketsets_mage_arc_2pc_arcane_missile');

-- ============================================================================
-- Batch 9 bindings
-- ============================================================================

-- Druid Balance 2pc Lunar Cycle — Moonfire periodic crit (emulated as 15% double)
-- Moonfire ranks: 8921 (r1), 8924-8929, 9833, 9834, 9835, 26987, 26988, 48462, 48463
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (8921,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (8924,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (8925,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (8926,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (8927,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (8928,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (8929,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (9833,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (9834,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (9835,  'spell_bracketsets_druid_balance_2pc_moonfire'),
    (26987, 'spell_bracketsets_druid_balance_2pc_moonfire'),
    (26988, 'spell_bracketsets_druid_balance_2pc_moonfire'),
    (48462, 'spell_bracketsets_druid_balance_2pc_moonfire'),
    (48463, 'spell_bracketsets_druid_balance_2pc_moonfire');

-- Priest Holy 2pc Renewed Light — Renew tick 15% double
-- Renew ranks: 139 (r1), 6074-6078, 10927-10929, 25315, 25316, 48067, 48068
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (139,   'spell_bracketsets_priest_holy_2pc_renew'),
    (6074,  'spell_bracketsets_priest_holy_2pc_renew'),
    (6075,  'spell_bracketsets_priest_holy_2pc_renew'),
    (6076,  'spell_bracketsets_priest_holy_2pc_renew'),
    (6077,  'spell_bracketsets_priest_holy_2pc_renew'),
    (6078,  'spell_bracketsets_priest_holy_2pc_renew'),
    (10927, 'spell_bracketsets_priest_holy_2pc_renew'),
    (10928, 'spell_bracketsets_priest_holy_2pc_renew'),
    (10929, 'spell_bracketsets_priest_holy_2pc_renew'),
    (25315, 'spell_bracketsets_priest_holy_2pc_renew'),
    (48067, 'spell_bracketsets_priest_holy_2pc_renew'),
    (48068, 'spell_bracketsets_priest_holy_2pc_renew');

-- Priest Shadow 4pc Vampire's Renewal — SW:Pain ticks heal caster 1% maxHP
-- SW:Pain ranks: 589 (r1), 594, 970, 992, 2767, 10892, 10893, 10894, 25367, 25368, 48124, 48125
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (589,   'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (594,   'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (970,   'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (992,   'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (2767,  'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (10892, 'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (10893, 'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (10894, 'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (25367, 'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (25368, 'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (48124, 'spell_bracketsets_priest_shadow_4pc_sw_pain'),
    (48125, 'spell_bracketsets_priest_shadow_4pc_sw_pain');

-- Rogue Assn 4pc Envenom's Wake — Rupture tick 10% chance refund 1 energy
-- Rupture ranks: 1943, 8639, 8640, 11273, 11274, 11275, 26867, 48671, 48672
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1943,  'spell_bracketsets_rogue_assn_4pc_rupture'),
    (8639,  'spell_bracketsets_rogue_assn_4pc_rupture'),
    (8640,  'spell_bracketsets_rogue_assn_4pc_rupture'),
    (11273, 'spell_bracketsets_rogue_assn_4pc_rupture'),
    (11274, 'spell_bracketsets_rogue_assn_4pc_rupture'),
    (11275, 'spell_bracketsets_rogue_assn_4pc_rupture'),
    (26867, 'spell_bracketsets_rogue_assn_4pc_rupture'),
    (48671, 'spell_bracketsets_rogue_assn_4pc_rupture'),
    (48672, 'spell_bracketsets_rogue_assn_4pc_rupture');

-- Warlock Demo 4pc Soulbound — Health Funnel transfers +25%
-- Health Funnel ranks: 755, 3698, 3699, 3700, 11693, 11694, 11695, 27259, 47856
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (755,   'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (3698,  'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (3699,  'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (3700,  'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (11693, 'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (11694, 'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (11695, 'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (27259, 'spell_bracketsets_warlock_demo_4pc_health_funnel'),
    (47856, 'spell_bracketsets_warlock_demo_4pc_health_funnel');

-- ============================================================================
-- Batch 10 bindings
-- ============================================================================

-- Warrior Prot 4pc Devastating Cycle (compound, Revenge -> Thunder Clap)
-- Revenge ranks: 6572, 6574, 7379, 11600, 11601, 25288, 25269, 30357, 57823
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (6572,  'spell_bracketsets_warrior_prot_4pc_revenge'),
    (6574,  'spell_bracketsets_warrior_prot_4pc_revenge'),
    (7379,  'spell_bracketsets_warrior_prot_4pc_revenge'),
    (11600, 'spell_bracketsets_warrior_prot_4pc_revenge'),
    (11601, 'spell_bracketsets_warrior_prot_4pc_revenge'),
    (25288, 'spell_bracketsets_warrior_prot_4pc_revenge'),
    (25269, 'spell_bracketsets_warrior_prot_4pc_revenge'),
    (30357, 'spell_bracketsets_warrior_prot_4pc_revenge'),
    (57823, 'spell_bracketsets_warrior_prot_4pc_revenge');
-- Thunder Clap ranks: 6343, 8198, 8204, 8205, 11580, 11581, 25264, 47501, 47502
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (6343,  'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (8198,  'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (8204,  'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (8205,  'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (11580, 'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (11581, 'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (25264, 'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (47501, 'spell_bracketsets_warrior_prot_4pc_thunder_clap'),
    (47502, 'spell_bracketsets_warrior_prot_4pc_thunder_clap');

-- Mage Arc 4pc Sustained Power (compound, Arc Explosion -> Arc Missile)
-- Arc Explosion ranks: 1449, 8437, 8438, 8439, 10201, 10202, 27082, 33933, 42920, 42921
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1449,  'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (8437,  'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (8438,  'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (8439,  'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (10201, 'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (10202, 'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (27082, 'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (33933, 'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (42920, 'spell_bracketsets_mage_arc_4pc_arcane_explosion'),
    (42921, 'spell_bracketsets_mage_arc_4pc_arcane_explosion');
-- Arc Missile damage ranks (same as batch 8 binding)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (7268,  'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (7269,  'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (7270,  'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (8418,  'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (8419,  'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (10273, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (10274, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (25346, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (27076, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (38704, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (42832, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus'),
    (42833, 'spell_bracketsets_mage_arc_4pc_arc_missile_bonus');

-- Shaman Resto 4pc Quick Mend (compound, LHW -> HW)
-- LHW ranks: 8004, 8008, 8010, 10466, 10467, 10468, 25420, 27624, 49275, 49276
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (8004,  'spell_bracketsets_shaman_resto_4pc_lhw'),
    (8008,  'spell_bracketsets_shaman_resto_4pc_lhw'),
    (8010,  'spell_bracketsets_shaman_resto_4pc_lhw'),
    (10466, 'spell_bracketsets_shaman_resto_4pc_lhw'),
    (10467, 'spell_bracketsets_shaman_resto_4pc_lhw'),
    (10468, 'spell_bracketsets_shaman_resto_4pc_lhw'),
    (25420, 'spell_bracketsets_shaman_resto_4pc_lhw'),
    (27624, 'spell_bracketsets_shaman_resto_4pc_lhw'),
    (49275, 'spell_bracketsets_shaman_resto_4pc_lhw'),
    (49276, 'spell_bracketsets_shaman_resto_4pc_lhw');
-- HW ranks (same as Cascading Tide batch 6 binding)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (331,   'spell_bracketsets_shaman_resto_4pc_hw'),
    (332,   'spell_bracketsets_shaman_resto_4pc_hw'),
    (547,   'spell_bracketsets_shaman_resto_4pc_hw'),
    (913,   'spell_bracketsets_shaman_resto_4pc_hw'),
    (939,   'spell_bracketsets_shaman_resto_4pc_hw'),
    (959,   'spell_bracketsets_shaman_resto_4pc_hw'),
    (8005,  'spell_bracketsets_shaman_resto_4pc_hw'),
    (10395, 'spell_bracketsets_shaman_resto_4pc_hw'),
    (10396, 'spell_bracketsets_shaman_resto_4pc_hw'),
    (25357, 'spell_bracketsets_shaman_resto_4pc_hw'),
    (25391, 'spell_bracketsets_shaman_resto_4pc_hw'),
    (25396, 'spell_bracketsets_shaman_resto_4pc_hw'),
    (49272, 'spell_bracketsets_shaman_resto_4pc_hw'),
    (49273, 'spell_bracketsets_shaman_resto_4pc_hw');

-- Paladin Holy 2pc Light's Compact: Flash of Light mana cost -10% (simplified)
-- FoL ranks: 19750, 19939-19943, 25514, 27137, 48784, 48785
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (19750, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (19939, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (19940, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (19941, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (19942, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (19943, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (25514, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (27137, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (48784, 'spell_bracketsets_paladin_holy_2pc_flash_of_light'),
    (48785, 'spell_bracketsets_paladin_holy_2pc_flash_of_light');

-- Druid Balance 4pc Solar Cadence: Wrath 10% chance refund mana (simplified)
-- Wrath ranks: 5176, 5177, 5178, 5179, 5180, 6780, 8905, 9912, 26984, 26985, 48459, 48460
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (5176,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (5177,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (5178,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (5179,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (5180,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (6780,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (8905,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (9912,  'spell_bracketsets_druid_balance_4pc_wrath'),
    (26984, 'spell_bracketsets_druid_balance_4pc_wrath'),
    (26985, 'spell_bracketsets_druid_balance_4pc_wrath'),
    (48459, 'spell_bracketsets_druid_balance_4pc_wrath'),
    (48460, 'spell_bracketsets_druid_balance_4pc_wrath');

-- ============================================================================
-- Batch 11 bindings
-- ============================================================================

-- Overpower ranks: 7384, 7887, 11584, 11585, 24407, 25275, 51223
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (7384,  'spell_bracketsets_warrior_arms_4pc_overpower'),
    (7887,  'spell_bracketsets_warrior_arms_4pc_overpower'),
    (11584, 'spell_bracketsets_warrior_arms_4pc_overpower'),
    (11585, 'spell_bracketsets_warrior_arms_4pc_overpower'),
    (24407, 'spell_bracketsets_warrior_arms_4pc_overpower'),
    (25275, 'spell_bracketsets_warrior_arms_4pc_overpower'),
    (51223, 'spell_bracketsets_warrior_arms_4pc_overpower');

-- Multi-Shot ranks: 2643, 14288, 14289, 14290, 25294, 27021, 49047, 49048
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (2643,  'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (14288, 'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (14289, 'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (14290, 'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (25294, 'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (27021, 'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (49047, 'spell_bracketsets_hunter_mm_4pc_multi_shot'),
    (49048, 'spell_bracketsets_hunter_mm_4pc_multi_shot');

-- Priest Disc 4pc Suppression's Echo: PW:Shield duration +2s (simplified)
-- Same PW:Shield ranks as Reinforced Shield binding; both scripts coexist
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (17,    'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (592,   'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (600,   'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (3747,  'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (6065,  'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (6066,  'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (10898, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (10899, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (10900, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (10901, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (25217, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (25218, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (48065, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration'),
    (48066, 'spell_bracketsets_priest_disc_4pc_pw_shield_duration');

-- Shaman Ele 2pc Stormcaller's Insight: Lightning Bolt mana -5% (simplified)
-- Same Lightning Bolt ranks as Lightning's Reach binding; both coexist
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (403,   'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (529,   'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (548,   'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (915,   'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (943,   'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (6041,  'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (10391, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (10392, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (15207, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (15208, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (25448, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (25449, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (49237, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt'),
    (49238, 'spell_bracketsets_shaman_ele_2pc_lightning_bolt');

-- Warlock Destro 2pc Shadow Cadence: Shadow Bolt 10% chance refund 50% mana
-- Shadow Bolt ranks: 686, 695, 705, 1088, 1106, 7641, 11659, 11660, 11661, 25307, 27209, 47808, 47809
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (686,   'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (695,   'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (705,   'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (1088,  'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (1106,  'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (7641,  'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (11659, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (11660, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (11661, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (25307, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (27209, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (47808, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt'),
    (47809, 'spell_bracketsets_warlock_destro_2pc_shadow_bolt');

-- Druid Resto 4pc Wake of Bloom: Regrowth periodic heal +10% (simplified)
-- Regrowth ranks (WotLK Spell.dbc, verified): 8936, 8938-8941, 9750, 9856-9858, 26980, 26981, 48442, 48443
-- (8942 and 8943 do NOT exist — Regrowth jumps from R5=8941 to R6=9750)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (8936,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (8938,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (8939,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (8940,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (8941,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (9750,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (9856,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (9857,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (9858,  'spell_bracketsets_druid_resto_4pc_regrowth'),
    (26980, 'spell_bracketsets_druid_resto_4pc_regrowth'),
    (26981, 'spell_bracketsets_druid_resto_4pc_regrowth'),
    (48442, 'spell_bracketsets_druid_resto_4pc_regrowth'),
    (48443, 'spell_bracketsets_druid_resto_4pc_regrowth');

-- ============================================================================
-- Batch 12 (final) bindings — 54 of 54 complete
-- ============================================================================

-- Paladin Holy 4pc Mending Spark: Holy Light healing +10% (simplified)
-- Holy Light ranks: 635, 639, 647, 1026, 1042, 3472, 10328, 10329, 25292, 27135, 27136, 48781, 48782
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (635,   'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (639,   'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (647,   'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (1026,  'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (1042,  'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (3472,  'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (10328, 'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (10329, 'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (25292, 'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (27135, 'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (27136, 'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (48781, 'spell_bracketsets_paladin_holy_4pc_holy_light'),
    (48782, 'spell_bracketsets_paladin_holy_4pc_holy_light');

-- Paladin Ret 2pc Crusader's Fervor: Seal of Righteousness damage +20%
-- SoR proc damage spell in WotLK 3.3.5a: 25742 (single unified ID)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (25742, 'spell_bracketsets_paladin_ret_2pc_sor');

-- Shaman Enh 2pc Stormstrike's Echo: Lightning Shield damage +15%
-- LS proc damage spell IDs (one per buff rank):
--   26364, 26365, 26366, 26367, 26369, 26370, 26363, 26371, 27635, 49278, 49279
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (26364, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26365, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26366, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26367, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26369, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26370, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26363, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (26371, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (27635, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (49278, 'spell_bracketsets_shaman_enh_2pc_lightning_shield'),
    (49279, 'spell_bracketsets_shaman_enh_2pc_lightning_shield');

-- Shaman Enh 4pc Searing Lash: Searing Totem attack damage +25%
-- Searing Bolt (totem's attack) ranks (WotLK Spell.dbc, verified):
--   3606, 6350, 6351, 6352, 8377, 25530, 49340, 49341
-- (8378 does NOT exist — Searing Bolt jumps from R5=8377 to R6=25530)
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (3606,  'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (6350,  'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (6351,  'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (6352,  'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (8377,  'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (25530, 'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (49340, 'spell_bracketsets_shaman_enh_4pc_searing_totem'),
    (49341, 'spell_bracketsets_shaman_enh_4pc_searing_totem');

-- Warlock Aff 4pc Soul Drain: Drain Soul tick refunds 5% max mana (simplified)
-- Drain Soul ranks: 1120, 8288, 8289, 11675, 27217, 47855
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (1120,  'spell_bracketsets_warlock_aff_4pc_drain_soul'),
    (8288,  'spell_bracketsets_warlock_aff_4pc_drain_soul'),
    (8289,  'spell_bracketsets_warlock_aff_4pc_drain_soul'),
    (11675, 'spell_bracketsets_warlock_aff_4pc_drain_soul'),
    (27217, 'spell_bracketsets_warlock_aff_4pc_drain_soul'),
    (47855, 'spell_bracketsets_warlock_aff_4pc_drain_soul');

-- Rogue Combat 4pc Adrenaline's Edge: Eviscerate +10% damage per CP above 3
-- Eviscerate ranks: 2098, 6760, 6761, 6762, 8623, 8624, 11299, 11300, 31016, 26865, 48667, 48668
INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    (2098,  'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (6760,  'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (6761,  'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (6762,  'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (8623,  'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (8624,  'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (11299, 'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (11300, 'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (31016, 'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (26865, 'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (48667, 'spell_bracketsets_rogue_combat_4pc_eviscerate'),
    (48668, 'spell_bracketsets_rogue_combat_4pc_eviscerate');
