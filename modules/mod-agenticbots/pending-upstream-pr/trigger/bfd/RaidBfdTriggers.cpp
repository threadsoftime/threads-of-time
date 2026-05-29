// SPDX-License-Identifier: GPL-2.0-or-later
//
// NOTE: These triggers are not yet wired into mod-agenticbots's startup. They depend
// on mod-playerbots exposing 4 custom-context registration methods — see
// docs/superpowers/plans/2026-05-24-threads-of-time-1.0.0-foundation.md Task 18.
// Once the upstream PR lands (or a ToT-internal patch is shipped), the
// AgenticbotsRegistration.cpp glue will register these triggers via RaidBfdTriggerContext.

#include "RaidBfdTriggers.h"

#include "GenericTriggers.h"
#include "ObjectAccessor.h"
#include "Playerbots.h"
#include "NearestNpcsValue.h"

// Spell IDs are kept in sync with
// patches/ac-bfd-raid/overlays/.../boss_*.cpp and
// sql/bracket1-raid/world_bfd_raid_smart_scripts.sql.
namespace
{
    constexpr uint32 SPELL_CURSE_OF_BLACKFATHOM = 1014;  // Curse of Agony R2 (Gelihast)
    constexpr uint32 SPELL_POLYMORPH            = 118;   // Polymorph R1 (Kelris)
    constexpr uint32 SPELL_FEAR                 = 5782;  // Fear R1 (Gelihast)
    constexpr uint32 SPELL_SHADOW_PAIN          = 589;   // SW:Pain R1 (Kelris)
    constexpr uint32 SPELL_FROST_ARROW          = 8407;  // Frostbolt R4 (Sarevess)
    constexpr uint32 SPELL_MIND_BLAST           = 8092;  // Mind Blast R6 (Kelris)
    constexpr uint32 SPELL_TIDAL_SURGE          = 1680;  // Whirlwind R1 (Aku'mai)
    constexpr uint32 SPELL_SUNDER_R3            = 7405;  // Sunder Armor R3 (Ghamoo)

    constexpr uint32 NPC_GHAMOO_RA              = 4887;
    constexpr uint32 NPC_LADY_SAREVESS          = 4831;
    constexpr uint32 NPC_KELRIS                 = 4832;
    constexpr uint32 NPC_AKUMAI                 = 4829;
    constexpr uint32 NPC_GELIHAST               = 6243;
    constexpr uint32 NPC_LORGUS_JETT            = 207356;
    constexpr uint32 NPC_BLACKFATHOM_SEA_WITCH  = 4805;

    // Find any aggressive BFD boss within range — uses possible_targets value
    // populated by the generic combat engine. The AI_VALUE macro needs the
    // caller's `context` member variable, so we use the explicit form here
    // because this is a free function rather than a member.
    Unit* FindBfdBossInCombat(PlayerbotAI* botAI)
    {
        GuidVector targets = botAI->GetAiObjectContext()
            ->GetValue<GuidVector>("possible targets")->Get();
        for (ObjectGuid guid : targets)
        {
            Creature* c = botAI->GetCreature(guid);
            if (!c || !c->IsAlive())
                continue;
            uint32 e = c->GetEntry();
            if (e == NPC_GHAMOO_RA || e == NPC_LADY_SAREVESS || e == NPC_KELRIS ||
                e == NPC_AKUMAI || e == NPC_GELIHAST || e == NPC_LORGUS_JETT)
                return c;
        }
        return nullptr;
    }
}  // anonymous namespace

// =============================== V1 ===========================================

BfdPartyMemberCursedTrigger::BfdPartyMemberCursedTrigger(PlayerbotAI* botAI)
    : Trigger(botAI, "bfd party member cursed") {}

bool BfdPartyMemberCursedTrigger::IsActive()
{
    Group* group = bot->GetGroup();
    if (!group)
        return false;

    for (GroupReference* ref = group->GetFirstMember(); ref != nullptr; ref = ref->next())
    {
        Player* member = ref->GetSource();
        if (!member || !member->IsAlive() || member->GetMapId() != bot->GetMapId())
            continue;
        if (member->HasAura(SPELL_CURSE_OF_BLACKFATHOM) || member->HasAura(SPELL_POLYMORPH))
            return true;
    }
    return false;
}

BfdPartyMemberMagicDebuffTrigger::BfdPartyMemberMagicDebuffTrigger(PlayerbotAI* botAI)
    : Trigger(botAI, "bfd party member magic debuff") {}

