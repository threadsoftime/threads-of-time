#ifndef MOD_BRACKET_SETS_BONUS_EFFECTS_H
#define MOD_BRACKET_SETS_BONUS_EFFECTS_H

namespace BracketSets
{
    // Step 6 of the implementation plan: this header declares the registration
    // entry point for all 54 AuraScript / SpellScript classes implementing the
    // individual bonus effects.
    //
    // v0 scaffold: registration is a no-op. Subsequent PRs add base classes
    // (PassiveStatModifier, ProcOnSpellHit, etc.) and one subclass per
    // (class, spec, threshold) bonus row in bracket_set_bonus_map.

    void RegisterBonusEffects();
}

#endif // MOD_BRACKET_SETS_BONUS_EFFECTS_H
