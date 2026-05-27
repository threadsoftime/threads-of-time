#ifndef MOD_WARFORGED_CONSTANTS_H
#define MOD_WARFORGED_CONSTANTS_H

#include "Common.h"        // for AC's uint8/uint32 typedefs
#include "ItemTemplate.h"  // for ITEM_QUALITY_* enum
#include "Item.h"          // for ENCHANTMENT_SLOT enum

namespace ModWarforged
{
    // Slots we own (per design spec §7).
    // Typed as EnchantmentSlot (the enum) so Item::SetEnchantment overload resolves
    // without an implicit narrowing conversion. Still implicit-converts to uint8 for
    // MockItem in tests (EnchantmentSlot's underlying type is small).
    static constexpr EnchantmentSlot WF_BONUS_SLOT  = BONUS_ENCHANTMENT_SLOT;     // 5
    static constexpr EnchantmentSlot WF_SOCKET_SLOT = PRISMATIC_ENCHANTMENT_SLOT; // 6

    // Existing AC prismatic-socket enchant ID (Eternal Belt Buckle uses this)
    static constexpr uint32 PRISMATIC_SOCKET_ENCHANT_ID = 3729;

    // Our Warforged enchant ID range — see spec §4.1 for the band/quality matrix
    static constexpr uint32 WF_ENCHANT_ID_MIN = 70001;
    static constexpr uint32 WF_ENCHANT_ID_MAX = 70063;

    // Ilvl bumped by Warforged proc (tooltip-display only; stats come from DBC enchant)
    static constexpr uint32 WF_ILVL_BUMP = 5;

    // ilvl band boundaries (inclusive upper bound; index == band number 0..6)
    static constexpr uint32 ILVL_BAND_BOUNDS[7] = { 25, 50, 80, 115, 150, 200, 9999 };

    // Returns true if the given enchant ID is one of ours (a Warforged tier enchant).
    inline bool IsWarforgedEnchant(uint32 enchantId)
    {
        return enchantId >= WF_ENCHANT_ID_MIN && enchantId <= WF_ENCHANT_ID_MAX;
    }
}

#endif // MOD_WARFORGED_CONSTANTS_H
