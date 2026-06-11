//! Per-spec Bracket-1 rotation plugins (M2 slice 2.2). Const tables of rank-1 spell ids;
//! rank/cooldown/power truth is SERVER-side (bot.cast_spell upranks + guards). Priorities
//! mined from mod-playerbots strategy sources (FrostMageStrategy.cpp, DpsRogueStrategy.cpp,
//! GenericWarriorStrategy.cpp, ShadowPriestStrategy.cpp — smite-based: shadowform is L40).
//! All ids verified rank-1 heads in data/sql/base/db_world/spell_ranks.sql.

use crate::combat::{AutoAttackAction, CastSpellAction, CombatContext, RotationPlugin};

// Warrior
pub const CHARGE: u32 = 100;
pub const REND: u32 = 772;
pub const HEROIC_STRIKE: u32 = 78;
pub const BATTLE_SHOUT: u32 = 6673;
// Rogue
pub const SINISTER_STRIKE: u32 = 1752;
pub const EVISCERATE: u32 = 2098;
pub const SLICE_AND_DICE: u32 = 5171;
// Mage
pub const FROSTBOLT: u32 = 116;
pub const FIRE_BLAST: u32 = 2136;
pub const ARCANE_INTELLECT: u32 = 1459;
pub const FROST_ARMOR: u32 = 168;
// Priest
pub const SMITE: u32 = 585;
pub const SHADOW_WORD_PAIN: u32 = 589;
pub const PW_FORTITUDE: u32 = 1243;
pub const INNER_FIRE: u32 = 588;
pub const PW_SHIELD: u32 = 17;  // rank-1 head, spell_ranks.sql (17,17,1)
pub const RENEW: u32 = 139;     // rank-1 head, spell_ranks.sql (139,139,1)

// Named preconditions (also unit-tested directly).
pub const EVISCERATE_PRECOND: fn(&CombatContext) -> bool = |c| c.combo_points >= 3;
pub const SND_PRECOND: fn(&CombatContext) -> bool = |c| c.combo_points >= 2;
/// Pull-time application: the client cannot see target debuffs, so dot/opener
/// abilities fire only while the target is near-full hp (first tick of a fight).
pub const SWP_PRECOND: fn(&CombatContext) -> bool = |c| c.target_hp_pct > 90.0;
pub const REND_PRECOND: fn(&CombatContext) -> bool = |c| c.target_hp_pct > 90.0;
/// Charge is 8–25y; fires only at pull distance.
pub const CHARGE_PRECOND: fn(&CombatContext) -> bool = |c| c.target_distance > 8.0 && c.target_distance < 25.0;
/// Defensives (design 2026-06-10 §2.3): shield when hurt, renew as emergency heal.
/// Weakened Soul re-cast rejection returns a typed `Failed` code and falls through.
pub const PW_SHIELD_PRECOND: fn(&CombatContext) -> bool = |c| c.bot_hp_pct < 50.0;
pub const RENEW_PRECOND: fn(&CombatContext) -> bool = |c| c.bot_hp_pct < 35.0;

const ALWAYS: fn(&CombatContext) -> bool = |_| true;

fn cast(name: &'static str, spell_id: u32, range: f64, precondition: fn(&CombatContext) -> bool) -> CastSpellAction {
    CastSpellAction { name, spell_id, range, precondition, self_cast: false }
}

fn buff(name: &'static str, spell_id: u32) -> CastSpellAction {
    CastSpellAction { name, spell_id, range: 0.0, precondition: ALWAYS, self_cast: true }
}

/// An in-rotation self-cast (defensives): like `buff` but with a real precondition.
fn defensive(name: &'static str, spell_id: u32, precondition: fn(&CombatContext) -> bool) -> CastSpellAction {
    CastSpellAction { name, spell_id, range: 0.0, precondition, self_cast: true }
}