bool BfdPartyMemberMagicDebuffTrigger::IsActive()
{
    Group* group = bot->GetGroup();
    if (!group)
        return false;

    for (GroupReference* ref = group->GetFirstMember(); ref != nullptr; ref = ref->next())
    {
        Player* member = ref->GetSource();
        if (!member || !member->IsAlive() || member->GetMapId() != bot->GetMapId())
            continue;
        if (member->HasAura(SPELL_FEAR) || member->HasAura(SPELL_SHADOW_PAIN))
            return true;
    }
    return false;
}

BfdBossInterruptibleCastTrigger::BfdBossInterruptibleCastTrigger(PlayerbotAI* botAI)
    : Trigger(botAI, "bfd boss interruptible cast") {}

bool BfdBossInterruptibleCastTrigger::IsActive()
{
    Unit* boss = FindBfdBossInCombat(botAI);
    if (!boss || !boss->HasUnitState(UNIT_STATE_CASTING))
        return false;

    Spell* spell = boss->GetCurrentSpell(CURRENT_GENERIC_SPELL);
    if (!spell || !spell->m_spellInfo)
        return false;

    uint32 id = spell->m_spellInfo->Id;
    return (id == SPELL_FROST_ARROW || id == SPELL_MIND_BLAST);
}

// =============================== V2 ===========================================

BfdAkumaiTidalSurgeTrigger::BfdAkumaiTidalSurgeTrigger(PlayerbotAI* botAI)
    : Trigger(botAI, "bfd akumai tidal surge") {}

bool BfdAkumaiTidalSurgeTrigger::IsActive()
{
    GuidVector targets = AI_VALUE(GuidVector, "possible targets");
    for (ObjectGuid guid : targets)
    {
        Creature* c = botAI->GetCreature(guid);
        if (!c || !c->IsAlive() || c->GetEntry() != NPC_AKUMAI)
            continue;

        // Aku'mai self-casts Whirlwind as instant — by the time the trigger
        // tick fires the spell may not still be in the casting state. Use the
        // aura on Aku'mai itself as a proxy (Whirlwind applies briefly).
        if (c->HasUnitState(UNIT_STATE_CASTING))
        {
            Spell* spell = c->GetCurrentSpell(CURRENT_GENERIC_SPELL);
            if (spell && spell->m_spellInfo && spell->m_spellInfo->Id == SPELL_TIDAL_SURGE)
                return true;
        }
        // Also fire while the boss is within whirlwind animation aura.
        if (c->HasAura(SPELL_TIDAL_SURGE))
            return true;
    }
    return false;
}

BfdSarevessAddsUpTrigger::BfdSarevessAddsUpTrigger(PlayerbotAI* botAI)
    : Trigger(botAI, "bfd sarevess adds up") {}

bool BfdSarevessAddsUpTrigger::IsActive()
{
    // Only DPS/tanks focus adds; healers stay on healing.
    if (botAI->IsHeal(bot))
        return false;

    bool sarevessAlive = false;
    bool seaWitchAlive = false;
    GuidVector targets = AI_VALUE(GuidVector, "possible targets");
    for (ObjectGuid guid : targets)
    {
        Creature* c = botAI->GetCreature(guid);
        if (!c || !c->IsAlive())
            continue;
        if (c->GetEntry() == NPC_LADY_SAREVESS)
            sarevessAlive = true;
        else if (c->GetEntry() == NPC_BLACKFATHOM_SEA_WITCH)
            seaWitchAlive = true;
    }
    return sarevessAlive && seaWitchAlive;
}

BfdGhamooSunderStackTrigger::BfdGhamooSunderStackTrigger(PlayerbotAI* botAI)
    : Trigger(botAI, "bfd ghamoo sunder stack") {}

bool BfdGhamooSunderStackTrigger::IsActive()
{
    // Only tanks can taunt-swap; everyone else passes.
    if (!botAI->IsTank(bot))
        return false;

    GuidVector targets = AI_VALUE(GuidVector, "possible targets");
    for (ObjectGuid guid : targets)
    {
        Creature* c = botAI->GetCreature(guid);
        if (!c || !c->IsAlive() || c->GetEntry() != NPC_GHAMOO_RA)
            continue;
        Unit* victim = c->GetVictim();
        if (!victim || victim == bot)
            return false;  // Either no current tank or I am already tanking.
        Aura* aura = victim->GetAura(SPELL_SUNDER_R3);
        if (aura && aura->GetStackAmount() >= 5)
            return true;
    }
    return false;
}
