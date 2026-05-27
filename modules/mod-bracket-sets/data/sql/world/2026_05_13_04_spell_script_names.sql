--
-- mod-bracket-sets — bind 54 marker spell IDs to spell_bracketsets_placeholder
--
-- AzerothCore's SpellMgr loads `spell_script_names` at boot and attaches the
-- named SpellScript/AuraScript class to each listed spell ID. This row set
-- binds all 54 Bracket 1 marker spells to spell_bracketsets_placeholder,
-- which is a no-op log+chat-notification script defined in
-- src/BracketSetsBonusEffects.cpp.
--
-- Display names match the refit catalog (see kb_0d42f4e5 and
-- docs/superpowers/specs/2026-05-13-phase-1-5-bracket-1-bonus-catalog-refit.md).
--
-- To replace a placeholder with a real per-bonus implementation:
--   1. Define a new AuraScript class in BracketSetsBonusEffects.cpp
--      (e.g., spell_bracketsets_warrior_arms_2pc_rend).
--   2. RegisterSpellScript(spell_bracketsets_warrior_arms_2pc_rend) in
--      RegisterBonusEffects().
--   3. UPDATE spell_script_names SET ScriptName = 'spell_bracketsets_warrior_arms_2pc_rend'
--      WHERE spell_id = 64938;
--   4. Reload world: .reload spell_script_names  (or restart worldserver).
--

-- Delete any prior bindings for our marker IDs so re-runs are idempotent.
DELETE FROM `spell_script_names`
WHERE `spell_id` IN (
    -- 54 IDs covering 9 classes × 3 specs × 2 thresholds for Bracket 1.

    -- Warrior
    64938, 64939, 67234, 67268, 64933, 64936,
    -- Paladin
    70755, 70756, 64881, 64882, 64878, 64879,
    -- Hunter
    64854, 64860, 67150, 70724, 70727, 70730,
    -- Rogue
    64914, 64915, 67209, 67211, 67186, 67187,
    -- Priest
    67202, 70798, 64910, 64912, 67193, 67198,
    -- Shaman
    67227, 64925, 67220, 67221, 67225, 67226,
    -- Mage
    64867, 67188, 67164, 67185, 67189, 70748,
    -- Warlock
    64931, 67231, 64932, 67230, 70839, 70841,
    -- Druid
    67125, 67126, 67121, 67123, 67127, 67128
);

