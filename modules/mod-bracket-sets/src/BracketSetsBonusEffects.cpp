#include "BracketSetsBonusEffects.h"

#include "BracketSetsConfig.h"
#include "BracketSetsRegistry.h"
#include "Chat.h"
#include "Log.h"
#include "Pet.h"
#include "Player.h"
#include "ScriptMgr.h"
#include "SpellAuraEffects.h"
#include "SpellAuras.h"
#include "SpellMgr.h"
#include "SpellScript.h"

// ============================================================================
// mod-bracket-sets — Bonus Effect Scripts (Step 6 framework)
// ============================================================================
//
// AzerothCore binds SpellScripts to spell IDs via the `spell_script_names`
// DB table, mapping (spell_id, ScriptName) to a registered class. Our 54
// marker spell IDs (see data/sql/world/2026_05_13_01_bracket_set_bonus_map_seed.sql)
// each need a row in spell_script_names pointing to a script class registered
// here.
//
// This file ships TWO classes:
//
//   1. spell_bracketsets_placeholder
//      Shared no-op script. Logs apply/remove and surfaces a chat
//      notification to the wearer ("|cff33ff99Set bonus: <name>|r" on apply,
//      "Set bonus lost: <name>" on remove). All 54 marker spell IDs are
//      bound to this script via SQL pack 04.
//      Net effect: the set-bonus system is end-to-end VISIBLE to the player
//      (you see the buff icon AND a chat message). But the bonus has NO
//      gameplay mechanic — it's a placeholder pending per-bonus
//      implementation.
//
//   2. Per-bonus subclasses (NONE in v1 — added in future PRs)
//      Each future PR replaces one or more of the spell_script_names rows
//      to point at a bespoke class implementing the actual mechanic.
//
// ============================================================================
// Catalog redesign note (open issue, captured in kb_*)
// ============================================================================
//
// Many bonuses in the authored catalog (see implementation plan or
// bracket_set_bonus_map.display_name column) reference abilities that don't
// unlock until level 40-60+:
//   - Mortal Strike (Warrior Arms talent, level 40)
//   - Bloodthirst (Fury talent, level 40)
//   - Mangle (Druid Feral talent, level 50)
//   - Stormstrike (Shaman Enh talent, level 40)
//   - Crusader Strike (Paladin Ret talent, level 50 in WotLK)
//   - Pyroblast (Mage Fire talent, level 50)
//   - Conflagrate (Warlock Destro talent, level 40)
//   - etc.
//
// Bracket 1 is level 25-34. Players don't have these abilities at this
// bracket, so the original-catalog bonuses can't fire. The catalog needs a
// pass before per-bonus mechanics are implemented:
//   - Replace high-level-ability triggers with L1-25 abilities (Heroic
//     Strike, Sinister Strike, Frostbolt, Renew, Smite, Lightning Bolt,
//     Power Word: Shield, Wrath, etc.)
//   - OR design simpler passive bonuses that don't depend on specific
//     abilities (e.g., "+X% spell crit while in stance Y").
//
// For Bracket 2 (level 35-44) and beyond, the original catalog mostly
// works since L40 abilities unlock.
//
// ============================================================================

namespace
{
    // -----------------------------------------------------------------------
    // Shared placeholder script: logs + chat notification, no mechanic.
    // -----------------------------------------------------------------------
    class spell_bracketsets_placeholder : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_placeholder);

        void HandleApply(AuraEffect const* aurEff, AuraEffectHandleModes /*mode*/)
        {
            if (!BracketSets::GetConfig().Enabled)
                return;

            uint32 spell_id = aurEff->GetSpellInfo() ? aurEff->GetSpellInfo()->Id : 0;
            Unit* target = GetTarget();
            if (!target)
                return;

            std::string name = BracketSets::LookupDisplayNameBySpell(spell_id);
            if (name.empty())
                name = "(unknown)";

            LOG_DEBUG("module",
                "[mod-bracket-sets] bonus applied: spell={} target_guid={} name='{}'",
                spell_id, target->GetGUID().GetCounter(), name);

            // Surface a chat message to the wearer so they know a bonus is live.
            if (Player* player = target->ToPlayer())
            {
                ChatHandler(player->GetSession()).PSendSysMessage(
                    "|cff33ff99Set bonus active: {}|r", name);
            }
        }

        void HandleRemove(AuraEffect const* aurEff, AuraEffectHandleModes /*mode*/)
        {
            if (!BracketSets::GetConfig().Enabled)
                return;

            uint32 spell_id = aurEff->GetSpellInfo() ? aurEff->GetSpellInfo()->Id : 0;
            Unit* target = GetTarget();
            if (!target)
                return;

            std::string name = BracketSets::LookupDisplayNameBySpell(spell_id);
            if (name.empty())
                name = "(unknown)";

            LOG_DEBUG("module",
                "[mod-bracket-sets] bonus removed: spell={} target_guid={} name='{}'",
                spell_id, target->GetGUID().GetCounter(), name);

            if (Player* player = target->ToPlayer())
            {
                ChatHandler(player->GetSession()).PSendSysMessage(
                    "|cffff7733Set bonus lost: {}|r", name);
            }
        }

        void Register() override
        {
            // The 54 marker spell IDs span multiple aura types (DUMMY,
            // PROC_TRIGGER_SPELL, OVERRIDE_CLASS_SCRIPTS, MOD_DAMAGE_PERCENT_DONE,
            // etc.). Registering for EFFECT_FIRST_FOUND + SPELL_AURA_ANY ensures
            // our hooks fire regardless of the marker's underlying aura type.
            // We use AURA_EFFECT_HANDLE_REAL so we only fire on real (not
            // reapply) apply events.
            OnEffectApply  += AuraEffectApplyFn(
                spell_bracketsets_placeholder::HandleApply,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
            OnEffectRemove += AuraEffectRemoveFn(
                spell_bracketsets_placeholder::HandleRemove,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
        }
    };
}

// ============================================================================
// Per-bonus implementations (batch 1: passive damage modifiers via target-spell hook)
// ============================================================================
//
// Pattern: the bonus AuraScript is bound to the TARGETED spell (Rend,
// Corruption, Holy Fire — not to our marker). In CalculateAmount, it checks
// whether the caster has our marker aura, and if so, increases the periodic
// damage amount by the configured percent. AC's spell_script_names supports
// multiple bindings per spell ID, so our script coexists with the engine's
// default Rend/Corruption/Holy Fire logic.
//
// Net behavior:
//   1. Player wears 2 Warrior Arms set pieces in Bracket 1.
//   2. Manager::ReevalAll calls player->AddAura(64938, player). Placeholder
//      script fires "Set bonus active: Sundered Resolve" chat message.
//   3. Player casts Rend. AC instantiates spell_warr_rend AND
//      spell_bracketsets_warrior_arms_2pc_rend for the resulting aura.
//   4. spell_bracketsets_warrior_arms_2pc_rend's CalculateBonus checks
//      caster->HasAura(64938) — true — multiplies the periodic damage
//      amount by 1.20.
//   5. Rend ticks for +20% damage for its duration.
//   6. Marker removed (unequip/level out) -> next Rend cast applies no
//      bonus -> existing Rend auras continue at their unboosted amount.
//
// SQL binding: data/sql/world/2026_05_13_05_bonus_implementations.sql adds
// rows binding each Rend/Corruption/Holy Fire spell-rank ID to the
// corresponding script class.