/// Build a rotation plugin by id (the profile/Goal `rotation_id` seam).
pub fn build(rotation_id: &str) -> Option<RotationPlugin> {
    match rotation_id {
        "auto_attack" => Some(RotationPlugin::melee_m1()),
        "warrior_b1" => Some(RotationPlugin::new(
            vec![
                Box::new(cast("charge", CHARGE, 25.0, CHARGE_PRECOND)),
                Box::new(cast("rend", REND, 5.0, REND_PRECOND)),
                Box::new(cast("heroic_strike", HEROIC_STRIKE, 5.0, ALWAYS)),
                Box::new(AutoAttackAction),
            ],
            vec![buff("battle_shout", BATTLE_SHOUT)],
            5.0,
        )),
        "rogue_b1" => Some(RotationPlugin::new(
            vec![
                Box::new(cast("eviscerate", EVISCERATE, 5.0, EVISCERATE_PRECOND)),
                Box::new(cast("slice_and_dice", SLICE_AND_DICE, 5.0, SND_PRECOND)),
                Box::new(cast("sinister_strike", SINISTER_STRIKE, 5.0, ALWAYS)),
                Box::new(AutoAttackAction),
            ],
            vec![],
            5.0,
        )),
        "mage_frost_b1" => Some(RotationPlugin::new(
            vec![
                Box::new(cast("frostbolt", FROSTBOLT, 30.0, ALWAYS)),
                Box::new(cast("fire_blast", FIRE_BLAST, 20.0, ALWAYS)),
                Box::new(AutoAttackAction),
            ],
            vec![buff("arcane_intellect", ARCANE_INTELLECT), buff("frost_armor", FROST_ARMOR)],
            25.0,
        )),
        "priest_smite_b1" => Some(RotationPlugin::new(
            vec![
                // Defensives FIRST — survival outranks damage (design §2.3).
                Box::new(defensive("pw_shield", PW_SHIELD, PW_SHIELD_PRECOND)),
                Box::new(defensive("renew", RENEW, RENEW_PRECOND)),
                Box::new(cast("shadow_word_pain", SHADOW_WORD_PAIN, 30.0, SWP_PRECOND)),
                Box::new(cast("smite", SMITE, 30.0, ALWAYS)),
                Box::new(AutoAttackAction),
            ],
            vec![buff("pw_fortitude", PW_FORTITUDE), buff("inner_fire", INNER_FIRE)],
            25.0,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatContext;

    fn ctx(combo: u8, target_hp: f32) -> CombatContext {
        CombatContext { bot_hp_pct: 100.0, bot_power_pct: 50.0, bot_mana_pct: Some(80.0),
                        combo_points: combo, target_hp_pct: target_hp, target_distance: 4.0 }
    }

    #[test]
    fn registry_knows_all_five() {
        for id in ["auto_attack", "warrior_b1", "rogue_b1", "mage_frost_b1", "priest_smite_b1"] {
            assert!(build(id).is_some(), "rotation {id} must build");
        }
        assert!(build("nonexistent").is_none());
    }

    #[test]
    fn casters_have_ranged_engage_melee_have_short() {
        assert_eq!(build("mage_frost_b1").unwrap().engage_range(), 25.0);
        assert_eq!(build("priest_smite_b1").unwrap().engage_range(), 25.0);
        assert_eq!(build("warrior_b1").unwrap().engage_range(), 5.0);
        assert_eq!(build("auto_attack").unwrap().engage_range(), 5.0);
    }

    #[test]
    fn rogue_finisher_gated_on_combo_points() {
        assert!(!(EVISCERATE_PRECOND)(&ctx(2, 50.0)));
        assert!((EVISCERATE_PRECOND)(&ctx(3, 50.0)));
    }

    #[test]
    fn pull_only_abilities_require_high_target_hp() {
        assert!((SWP_PRECOND)(&ctx(0, 95.0)));
        assert!(!(SWP_PRECOND)(&ctx(0, 60.0)));
    }

    #[test]
    fn buff_lists_present_for_buffing_specs() {
        assert_eq!(build("mage_frost_b1").unwrap().buffs().len(), 2);
        assert_eq!(build("priest_smite_b1").unwrap().buffs().len(), 2);
        assert_eq!(build("warrior_b1").unwrap().buffs().len(), 1);
        assert!(build("rogue_b1").unwrap().buffs().is_empty());
    }

    #[test]
    fn priest_defensives_gate_on_own_hp() {
        let healthy = ctx(0, 50.0); // helper builds bot_hp_pct: 100.0
        assert!(!(PW_SHIELD_PRECOND)(&healthy));
        assert!(!(RENEW_PRECOND)(&healthy));

        let mut hurt = ctx(0, 50.0);
        hurt.bot_hp_pct = 45.0;
        assert!((PW_SHIELD_PRECOND)(&hurt), "shield under 50%");
        assert!(!(RENEW_PRECOND)(&hurt), "renew only under 35%");

        hurt.bot_hp_pct = 30.0;
        assert!((RENEW_PRECOND)(&hurt), "renew under 35%");
    }
}