INSERT INTO `spell_script_names` (`spell_id`, `ScriptName`) VALUES
    -- Warrior (class 1)
    (64938, 'spell_bracketsets_placeholder'),  -- Arms 2pc Sundered Resolve
    (64939, 'spell_bracketsets_placeholder'),  -- Arms 4pc Overpowering Strike
    (67234, 'spell_bracketsets_placeholder'),  -- Fury 2pc Berserker's Cadence
    (67268, 'spell_bracketsets_placeholder'),  -- Fury 4pc Wrathful Strike
    (64933, 'spell_bracketsets_placeholder'),  -- Prot 2pc Bulwark Surge
    (64936, 'spell_bracketsets_placeholder'),  -- Prot 4pc Devastating Cycle

    -- Paladin (class 2)
    (70755, 'spell_bracketsets_placeholder'),  -- Holy 2pc Light's Compact
    (70756, 'spell_bracketsets_placeholder'),  -- Holy 4pc Mending Spark
    (64881, 'spell_bracketsets_placeholder'),  -- Prot 2pc Righteous Provocation
    (64882, 'spell_bracketsets_placeholder'),  -- Prot 4pc Sanctified Bulwark
    (64878, 'spell_bracketsets_placeholder'),  -- Ret  2pc Crusader's Fervor
    (64879, 'spell_bracketsets_placeholder'),  -- Ret  4pc Verdict of Light

    -- Hunter (class 3)
    (64854, 'spell_bracketsets_placeholder'),  -- BM   2pc Tide-Born Beast
    (64860, 'spell_bracketsets_placeholder'),  -- BM   4pc Primal Frenzy
    (67150, 'spell_bracketsets_placeholder'),  -- MM   2pc Piercing Shot
    (70724, 'spell_bracketsets_placeholder'),  -- MM   4pc Arcane Cadence
    (70727, 'spell_bracketsets_placeholder'),  -- Surv 2pc Venomous Tide
    (70730, 'spell_bracketsets_placeholder'),  -- Surv 4pc Detonation

    -- Rogue (class 4)
    (64914, 'spell_bracketsets_placeholder'),  -- Assn   2pc Vile Toxins
    (64915, 'spell_bracketsets_placeholder'),  -- Assn   4pc Envenom's Wake
    (67209, 'spell_bracketsets_placeholder'),  -- Combat 2pc Sinister Edge
    (67211, 'spell_bracketsets_placeholder'),  -- Combat 4pc Adrenaline's Edge
    (67186, 'spell_bracketsets_placeholder'),  -- Sub    2pc Backstab's Bite
    (67187, 'spell_bracketsets_placeholder'),  -- Sub    4pc Stepping Shadow

    -- Priest (class 5)
    (67202, 'spell_bracketsets_placeholder'),  -- Disc   2pc Reinforced Shield
    (70798, 'spell_bracketsets_placeholder'),  -- Disc   4pc Suppression's Echo
    (64910, 'spell_bracketsets_placeholder'),  -- Holy   2pc Renewed Light
    (64912, 'spell_bracketsets_placeholder'),  -- Holy   4pc Fire of the Fold
    (67193, 'spell_bracketsets_placeholder'),  -- Shadow 2pc Mind Crack
    (67198, 'spell_bracketsets_placeholder'),  -- Shadow 4pc Vampire's Renewal

    -- Shaman (class 7)
    (67227, 'spell_bracketsets_placeholder'),  -- Ele   2pc Stormcaller's Insight
    (64925, 'spell_bracketsets_placeholder'),  -- Ele   4pc Lightning's Reach
    (67220, 'spell_bracketsets_placeholder'),  -- Enh   2pc Stormstrike's Echo
    (67221, 'spell_bracketsets_placeholder'),  -- Enh   4pc Searing Lash
    (67225, 'spell_bracketsets_placeholder'),  -- Resto 2pc Cascading Tide
    (67226, 'spell_bracketsets_placeholder'),  -- Resto 4pc Quick Mend

    -- Mage (class 8)
    (64867, 'spell_bracketsets_placeholder'),  -- Arc   2pc Final Missile
    (67188, 'spell_bracketsets_placeholder'),  -- Arc   4pc Sustained Power
    (67164, 'spell_bracketsets_placeholder'),  -- Fire  2pc Incandescent Burn
    (67185, 'spell_bracketsets_placeholder'),  -- Fire  4pc Pyroblast Surge
    (67189, 'spell_bracketsets_placeholder'),  -- Frost 2pc Frozen Tide
    (70748, 'spell_bracketsets_placeholder'),  -- Frost 4pc Glacial Lance

    -- Warlock (class 9)
    (64931, 'spell_bracketsets_placeholder'),  -- Aff    2pc Corrupted Flesh
    (67231, 'spell_bracketsets_placeholder'),  -- Aff    4pc Soul Drain
    (64932, 'spell_bracketsets_placeholder'),  -- Demo   2pc Demonic Augmentation
    (67230, 'spell_bracketsets_placeholder'),  -- Demo   4pc Soulbound
    (70839, 'spell_bracketsets_placeholder'),  -- Destro 2pc Shadow Cadence
    (70841, 'spell_bracketsets_placeholder'),  -- Destro 4pc Lingering Flame

    -- Druid (class 11)
    (67125, 'spell_bracketsets_placeholder'),  -- Balance 2pc Lunar Cycle
    (67126, 'spell_bracketsets_placeholder'),  -- Balance 4pc Solar Cadence
    (67121, 'spell_bracketsets_placeholder'),  -- Feral   2pc Primal Economy
    (67123, 'spell_bracketsets_placeholder'),  -- Feral   4pc Beast's Refund
    (67127, 'spell_bracketsets_placeholder'),  -- Resto   2pc Rejuvenated
    (67128, 'spell_bracketsets_placeholder'); -- Resto   4pc Wake of Bloom

-- Verification: should return 54
-- SELECT COUNT(*) FROM spell_script_names WHERE ScriptName = 'spell_bracketsets_placeholder';