namespace
{
    // -----------------------------------------------------------------------
    // Warrior Arms 2pc — Sundered Resolve: Rend damage +20%
    // -----------------------------------------------------------------------
    class spell_bracketsets_warrior_arms_2pc_rend : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warrior_arms_2pc_rend);

        static constexpr uint32 MARKER_SPELL_ID = 64938; // Warrior Arms 2pc marker
        static constexpr float  BONUS_PCT       = 20.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_warrior_arms_2pc_rend::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Warlock Affliction 2pc — Corrupted Flesh: Corruption damage +15%
    // -----------------------------------------------------------------------
    class spell_bracketsets_warlock_aff_2pc_corruption : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warlock_aff_2pc_corruption);

        static constexpr uint32 MARKER_SPELL_ID = 64931; // Warlock Aff 2pc marker
        static constexpr float  BONUS_PCT       = 15.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_warlock_aff_2pc_corruption::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Priest Holy 4pc — Fire of the Fold: Holy Fire periodic damage +25%
    // -----------------------------------------------------------------------
    class spell_bracketsets_priest_holy_4pc_holy_fire : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_priest_holy_4pc_holy_fire);

        static constexpr uint32 MARKER_SPELL_ID = 64912; // Priest Holy 4pc marker
        static constexpr float  BONUS_PCT       = 25.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            // Holy Fire DBC layout: EFFECT_0 = direct SCHOOL_DAMAGE (effect 2,
            // no aura), EFFECT_1 = PERIODIC_DAMAGE (1s tick). We boost only
            // the DoT here -- the instant hit is unmodified for V1.
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_priest_holy_4pc_holy_fire::CalculateBonus,
                EFFECT_1, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // ========================================================================
    // Batch 2: 3 more periodic damage modifiers + 1 instant damage modifier
    // ========================================================================

    // -----------------------------------------------------------------------
    // Hunter Survival 2pc — Venomous Tide: Serpent Sting damage +15%
    // -----------------------------------------------------------------------
    class spell_bracketsets_hunter_surv_2pc_serpent_sting : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_hunter_surv_2pc_serpent_sting);

        static constexpr uint32 MARKER_SPELL_ID = 70727; // Hunter Surv 2pc marker
        static constexpr float  BONUS_PCT       = 15.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_hunter_surv_2pc_serpent_sting::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Rogue Assassination 2pc — Vile Toxins: Garrote damage +20%
    // -----------------------------------------------------------------------
    class spell_bracketsets_rogue_assn_2pc_garrote : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_rogue_assn_2pc_garrote);

        static constexpr uint32 MARKER_SPELL_ID = 64914; // Rogue Assn 2pc marker
        static constexpr float  BONUS_PCT       = 20.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_rogue_assn_2pc_garrote::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Mage Fire 2pc — Incandescent Burn: Ignite periodic damage +25%
    // -----------------------------------------------------------------------
    // Note: Ignite is a Fire-talent passive (L11+ talent investment). When
    // talented, Fireball/Scorch crits proc spell 12654 (Ignite periodic).
    // Untalented Fire mages don't trigger this — the bonus is dormant, which
    // is acceptable: a Bracket-1 Fire mage is expected to have Ignite (it's
    // the first 5-point Fire talent and Fire spec's defining mechanic).
    class spell_bracketsets_mage_fire_2pc_ignite : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_mage_fire_2pc_ignite);

        static constexpr uint32 MARKER_SPELL_ID = 67164; // Mage Fire 2pc marker
        static constexpr float  BONUS_PCT       = 25.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_mage_fire_2pc_ignite::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // ========================================================================
    // Batch 2 pattern B: instant damage modifier via SpellScript OnHit
    // ========================================================================
    //
    // For instant-damage abilities (Cleave, Seal procs, Heroic Strike, etc.),
    // periodic-damage CalcAmount doesn't apply. Instead, bind a SpellScript
    // to the target spell and modify GetHitDamage / SetHitDamage in OnHit.
    // Mirrors AC's spell_warr_heroic_strike pattern.

    // -----------------------------------------------------------------------
    // Warrior Fury 2pc — Berserker's Cadence: Cleave damage +15%
    // -----------------------------------------------------------------------
    class spell_bracketsets_warrior_fury_2pc_cleave : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_warrior_fury_2pc_cleave);

        static constexpr uint32 MARKER_SPELL_ID = 67234; // Warrior Fury 2pc marker
        static constexpr float  BONUS_PCT       = 15.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_warrior_fury_2pc_cleave::HandleOnHit);
        }
    };

    // ========================================================================
    // Batch 3: 2 more pattern-B + 3 new patterns (absorb, periodic-heal, conditional)
    // ========================================================================

    // -----------------------------------------------------------------------
    // Shaman Elemental 4pc — Lightning's Reach: Lightning Bolt damage +5%
    // -----------------------------------------------------------------------
    // Refit also calls for "+5 yards range" but server-side range modifier
    // requires DBC or class-aura work — deferred. The +5% damage half is
    // implemented here.
    class spell_bracketsets_shaman_ele_4pc_lightning_bolt : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_ele_4pc_lightning_bolt);

        static constexpr uint32 MARKER_SPELL_ID = 64925; // Sham Ele 4pc marker
        static constexpr float  BONUS_PCT       = 5.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_shaman_ele_4pc_lightning_bolt::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Hunter Survival 4pc — Detonation: Immolation Trap initial damage +25%
    // -----------------------------------------------------------------------
    // Bound to the trap-effect damage spell (not the trap-creation spell).
    class spell_bracketsets_hunter_surv_4pc_immolation_trap : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_hunter_surv_4pc_immolation_trap);

        static constexpr uint32 MARKER_SPELL_ID = 70730; // Hunter Surv 4pc marker
        static constexpr float  BONUS_PCT       = 25.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_hunter_surv_4pc_immolation_trap::HandleOnHit);
        }
    };

    // ========================================================================
    // Pattern D — Absorb amount modifier (DoEffectCalcAmount + SCHOOL_ABSORB)
    // ========================================================================
    // Variant of pattern A — same AuraEffectCalcAmountFn hook, different
    // aura type (SPELL_AURA_SCHOOL_ABSORB). AC's spell_pri_power_word_shield_aura
    // uses this exact hook to set the base absorb amount; our script runs
    // alongside and additively scales the result.

    // -----------------------------------------------------------------------
    // Priest Discipline 2pc — Reinforced Shield: Power Word: Shield absorb +10%
    // -----------------------------------------------------------------------
    class spell_bracketsets_priest_disc_2pc_power_word_shield : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_priest_disc_2pc_power_word_shield);

        static constexpr uint32 MARKER_SPELL_ID = 67202; // Priest Disc 2pc marker
        static constexpr float  BONUS_PCT       = 10.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_priest_disc_2pc_power_word_shield::CalculateBonus,
                EFFECT_0, SPELL_AURA_SCHOOL_ABSORB);
        }
    };

    // ========================================================================
    // Pattern A-heal-variant — Periodic heal amount modifier
    // ========================================================================
    // Identical to pattern A but on SPELL_AURA_PERIODIC_HEAL instead of
    // SPELL_AURA_PERIODIC_DAMAGE. Used for HoT modifications.

    // -----------------------------------------------------------------------
    // Druid Restoration 2pc — Rejuvenated: Rejuvenation periodic heal +10%
    // -----------------------------------------------------------------------
    // The refit catalog originally read "first tick doubled" — simplified to
    // "+10% all ticks" for v1 (first-tick-only requires periodic-tick
    // counter state that's not needed for proof-of-life). Can be revisited
    // when stricter parity to the catalog is desired.
    class spell_bracketsets_druid_resto_2pc_rejuvenation : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_druid_resto_2pc_rejuvenation);

        static constexpr uint32 MARKER_SPELL_ID = 67127; // Druid Resto 2pc marker
        static constexpr float  BONUS_PCT       = 10.f;

        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }

        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_druid_resto_2pc_rejuvenation::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_HEAL);
        }
    };

    // ========================================================================
    // Pattern I — Conditional damage modifier (target-state-gated)
    // ========================================================================
    // SpellScript OnHit + SetHitDamage like pattern B, but the +X% only
    // applies when the target satisfies a condition (frozen, behind, low HP,
    // etc.). The condition check goes inside HandleOnHit before SetHitDamage.

    // -----------------------------------------------------------------------
    // Mage Frost 4pc — Glacial Lance: Frostbolt vs frozen target +30%
    // -----------------------------------------------------------------------
    // "Frozen" = target has AURA_STATE_FROZEN, set when any freeze/root
    // (Frost Nova, Freezing Trap, etc.) is active.
    class spell_bracketsets_mage_frost_4pc_frostbolt : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_mage_frost_4pc_frostbolt);

        static constexpr uint32 MARKER_SPELL_ID = 70748; // Mage Frost 4pc marker
        static constexpr float  BONUS_PCT       = 30.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            Unit* target = GetHitUnit();
            if (!caster || !target)
                return;
            if (!caster->HasAura(MARKER_SPELL_ID))
                return;
            if (!target->HasAuraState(AURA_STATE_FROZEN))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_mage_frost_4pc_frostbolt::HandleOnHit);
        }
    };

    // ========================================================================
    // Batch 4: 2 cooldown reducers + 2 duration extenders (new patterns F + G)
    // ========================================================================

    // ========================================================================
    // Pattern F — Cooldown reducer (SpellScript AfterCast + ModifySpellCooldown)
    // ========================================================================
    // When the target spell is cast, our AfterCast hook fires. If the caster
    // has the marker, we call Player::ModifySpellCooldown(spell_id, delta)
    // with a negative delta to shorten the just-applied cooldown.

    // -----------------------------------------------------------------------
    // Mage Fire 4pc — Pyroblast Surge: Fire Blast cooldown -1s
    // -----------------------------------------------------------------------
    class spell_bracketsets_mage_fire_4pc_fire_blast : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_mage_fire_4pc_fire_blast);

        static constexpr uint32 MARKER_SPELL_ID  = 67185;   // Mage Fire 4pc marker
        static constexpr int32  COOLDOWN_DELTA_MS = -1000;  // -1 second

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            player->ModifySpellCooldown(GetSpellInfo()->Id, COOLDOWN_DELTA_MS);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_mage_fire_4pc_fire_blast::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Rogue Subtlety 4pc — Stepping Shadow: Vanish cooldown -30s
    // -----------------------------------------------------------------------
    class spell_bracketsets_rogue_sub_4pc_vanish : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_rogue_sub_4pc_vanish);

        static constexpr uint32 MARKER_SPELL_ID  = 67187;   // Rogue Sub 4pc marker
        static constexpr int32  COOLDOWN_DELTA_MS = -30000; // -30 seconds

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            player->ModifySpellCooldown(GetSpellInfo()->Id, COOLDOWN_DELTA_MS);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_rogue_sub_4pc_vanish::HandleAfterCast);
        }
    };

    // ========================================================================
    // Pattern G — Duration extender (AuraScript OnEffectApply + SetMaxDuration)
    // ========================================================================
    // Hook the target aura's apply. If caster has the marker, extend the
    // aura's max duration by the configured ms and RefreshDuration so the
    // engine respects the new cap.

    // -----------------------------------------------------------------------
    // Warlock Destruction 4pc — Lingering Flame: Immolate duration +6s
    // -----------------------------------------------------------------------
    class spell_bracketsets_warlock_destro_4pc_immolate : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warlock_destro_4pc_immolate);

        static constexpr uint32 MARKER_SPELL_ID       = 70841;
        static constexpr int32  DURATION_EXTENSION_MS = 6000; // +6 seconds

        void HandleApply(AuraEffect const* /*aurEff*/, AuraEffectHandleModes /*mode*/)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            SetMaxDuration(GetMaxDuration() + DURATION_EXTENSION_MS);
            RefreshDuration();
        }

        void Register() override
        {
            // Immolate's aura is APPLY_AURA / PERIODIC_DAMAGE on EFFECT_0.
            OnEffectApply += AuraEffectApplyFn(
                spell_bracketsets_warlock_destro_4pc_immolate::HandleApply,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE, AURA_EFFECT_HANDLE_REAL);
        }
    };

    // -----------------------------------------------------------------------
    // Mage Frost 2pc — Frozen Tide: Frost Nova freeze duration +1s
    // -----------------------------------------------------------------------
    // Frost Nova applies SPELL_AURA_MOD_ROOT on EFFECT_1 (EFFECT_0 is direct
    // frost damage, no aura). Hook the root-effect apply.
    class spell_bracketsets_mage_frost_2pc_frost_nova : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_mage_frost_2pc_frost_nova);

        static constexpr uint32 MARKER_SPELL_ID       = 67189;
        static constexpr int32  DURATION_EXTENSION_MS = 1000; // +1 second

        void HandleApply(AuraEffect const* /*aurEff*/, AuraEffectHandleModes /*mode*/)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            SetMaxDuration(GetMaxDuration() + DURATION_EXTENSION_MS);
            RefreshDuration();
        }

        void Register() override
        {
            OnEffectApply += AuraEffectApplyFn(
                spell_bracketsets_mage_frost_2pc_frost_nova::HandleApply,
                EFFECT_1, SPELL_AURA_MOD_ROOT, AURA_EFFECT_HANDLE_REAL);
        }
    };

    // ========================================================================
    // Batch 5: 1 proc refund (pattern H) + 2 pet stat mods (pattern J)
    // ========================================================================

    // ========================================================================
    // Pattern H — Proc refund (SpellScript OnHit + roll_chance + ModifyPower)
    // ========================================================================
    // SpellScript bound to the target ability. OnHit checks the marker, rolls
    // a chance, and energizes the player on success. Lighter than AuraScript
    // OnProc because we bypass the marker's DBC-fixed procEx flags (which were
    // designed for the original retail bonus, not ours).

    // -----------------------------------------------------------------------
    // Rogue Combat 2pc — Sinister Edge: Sinister Strike 10% chance +1 energy
    // -----------------------------------------------------------------------
    class spell_bracketsets_rogue_combat_2pc_sinister_strike : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_rogue_combat_2pc_sinister_strike);

        static constexpr uint32 MARKER_SPELL_ID = 67209; // Rogue Combat 2pc marker
        static constexpr int32  PROC_CHANCE_PCT = 10;
        static constexpr int32  ENERGY_REFUND   = 1;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            if (!roll_chance_i(PROC_CHANCE_PCT))
                return;
            player->ModifyPower(POWER_ENERGY, ENERGY_REFUND);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_rogue_combat_2pc_sinister_strike::HandleOnHit);
        }
    };

    // ========================================================================
    // Pattern J — Pet stat modifier (AuraScript on marker, modifies pet on apply)
    // ========================================================================
    // AuraScript bound directly to the MARKER spell (not a target). OnApply
    // gets the player's pet (if any) and modifies its stats via
    // Unit::ApplyStatPctModifier. OnRemove reverses with negative delta.
    //
    // Coexists with placeholder: both scripts bind to the marker ID, both
    // fire on apply/remove. Placeholder sends chat; this script modifies pet.
    //
    // Limitation v1: if player doesn't have a pet when marker applies, the
    // bonus is lost until a future marker re-eval. Pet-summon hook would be
    // needed for full correctness — deferred.

    // -----------------------------------------------------------------------
    // Hunter Beast Mastery 2pc — Tide-Born Beast: Pet damage +5%
    // -----------------------------------------------------------------------
    class spell_bracketsets_hunter_bm_2pc_pet_damage : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_hunter_bm_2pc_pet_damage);

        static constexpr float BONUS_PCT = 5.0f;

        void HandleApply(AuraEffect const* /*aurEff*/, AuraEffectHandleModes /*mode*/)
        {
            Player* player = GetTarget()->ToPlayer();
            if (!player)
                return;
            Pet* pet = player->GetPet();
            if (!pet)
                return;
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_MAINHAND, TOTAL_PCT, BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_OFFHAND,  TOTAL_PCT, BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_RANGED,   TOTAL_PCT, BONUS_PCT);
        }

        void HandleRemove(AuraEffect const* /*aurEff*/, AuraEffectHandleModes /*mode*/)
        {
            Player* player = GetTarget()->ToPlayer();
            if (!player)
                return;
            Pet* pet = player->GetPet();
            if (!pet)
                return;
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_MAINHAND, TOTAL_PCT, -BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_OFFHAND,  TOTAL_PCT, -BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_RANGED,   TOTAL_PCT, -BONUS_PCT);
        }

        void Register() override
        {
            OnEffectApply  += AuraEffectApplyFn(
                spell_bracketsets_hunter_bm_2pc_pet_damage::HandleApply,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
            OnEffectRemove += AuraEffectRemoveFn(
                spell_bracketsets_hunter_bm_2pc_pet_damage::HandleRemove,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
        }
    };

    // -----------------------------------------------------------------------
    // Warlock Demonology 2pc — Demonic Augmentation: Pet damage +10%
    // -----------------------------------------------------------------------
    // Same pattern as Hunter BM 2pc, different bonus percentage and marker.
    class spell_bracketsets_warlock_demo_2pc_pet_damage : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warlock_demo_2pc_pet_damage);

        static constexpr float BONUS_PCT = 10.0f;

        void HandleApply(AuraEffect const* /*aurEff*/, AuraEffectHandleModes /*mode*/)
        {
            Player* player = GetTarget()->ToPlayer();
            if (!player)
                return;
            Pet* pet = player->GetPet();
            if (!pet)
                return;
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_MAINHAND, TOTAL_PCT, BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_OFFHAND,  TOTAL_PCT, BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_RANGED,   TOTAL_PCT, BONUS_PCT);
        }

        void HandleRemove(AuraEffect const* /*aurEff*/, AuraEffectHandleModes /*mode*/)
        {
            Player* player = GetTarget()->ToPlayer();
            if (!player)
                return;
            Pet* pet = player->GetPet();
            if (!pet)
                return;
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_MAINHAND, TOTAL_PCT, -BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_OFFHAND,  TOTAL_PCT, -BONUS_PCT);
            pet->ApplyStatPctModifier(UNIT_MOD_DAMAGE_RANGED,   TOTAL_PCT, -BONUS_PCT);
        }

        void Register() override
        {
            OnEffectApply  += AuraEffectApplyFn(
                spell_bracketsets_warlock_demo_2pc_pet_damage::HandleApply,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
            OnEffectRemove += AuraEffectRemoveFn(
                spell_bracketsets_warlock_demo_2pc_pet_damage::HandleRemove,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
        }
    };

    // ========================================================================
    // Batch 6: 3 cost modifiers (pattern E, new) + 1 instant damage (pattern B)
    // ========================================================================

    // ========================================================================
    // Pattern E — Cost modifier (SpellScript AfterCast + ModifyPower refund)
    // ========================================================================
    // SpellScript bound to the target ability. AfterCast checks the marker
    // and refunds the configured amount of power. Mirrors patterns F and H
    // architecturally — same hook, different post-cast adjustment (cooldown,
    // chance-roll energize, or unconditional energize).
    //
    // Note on rage units: AC stores rage as tenths (5 rage = 50 power units).
    // The constants below use POWER UNITS, not display units. Energy and mana
    // use 1:1 units (5 energy = 5 power units).

    // -----------------------------------------------------------------------
    // Druid Feral 2pc — Primal Economy (Claw): Claw -5 energy
    // -----------------------------------------------------------------------
    // Primal Economy is one bonus that affects two abilities (Claw + Maul).
    // Two scripts, both keyed to the same marker (67121).
    class spell_bracketsets_druid_feral_2pc_claw : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_druid_feral_2pc_claw);

        static constexpr uint32 MARKER_SPELL_ID = 67121; // Druid Feral 2pc marker
        static constexpr int32  ENERGY_REFUND   = 5;

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            player->ModifyPower(POWER_ENERGY, ENERGY_REFUND);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_druid_feral_2pc_claw::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Druid Feral 2pc — Primal Economy (Maul): Maul -5 rage
    // -----------------------------------------------------------------------
    class spell_bracketsets_druid_feral_2pc_maul : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_druid_feral_2pc_maul);

        static constexpr uint32 MARKER_SPELL_ID = 67121; // Druid Feral 2pc marker (same as Claw)
        static constexpr int32  RAGE_REFUND     = 50;    // 5 rage * 10 (AC stores rage in tenths)

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            player->ModifyPower(POWER_RAGE, RAGE_REFUND);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_druid_feral_2pc_maul::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Shaman Restoration 2pc — Cascading Tide: Healing Wave mana cost -10%
    // -----------------------------------------------------------------------
    // Refund is computed as 10% of the spell's base ManaCost from SpellInfo.
    // This is approximate — the actual post-talent cost may differ — but
    // sufficient for Bracket 1 mechanic feel.
    class spell_bracketsets_shaman_resto_2pc_healing_wave : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_resto_2pc_healing_wave);

        static constexpr uint32 MARKER_SPELL_ID = 67225; // Sham Resto 2pc marker
        static constexpr int32  REFUND_PCT      = 10;

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell)
                return;
            int32 baseCost = spell->ManaCost;
            if (baseCost <= 0)
                return;
            int32 refund = CalculatePct(baseCost, REFUND_PCT);
            player->ModifyPower(POWER_MANA, refund);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_shaman_resto_2pc_healing_wave::HandleAfterCast);
        }
    };

    // ========================================================================
    // Pattern B (additional) — Pal Ret 4pc Verdict of Light: Judgement +15%
    // ========================================================================
    // The refit also calls for "+2% mana refund on crit" but that's a
    // separate compound effect — implemented in v2. The damage half lands
    // here as a clean Pattern B replication.

    // -----------------------------------------------------------------------
    // Paladin Retribution 4pc — Verdict of Light: Judgement damage +15%
    // -----------------------------------------------------------------------
    class spell_bracketsets_paladin_ret_4pc_judgement : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_paladin_ret_4pc_judgement);

        static constexpr uint32 MARKER_SPELL_ID = 64879; // Paladin Ret 4pc marker
        static constexpr float  BONUS_PCT       = 15.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_paladin_ret_4pc_judgement::HandleOnHit);
        }
    };

    // ========================================================================
    // Batch 7: 2 proc refunds + 1 conditional damage + 2 instant damage (B/H/I)
    // ========================================================================

    // -----------------------------------------------------------------------
    // Warrior Fury 4pc — Wrathful Strike: Heroic Strike 15% chance no rage
    // -----------------------------------------------------------------------
    // "No rage" implemented as refunding the rage cost after consumption.
    // SpellInfo->ManaCost holds the rage cost (AC stores rage in tenths;
    // ManaCost is in the same storage units, so refund passes through directly).
    class spell_bracketsets_warrior_fury_4pc_heroic_strike : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_warrior_fury_4pc_heroic_strike);

        static constexpr uint32 MARKER_SPELL_ID = 67268; // Warrior Fury 4pc marker
        static constexpr int32  PROC_CHANCE_PCT = 15;

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            if (!roll_chance_i(PROC_CHANCE_PCT))
                return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell)
                return;
            int32 refund = spell->ManaCost;
            if (refund > 0)
                player->ModifyPower(POWER_RAGE, refund);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_warrior_fury_4pc_heroic_strike::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Druid Feral 4pc — Beast's Refund: Maul 15% chance to refund full rage
    // -----------------------------------------------------------------------
    class spell_bracketsets_druid_feral_4pc_maul : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_druid_feral_4pc_maul);

        static constexpr uint32 MARKER_SPELL_ID = 67123; // Druid Feral 4pc marker
        static constexpr int32  PROC_CHANCE_PCT = 15;

        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster)
                return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID))
                return;
            if (!roll_chance_i(PROC_CHANCE_PCT))
                return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell)
                return;
            int32 refund = spell->ManaCost;
            if (refund > 0)
                player->ModifyPower(POWER_RAGE, refund);
        }

        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_druid_feral_4pc_maul::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Rogue Subtlety 2pc — Backstab's Bite: Backstab +10% damage when behind target
    // -----------------------------------------------------------------------
    // "Behind target" check uses Unit::isInBack which returns true when the
    // caster is within the target's rear arc.
    class spell_bracketsets_rogue_sub_2pc_backstab : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_rogue_sub_2pc_backstab);

        static constexpr uint32 MARKER_SPELL_ID = 67186;
        static constexpr float  BONUS_PCT       = 10.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            Unit* target = GetHitUnit();
            if (!caster || !target)
                return;
            if (!caster->HasAura(MARKER_SPELL_ID))
                return;
            if (!caster->isInBack(target))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_rogue_sub_2pc_backstab::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Hunter MM 2pc — Piercing Shot: Arcane Shot damage +10%
    // -----------------------------------------------------------------------
    // Simplified from refit "Arcane Shot reduces target armor 5% for 8s".
    // The armor-debuff variant needs a real DBC spell to cast on target;
    // the damage-bonus variant is functionally equivalent for Bracket 1
    // feel and uses pattern B cleanly. Documented as a v1 simplification.
    class spell_bracketsets_hunter_mm_2pc_arcane_shot : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_hunter_mm_2pc_arcane_shot);

        static constexpr uint32 MARKER_SPELL_ID = 67150;
        static constexpr float  BONUS_PCT       = 10.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_hunter_mm_2pc_arcane_shot::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Priest Shadow 2pc — Mind Crack: Mind Blast damage +15%
    // -----------------------------------------------------------------------
    // Simplified from refit "Mind Blast reduces target shadow resistance
    // 10 for 8s". The resistance-debuff variant needs a DBC spell to cast
    // on target; the damage-bonus is the v1 simplification.
    class spell_bracketsets_priest_shadow_2pc_mind_blast : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_priest_shadow_2pc_mind_blast);

        static constexpr uint32 MARKER_SPELL_ID = 67193;
        static constexpr float  BONUS_PCT       = 15.f;

        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID))
                return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }

        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_priest_shadow_2pc_mind_blast::HandleOnHit);
        }
    };

    // ========================================================================
    // Batch 8: 5 bonuses across patterns J-variant, L, D-variant, K-variant
    // ========================================================================

    // -----------------------------------------------------------------------
    // Hunter BM 4pc — Primal Frenzy: Pet max HP +10%
    // -----------------------------------------------------------------------
    // Simplified from refit "Pet out-of-combat HP regen +50%". Out-of-combat
    // regen rate isn't a directly modifiable stat in AC; flat HP boost
    // delivers the equivalent "tankier pet" feel. Pattern J-style apply/
    // remove on marker.
    class spell_bracketsets_hunter_bm_4pc_pet_hp : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_hunter_bm_4pc_pet_hp);
        static constexpr float BONUS_PCT = 10.0f;

        void HandleApply(AuraEffect const*, AuraEffectHandleModes)
        {
            Player* p = GetTarget()->ToPlayer();
            if (!p) return;
            Pet* pet = p->GetPet();
            if (!pet) return;
            pet->ApplyStatPctModifier(UNIT_MOD_HEALTH, TOTAL_PCT, BONUS_PCT);
        }
        void HandleRemove(AuraEffect const*, AuraEffectHandleModes)
        {
            Player* p = GetTarget()->ToPlayer();
            if (!p) return;
            Pet* pet = p->GetPet();
            if (!pet) return;
            pet->ApplyStatPctModifier(UNIT_MOD_HEALTH, TOTAL_PCT, -BONUS_PCT);
        }
        void Register() override
        {
            OnEffectApply  += AuraEffectApplyFn(spell_bracketsets_hunter_bm_4pc_pet_hp::HandleApply,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
            OnEffectRemove += AuraEffectRemoveFn(spell_bracketsets_hunter_bm_4pc_pet_hp::HandleRemove,
                EFFECT_FIRST_FOUND, SPELL_AURA_ANY, AURA_EFFECT_HANDLE_REAL);
        }
    };

    // -----------------------------------------------------------------------
    // Paladin Prot 2pc — Righteous Provocation: Hand of Reckoning damage +50%
    // -----------------------------------------------------------------------
    // Simplified from refit "Hand of Reckoning +50% threat". Damage and
    // threat scale together in WotLK; +50% damage delivers +50% threat as
    // a side effect. Pure-threat hook isn't exposed in AC SpellScript.
    class spell_bracketsets_paladin_prot_2pc_hand_of_reckoning : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_paladin_prot_2pc_hand_of_reckoning);
        static constexpr uint32 MARKER_SPELL_ID = 64881;
        static constexpr float  BONUS_PCT       = 50.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_paladin_prot_2pc_hand_of_reckoning::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Paladin Prot 4pc — Sanctified Ground: Consecration tick damage +20%
    // -----------------------------------------------------------------------
    // V1 redesign (was "Blessing of Sanctuary +2% DR"): WotLK 3.3.5a routes
    // BoS damage reduction through AC's special-case dummy aura code path,
    // so DoEffectCalcAmount on the BoS aura has nothing to hook. Switched
    // to Pattern A on Consecration, which is the iconic Prot AoE for the
    // L25-34 bracket and has a clean PERIODIC_DAMAGE effect on EFFECT_0
    // (Persistent Area Aura, 1s tick) that Pattern A binds to natively.
    class spell_bracketsets_paladin_prot_4pc_consecration : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_paladin_prot_4pc_consecration);
        static constexpr uint32 MARKER_SPELL_ID = 64882;
        static constexpr float  BONUS_PCT       = 20.f;
        void CalculateBonus(AuraEffect const* /*aurEff*/, int32& amount, bool& /*canBeRecalculated*/)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    AddPct(amount, BONUS_PCT);
        }
        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_paladin_prot_4pc_consecration::CalculateBonus,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Warrior Prot 2pc — Bulwark Surge: Shield Block block value +5%
    // -----------------------------------------------------------------------
    // Shield Block (2565) grants +block chance and +block value while active.
    // Hook the block-value-percent effect via DoEffectCalcAmount.
    class spell_bracketsets_warrior_prot_2pc_shield_block : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warrior_prot_2pc_shield_block);
        static constexpr uint32 MARKER_SPELL_ID = 64933;
        static constexpr int32  BONUS_FLAT      = 5; // +5% block value
        void CalcAmount(AuraEffect const*, int32& amount, bool&)
        {
            if (Unit* caster = GetCaster())
                if (caster->HasAura(MARKER_SPELL_ID))
                    amount += BONUS_FLAT;
        }
        void Register() override
        {
            // Effect index for SHIELD_BLOCKVALUE_PCT varies by source; bind
            // both common indices. EFFECT_FIRST_FOUND covers the common case.
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_warrior_prot_2pc_shield_block::CalcAmount,
                EFFECT_FIRST_FOUND, SPELL_AURA_MOD_SHIELD_BLOCKVALUE_PCT);
        }
    };

    // -----------------------------------------------------------------------
    // Mage Arcane 2pc — Final Missile: Arcane Missile damage +15%
    // -----------------------------------------------------------------------
    // Simplified from refit "Arcane Missiles last tick +30% crit chance".
    // The per-tick crit-chance variant requires tracking tick number on
    // the channel; pattern B on the missile damage spell (7268) delivers
    // similar net DPS uplift. Bound to Arcane Missile damage ranks.
    class spell_bracketsets_mage_arc_2pc_arcane_missile : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_mage_arc_2pc_arcane_missile);
        static constexpr uint32 MARKER_SPELL_ID = 64867;
        static constexpr float  BONUS_PCT       = 15.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_mage_arc_2pc_arcane_missile::HandleOnHit);
        }
    };

    // ========================================================================
    // Batch 9: 5 bonuses focused on periodic-tick mechanics + crit-as-double
    // ========================================================================

    // Pattern K (new) — Crit-as-double on periodic damage/heal.
    // No direct crit-chance hook in AC SpellScript; emulate via random
    // amount doubling in DoEffectCalcAmount.

    // -----------------------------------------------------------------------
    // Druid Balance 2pc — Lunar Cycle: Moonfire periodic +15% crit chance
    // -----------------------------------------------------------------------
    class spell_bracketsets_druid_balance_2pc_moonfire : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_druid_balance_2pc_moonfire);
        static constexpr uint32 MARKER_SPELL_ID = 67125;
        static constexpr int32  CRIT_BONUS_PCT  = 15; // additional crit chance
        void CalcAmount(AuraEffect const*, int32& amount, bool&)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            if (roll_chance_i(CRIT_BONUS_PCT))
                amount *= 2; // emulate crit
        }
        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_druid_balance_2pc_moonfire::CalcAmount,
                EFFECT_FIRST_FOUND, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Priest Holy 2pc — Renewed Light: Renew tick 15% chance to be doubled
    // -----------------------------------------------------------------------
    class spell_bracketsets_priest_holy_2pc_renew : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_priest_holy_2pc_renew);
        static constexpr uint32 MARKER_SPELL_ID = 64910;
        static constexpr int32  PROC_CHANCE_PCT = 15;
        void CalcAmount(AuraEffect const*, int32& amount, bool&)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            if (roll_chance_i(PROC_CHANCE_PCT))
                amount *= 2;
        }
        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_priest_holy_2pc_renew::CalcAmount,
                EFFECT_FIRST_FOUND, SPELL_AURA_PERIODIC_HEAL);
        }
    };

    // Pattern N (new) — On-tick effect via OnEffectPeriodic
    // Each periodic tick fires a side effect (heal caster, refund resource).

    // -----------------------------------------------------------------------
    // Priest Shadow 4pc — Vampire's Renewal: SW:Pain ticks heal caster 1% maxHP
    // -----------------------------------------------------------------------
    class spell_bracketsets_priest_shadow_4pc_sw_pain : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_priest_shadow_4pc_sw_pain);
        static constexpr uint32 MARKER_SPELL_ID = 67198;
        static constexpr int32  HEAL_PCT_MAX_HP = 1;
        void HandlePeriodic(AuraEffect const* /*aurEff*/)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 heal = CalculatePct(static_cast<int32>(caster->GetMaxHealth()), HEAL_PCT_MAX_HP);
            if (heal > 0)
                caster->ModifyHealth(heal);
        }
        void Register() override
        {
            OnEffectPeriodic += AuraEffectPeriodicFn(
                spell_bracketsets_priest_shadow_4pc_sw_pain::HandlePeriodic,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Rogue Assassination 4pc — Envenom's Wake: Rupture tick 10%/tick chance to refund 1 energy
    // -----------------------------------------------------------------------
    class spell_bracketsets_rogue_assn_4pc_rupture : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_rogue_assn_4pc_rupture);
        static constexpr uint32 MARKER_SPELL_ID = 64915;
        static constexpr int32  PROC_CHANCE_PCT = 10;
        static constexpr int32  ENERGY_REFUND   = 1;
        void HandlePeriodic(AuraEffect const* /*aurEff*/)
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            if (!roll_chance_i(PROC_CHANCE_PCT)) return;
            player->ModifyPower(POWER_ENERGY, ENERGY_REFUND);
        }
        void Register() override
        {
            OnEffectPeriodic += AuraEffectPeriodicFn(
                spell_bracketsets_rogue_assn_4pc_rupture::HandlePeriodic,
                EFFECT_0, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Warlock Demonology 4pc — Soulbound: Health Funnel transfers +25%
    // -----------------------------------------------------------------------
    // Health Funnel periodically heals the pet at the cost of caster HP.
    // Hook the periodic-heal effect's calc amount, scale by +25% if marker
    // is present on the caster.
    class spell_bracketsets_warlock_demo_4pc_health_funnel : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warlock_demo_4pc_health_funnel);
        static constexpr uint32 MARKER_SPELL_ID = 67230;
        static constexpr float  BONUS_PCT       = 25.f;
        void CalcAmount(AuraEffect const*, int32& amount, bool&)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            AddPct(amount, BONUS_PCT);
        }
        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_warlock_demo_4pc_health_funnel::CalcAmount,
                EFFECT_FIRST_FOUND, SPELL_AURA_PERIODIC_HEAL);
        }
    };

    // ========================================================================
    // Batch 10: 3 compound proc chains (Pattern M, new) + 2 simplifications
    // ========================================================================

    // Pattern M (new) — compound "after X cast, next Y modified" chains.
    // Two scripts per bonus:
    //   - Trigger script: AfterCast on the trigger spell, applies an
    //     intermediate aura (chosen from spare DBC marker IDs) for a short
    //     duration via Aura::SetDuration.
    //   - Consumer script: AfterCast/OnHit on the target spell, checks for
    //     intermediate aura, applies the modifier, removes the aura so the
    //     buff is single-use.

    // -----------------------------------------------------------------------
    // Warrior Prot 4pc — Devastating Cycle: after Revenge, Thunder Clap -50% rage
    // -----------------------------------------------------------------------
    static constexpr uint32 DEVASTATING_CYCLE_INTERMEDIATE = 64752;
    class spell_bracketsets_warrior_prot_4pc_revenge : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_warrior_prot_4pc_revenge);
        static constexpr uint32 MARKER_SPELL_ID = 64936;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            if (Aura* a = caster->AddAura(DEVASTATING_CYCLE_INTERMEDIATE, caster))
            {
                a->SetMaxDuration(5000);
                a->SetDuration(5000);
            }
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_warrior_prot_4pc_revenge::HandleAfterCast);
        }
    };
    class spell_bracketsets_warrior_prot_4pc_thunder_clap : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_warrior_prot_4pc_thunder_clap);
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(DEVASTATING_CYCLE_INTERMEDIATE)) return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell) return;
            int32 refund = spell->ManaCost / 2;
            if (refund > 0) player->ModifyPower(POWER_RAGE, refund);
            caster->RemoveAurasDueToSpell(DEVASTATING_CYCLE_INTERMEDIATE);
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_warrior_prot_4pc_thunder_clap::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Mage Arcane 4pc — Sustained Power: after Arcane Explosion, next Arc Missile +10%
    // -----------------------------------------------------------------------
    static constexpr uint32 SUSTAINED_POWER_INTERMEDIATE = 64760;
    class spell_bracketsets_mage_arc_4pc_arcane_explosion : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_mage_arc_4pc_arcane_explosion);
        static constexpr uint32 MARKER_SPELL_ID = 67188;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            if (Aura* a = caster->AddAura(SUSTAINED_POWER_INTERMEDIATE, caster))
            {
                a->SetMaxDuration(8000);
                a->SetDuration(8000);
            }
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_mage_arc_4pc_arcane_explosion::HandleAfterCast);
        }
    };
    class spell_bracketsets_mage_arc_4pc_arc_missile_bonus : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_mage_arc_4pc_arc_missile_bonus);
        static constexpr float BONUS_PCT = 10.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(SUSTAINED_POWER_INTERMEDIATE)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
            // Don't consume — let entire 5-missile channel benefit during the 8s window.
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_mage_arc_4pc_arc_missile_bonus::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Shaman Resto 4pc — Quick Mend: after LHW, next HW heals +10%
    // -----------------------------------------------------------------------
    // Simplified from refit "-0.5s cast time" — cast-time modification on
    // a per-spell basis needs SPELLMOD_CASTING_TIME aura which we can't
    // easily inject. Healing boost delivers similar feel.
    static constexpr uint32 QUICK_MEND_INTERMEDIATE = 64818;
    class spell_bracketsets_shaman_resto_4pc_lhw : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_resto_4pc_lhw);
        static constexpr uint32 MARKER_SPELL_ID = 67226;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            if (Aura* a = caster->AddAura(QUICK_MEND_INTERMEDIATE, caster))
            {
                a->SetMaxDuration(5000);
                a->SetDuration(5000);
            }
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_shaman_resto_4pc_lhw::HandleAfterCast);
        }
    };
    class spell_bracketsets_shaman_resto_4pc_hw : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_resto_4pc_hw);
        static constexpr float BONUS_PCT = 10.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(QUICK_MEND_INTERMEDIATE)) return;
            int32 heal = GetHitHeal();
            AddPct(heal, BONUS_PCT);
            SetHitHeal(heal);
            caster->RemoveAurasDueToSpell(QUICK_MEND_INTERMEDIATE);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_shaman_resto_4pc_hw::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Paladin Holy 2pc — Light's Compact: Flash of Light mana cost -10%
    // -----------------------------------------------------------------------
    // Simplified from refit "FoL 10% chance instant cast" — cast-time
    // modification requires SPELLMOD_CASTING_TIME aura that's hard to inject
    // mid-engine. Mana reduction (Pattern E) delivers a similar "easier to
    // cast" feel.
    class spell_bracketsets_paladin_holy_2pc_flash_of_light : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_paladin_holy_2pc_flash_of_light);
        static constexpr uint32 MARKER_SPELL_ID = 70755;
        static constexpr int32  REFUND_PCT      = 10;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell) return;
            int32 refund = CalculatePct(spell->ManaCost, REFUND_PCT);
            if (refund > 0) player->ModifyPower(POWER_MANA, refund);
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_paladin_holy_2pc_flash_of_light::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Druid Balance 4pc — Solar Cadence: Wrath 10% chance refund full mana cost
    // -----------------------------------------------------------------------
    // Simplified from refit "Wrath grants Nature's Grace" — the Clearcasting/
    // Nature's Grace cast-time mechanic needs aura injection. Mana refund
    // (Pattern H) is the v1 simplification.
    class spell_bracketsets_druid_balance_4pc_wrath : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_druid_balance_4pc_wrath);
        static constexpr uint32 MARKER_SPELL_ID = 67126;
        static constexpr int32  PROC_CHANCE_PCT = 10;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            if (!roll_chance_i(PROC_CHANCE_PCT)) return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell) return;
            int32 refund = spell->ManaCost;
            if (refund > 0) player->ModifyPower(POWER_MANA, refund);
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_druid_balance_4pc_wrath::HandleAfterCast);
        }
    };

    // ========================================================================
    // Batch 11: 6 simplification replications using patterns H, B, G, E
    // ========================================================================
    // These take the catalog's crit-based compound chains and downgrade to
    // chance-based proc-on-cast (Pattern H) since crit-detection in SpellScript
    // isn't cleanly exposed. Functionally similar power budget. Documented
    // per-bonus as v1 simplifications.

    // -----------------------------------------------------------------------
    // Warrior Arms 4pc Overpowering Strike — Overpower 30% chance refund rage
    // (simplified from "after crit, next Overpower no rage")
    // -----------------------------------------------------------------------
    class spell_bracketsets_warrior_arms_4pc_overpower : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_warrior_arms_4pc_overpower);
        static constexpr uint32 MARKER_SPELL_ID = 64939;
        static constexpr int32  PROC_CHANCE_PCT = 30;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            if (!roll_chance_i(PROC_CHANCE_PCT)) return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell) return;
            int32 refund = spell->ManaCost;
            if (refund > 0) player->ModifyPower(POWER_RAGE, refund);
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_warrior_arms_4pc_overpower::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Hunter MM 4pc Arcane Cadence — Multi-Shot damage +15%
    // (simplified from "Auto Shot crit -> next Arcane Shot guaranteed crit")
    // -----------------------------------------------------------------------
    class spell_bracketsets_hunter_mm_4pc_multi_shot : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_hunter_mm_4pc_multi_shot);
        static constexpr uint32 MARKER_SPELL_ID = 70724;
        static constexpr float  BONUS_PCT       = 15.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_hunter_mm_4pc_multi_shot::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Priest Disc 4pc Suppression's Echo — PW:Shield duration +2s
    // (simplified from "after PW:Shield, target healing received +5% for 8s")
    // -----------------------------------------------------------------------
    class spell_bracketsets_priest_disc_4pc_pw_shield_duration : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_priest_disc_4pc_pw_shield_duration);
        static constexpr uint32 MARKER_SPELL_ID = 70798;
        static constexpr int32  DURATION_EXTENSION_MS = 2000;
        void HandleApply(AuraEffect const*, AuraEffectHandleModes)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            SetMaxDuration(GetMaxDuration() + DURATION_EXTENSION_MS);
            RefreshDuration();
        }
        void Register() override
        {
            OnEffectApply += AuraEffectApplyFn(
                spell_bracketsets_priest_disc_4pc_pw_shield_duration::HandleApply,
                EFFECT_0, SPELL_AURA_SCHOOL_ABSORB, AURA_EFFECT_HANDLE_REAL);
        }
    };

    // -----------------------------------------------------------------------
    // Shaman Ele 2pc Stormcaller's Insight — Lightning Bolt mana cost -5%
    // (simplified from "LB crits grant +2% haste 6s")
    // -----------------------------------------------------------------------
    class spell_bracketsets_shaman_ele_2pc_lightning_bolt : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_ele_2pc_lightning_bolt);
        static constexpr uint32 MARKER_SPELL_ID = 67227;
        static constexpr int32  REFUND_PCT      = 5;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell) return;
            int32 refund = CalculatePct(spell->ManaCost, REFUND_PCT);
            if (refund > 0) player->ModifyPower(POWER_MANA, refund);
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_shaman_ele_2pc_lightning_bolt::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Warlock Destro 2pc Shadow Cadence — Shadow Bolt 10% chance refund 50% mana
    // (simplified from "SB crit -> next SB -0.5s cast")
    // -----------------------------------------------------------------------
    class spell_bracketsets_warlock_destro_2pc_shadow_bolt : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_warlock_destro_2pc_shadow_bolt);
        static constexpr uint32 MARKER_SPELL_ID = 70839;
        static constexpr int32  PROC_CHANCE_PCT = 10;
        static constexpr int32  REFUND_PCT      = 50;
        void HandleAfterCast()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            if (!roll_chance_i(PROC_CHANCE_PCT)) return;
            SpellInfo const* spell = GetSpellInfo();
            if (!spell) return;
            int32 refund = CalculatePct(spell->ManaCost, REFUND_PCT);
            if (refund > 0) player->ModifyPower(POWER_MANA, refund);
        }
        void Register() override
        {
            AfterCast += SpellCastFn(spell_bracketsets_warlock_destro_2pc_shadow_bolt::HandleAfterCast);
        }
    };

    // -----------------------------------------------------------------------
    // Druid Resto 4pc Wake of Bloom — Regrowth periodic heal +10%
    // (simplified from "Regrowth bloom 25% chance to splash 2 nearby allies")
    // -----------------------------------------------------------------------
    class spell_bracketsets_druid_resto_4pc_regrowth : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_druid_resto_4pc_regrowth);
        static constexpr uint32 MARKER_SPELL_ID = 67128;
        static constexpr float  BONUS_PCT       = 10.f;
        void CalcAmount(AuraEffect const*, int32& amount, bool&)
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            AddPct(amount, BONUS_PCT);
        }
        void Register() override
        {
            DoEffectCalcAmount += AuraEffectCalcAmountFn(
                spell_bracketsets_druid_resto_4pc_regrowth::CalcAmount,
                EFFECT_FIRST_FOUND, SPELL_AURA_PERIODIC_HEAL);
        }
    };

    // ========================================================================
    // Batch 12 (final): 6 last bonuses — 54 of 54 complete
    // ========================================================================

    // -----------------------------------------------------------------------
    // Paladin Holy 4pc Mending Spark — Holy Light healing +10%
    // (simplified from refit "Holy Light 25% chance to apply small HoT")
    // -----------------------------------------------------------------------
    class spell_bracketsets_paladin_holy_4pc_holy_light : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_paladin_holy_4pc_holy_light);
        static constexpr uint32 MARKER_SPELL_ID = 70756;
        static constexpr float  BONUS_PCT       = 10.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 heal = GetHitHeal();
            AddPct(heal, BONUS_PCT);
            SetHitHeal(heal);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_paladin_holy_4pc_holy_light::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Paladin Ret 2pc Crusader's Fervor — Seal of Righteousness damage +20%
    // -----------------------------------------------------------------------
    // SoR's melee-proc damage spell (25742 in WotLK 3.3.5a) is the trigger
    // we want to scale. Pattern B on the proc spell.
    class spell_bracketsets_paladin_ret_2pc_sor : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_paladin_ret_2pc_sor);
        static constexpr uint32 MARKER_SPELL_ID = 64878;
        static constexpr float  BONUS_PCT       = 20.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_paladin_ret_2pc_sor::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Shaman Enh 2pc Stormstrike's Echo — Lightning Shield damage +15%
    // -----------------------------------------------------------------------
    // The Lightning Shield aura on the shaman triggers a damage spell on
    // attackers when a charge fires. Each LS rank has its own damage spell.
    // Pattern B on the LS proc damage spells.
    class spell_bracketsets_shaman_enh_2pc_lightning_shield : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_enh_2pc_lightning_shield);
        static constexpr uint32 MARKER_SPELL_ID = 67220;
        static constexpr float  BONUS_PCT       = 15.f;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster || !caster->HasAura(MARKER_SPELL_ID)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_shaman_enh_2pc_lightning_shield::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Shaman Enh 4pc Searing Lash — Searing Totem damage +25%
    // -----------------------------------------------------------------------
    // Searing Totem is a creature that auto-casts attack spells at enemies.
    // GetCaster() returns the totem; the marker check goes against the
    // totem's owner (the shaman).
    class spell_bracketsets_shaman_enh_4pc_searing_totem : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_shaman_enh_4pc_searing_totem);
        static constexpr uint32 MARKER_SPELL_ID = 67221;
        static constexpr float  BONUS_PCT       = 25.f;
        void HandleOnHit()
        {
            Unit* totem = GetCaster();
            if (!totem) return;
            Unit* owner = totem->GetOwner();
            if (!owner || !owner->HasAura(MARKER_SPELL_ID)) return;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_shaman_enh_4pc_searing_totem::HandleOnHit);
        }
    };

    // -----------------------------------------------------------------------
    // Warlock Aff 4pc Soul Drain — Drain Soul tick refunds 5% maximum mana
    // (simplified from refit "Drain Soul kill refunds 30% mana" — per-tick
    // refund delivers similar net mana sustain without requiring kill detection)
    // -----------------------------------------------------------------------
    class spell_bracketsets_warlock_aff_4pc_drain_soul : public AuraScript
    {
        PrepareAuraScript(spell_bracketsets_warlock_aff_4pc_drain_soul);
        static constexpr uint32 MARKER_SPELL_ID = 67231;
        static constexpr int32  REFUND_PCT_MAX_MANA = 5;
        void HandlePeriodic(AuraEffect const* /*aurEff*/)
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            int32 refund = CalculatePct(static_cast<int32>(player->GetMaxPower(POWER_MANA)), REFUND_PCT_MAX_MANA);
            if (refund > 0) player->ModifyPower(POWER_MANA, refund);
        }
        void Register() override
        {
            // Drain Soul DBC layout: EFFECT_0 = CHANNEL_DEATH_ITEM (aura 86),
            // EFFECT_1 = PERIODIC_DAMAGE (aura 3, 3s tick), EFFECT_2 = PROC.
            // The DoT we want to ride lives on EFFECT_1, not EFFECT_0.
            OnEffectPeriodic += AuraEffectPeriodicFn(
                spell_bracketsets_warlock_aff_4pc_drain_soul::HandlePeriodic,
                EFFECT_1, SPELL_AURA_PERIODIC_DAMAGE);
        }
    };

    // -----------------------------------------------------------------------
    // Rogue Combat 4pc Adrenaline's Edge — Eviscerate +10% damage per CP above 3
    // -----------------------------------------------------------------------
    class spell_bracketsets_rogue_combat_4pc_eviscerate : public SpellScript
    {
        PrepareSpellScript(spell_bracketsets_rogue_combat_4pc_eviscerate);
        static constexpr uint32 MARKER_SPELL_ID = 67211;
        static constexpr int32  BONUS_PCT_PER_CP_OVER_3 = 10;
        void HandleOnHit()
        {
            Unit* caster = GetCaster();
            if (!caster) return;
            Player* player = caster->ToPlayer();
            if (!player || !player->HasAura(MARKER_SPELL_ID)) return;
            uint8 cp = player->GetComboPoints();
            if (cp <= 3) return;
            int32 extra_cp = cp - 3;
            int32 damage = GetHitDamage();
            AddPct(damage, BONUS_PCT_PER_CP_OVER_3 * extra_cp);
            SetHitDamage(damage);
        }
        void Register() override
        {
            OnHit += SpellHitFn(spell_bracketsets_rogue_combat_4pc_eviscerate::HandleOnHit);
        }
    };
}

namespace BracketSets
{
    void RegisterBonusEffects()
    {
        // Marker placeholder — bound to all 54 marker spell IDs by
        // data/sql/world/2026_05_13_04_spell_script_names.sql. Provides
        // chat notification on apply/remove but no game mechanic.
        RegisterSpellScript(spell_bracketsets_placeholder);

        // === Batch 1: periodic-damage modifiers (3 bonuses) ===
        // Each script is bound to the TARGETED spell's ranks (Rend,
        // Corruption, Holy Fire) by data/sql/world/2026_05_13_05_bonus_implementations.sql.
        RegisterSpellScript(spell_bracketsets_warrior_arms_2pc_rend);
        RegisterSpellScript(spell_bracketsets_warlock_aff_2pc_corruption);
        RegisterSpellScript(spell_bracketsets_priest_holy_4pc_holy_fire);

        // === Batch 2: 3 more periodic + 1 instant damage modifier ===
        // Periodic damage (pattern A — AuraScript DoEffectCalcAmount):
        RegisterSpellScript(spell_bracketsets_hunter_surv_2pc_serpent_sting);
        RegisterSpellScript(spell_bracketsets_rogue_assn_2pc_garrote);
        RegisterSpellScript(spell_bracketsets_mage_fire_2pc_ignite);
        // Instant damage (pattern B — SpellScript OnHit + SetHitDamage):
        RegisterSpellScript(spell_bracketsets_warrior_fury_2pc_cleave);

        // === Batch 3: 2 instant damage + 3 new patterns ===
        // Pattern B (instant damage):
        RegisterSpellScript(spell_bracketsets_shaman_ele_4pc_lightning_bolt);
        RegisterSpellScript(spell_bracketsets_hunter_surv_4pc_immolation_trap);
        // Pattern D (absorb amount via SPELL_AURA_SCHOOL_ABSORB):
        RegisterSpellScript(spell_bracketsets_priest_disc_2pc_power_word_shield);
        // Pattern A-heal (periodic heal amount via SPELL_AURA_PERIODIC_HEAL):
        RegisterSpellScript(spell_bracketsets_druid_resto_2pc_rejuvenation);
        // Pattern I (conditional damage — frozen-target gating):
        RegisterSpellScript(spell_bracketsets_mage_frost_4pc_frostbolt);

        // === Batch 4: 2 cooldown reducers + 2 duration extenders ===
        // Pattern F (cooldown reducer via SpellScript AfterCast + ModifySpellCooldown):
        RegisterSpellScript(spell_bracketsets_mage_fire_4pc_fire_blast);
        RegisterSpellScript(spell_bracketsets_rogue_sub_4pc_vanish);
        // Pattern G (duration extender via AuraScript OnApply + SetMaxDuration):
        RegisterSpellScript(spell_bracketsets_warlock_destro_4pc_immolate);
        RegisterSpellScript(spell_bracketsets_mage_frost_2pc_frost_nova);

        // === Batch 5: 1 proc refund + 2 pet stat modifiers ===
        // Pattern H (proc refund via SpellScript OnHit + roll_chance + ModifyPower):
        RegisterSpellScript(spell_bracketsets_rogue_combat_2pc_sinister_strike);
        // Pattern J (pet stat modifier on marker apply/remove):
        RegisterSpellScript(spell_bracketsets_hunter_bm_2pc_pet_damage);
        RegisterSpellScript(spell_bracketsets_warlock_demo_2pc_pet_damage);

        // === Batch 6: 3 cost modifiers + 1 instant damage modifier ===
        // Pattern E (cost modifier via SpellScript AfterCast + ModifyPower):
        RegisterSpellScript(spell_bracketsets_druid_feral_2pc_claw);
        RegisterSpellScript(spell_bracketsets_druid_feral_2pc_maul);
        RegisterSpellScript(spell_bracketsets_shaman_resto_2pc_healing_wave);
        // Pattern B (replication):
        RegisterSpellScript(spell_bracketsets_paladin_ret_4pc_judgement);

        // === Batch 7: 2 proc refund + 1 conditional + 2 instant damage ===
        RegisterSpellScript(spell_bracketsets_warrior_fury_4pc_heroic_strike);
        RegisterSpellScript(spell_bracketsets_druid_feral_4pc_maul);
        RegisterSpellScript(spell_bracketsets_rogue_sub_2pc_backstab);
        RegisterSpellScript(spell_bracketsets_hunter_mm_2pc_arcane_shot);
        RegisterSpellScript(spell_bracketsets_priest_shadow_2pc_mind_blast);

        // === Batch 8: pet HP + threat + DR mods + block value + Arc Missile ===
        RegisterSpellScript(spell_bracketsets_hunter_bm_4pc_pet_hp);
        RegisterSpellScript(spell_bracketsets_paladin_prot_2pc_hand_of_reckoning);
        RegisterSpellScript(spell_bracketsets_paladin_prot_4pc_consecration);
        RegisterSpellScript(spell_bracketsets_warrior_prot_2pc_shield_block);
        RegisterSpellScript(spell_bracketsets_mage_arc_2pc_arcane_missile);

        // === Batch 9: periodic-tick mechanics + crit-as-double ===
        RegisterSpellScript(spell_bracketsets_druid_balance_2pc_moonfire);
        RegisterSpellScript(spell_bracketsets_priest_holy_2pc_renew);
        RegisterSpellScript(spell_bracketsets_priest_shadow_4pc_sw_pain);
        RegisterSpellScript(spell_bracketsets_rogue_assn_4pc_rupture);
        RegisterSpellScript(spell_bracketsets_warlock_demo_4pc_health_funnel);

        // === Batch 10: compound proc chains (Pattern M) + simplifications ===
        // 3 compounds — each requires 2 scripts (trigger + consumer)
        RegisterSpellScript(spell_bracketsets_warrior_prot_4pc_revenge);
        RegisterSpellScript(spell_bracketsets_warrior_prot_4pc_thunder_clap);
        RegisterSpellScript(spell_bracketsets_mage_arc_4pc_arcane_explosion);
        RegisterSpellScript(spell_bracketsets_mage_arc_4pc_arc_missile_bonus);
        RegisterSpellScript(spell_bracketsets_shaman_resto_4pc_lhw);
        RegisterSpellScript(spell_bracketsets_shaman_resto_4pc_hw);
        // 2 simplifications
        RegisterSpellScript(spell_bracketsets_paladin_holy_2pc_flash_of_light);
        RegisterSpellScript(spell_bracketsets_druid_balance_4pc_wrath);

        // === Batch 11: 6 simplification replications (crit-chain downgrades) ===
        RegisterSpellScript(spell_bracketsets_warrior_arms_4pc_overpower);
        RegisterSpellScript(spell_bracketsets_hunter_mm_4pc_multi_shot);
        RegisterSpellScript(spell_bracketsets_priest_disc_4pc_pw_shield_duration);
        RegisterSpellScript(spell_bracketsets_shaman_ele_2pc_lightning_bolt);
        RegisterSpellScript(spell_bracketsets_warlock_destro_2pc_shadow_bolt);
        RegisterSpellScript(spell_bracketsets_druid_resto_4pc_regrowth);

        // === Batch 12 (final): the last 6 — 54 of 54 complete ===
        RegisterSpellScript(spell_bracketsets_paladin_holy_4pc_holy_light);
        RegisterSpellScript(spell_bracketsets_paladin_ret_2pc_sor);
        RegisterSpellScript(spell_bracketsets_shaman_enh_2pc_lightning_shield);
        RegisterSpellScript(spell_bracketsets_shaman_enh_4pc_searing_totem);
        RegisterSpellScript(spell_bracketsets_warlock_aff_4pc_drain_soul);
        RegisterSpellScript(spell_bracketsets_rogue_combat_4pc_eviscerate);

        // === Batches 7+: 32 bonuses remain on placeholder ===
        // Patterns still to introduce:
        //   - Crit chance modifier (Moonfire +15% periodic crit chance):
        //     No CalcCritChance hook in AC SpellScript — need alternate
        //     approach (e.g. apply secondary aura with SPELL_AURA_MOD_CRIT_CHANCE)
        //   - Threat modifier (Hand of Reckoning +50%):
        //     SpellScript with threat scaling at hit time
        //   - "After-event-next-spell" chains (Devastating Cycle,
        //     Overpowering Strike, Wrathful Strike, Quick Mend):
        //     compound proc + temporary buff aura
        //   - Multi-rank proc damage spells (Lightning Shield damage,
        //     Searing Totem attack): bind to many proc-spell IDs
        //   - Bespoke (Wake of Bloom Regrowth splash, Verdict of Light
        //     Judgement mana refund on crit, Adrenaline's Edge per-CP
        //     scaling): per-bonus custom logic
    }
}
