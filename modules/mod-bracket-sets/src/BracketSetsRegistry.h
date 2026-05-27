#ifndef MOD_BRACKET_SETS_REGISTRY_H
#define MOD_BRACKET_SETS_REGISTRY_H

#include "Common.h"

#include <cstdint>
#include <unordered_map>

namespace BracketSets
{
    // Key into the bonus map: (itemset_id, threshold 2 or 4, class 1..11, spec 1..3).
    struct BonusKey
    {
        uint32 itemset_id;
        uint8 threshold;
        uint8 class_id;
        uint8 spec_id;

        bool operator==(BonusKey const& other) const
        {
            return itemset_id == other.itemset_id &&
                   threshold  == other.threshold  &&
                   class_id   == other.class_id   &&
                   spec_id    == other.spec_id;
        }
    };

    struct BonusKeyHash
    {
        size_t operator()(BonusKey const& k) const noexcept
        {
            // FNV-1a-ish mix on the packed 32-bit ((itemset<<16)|(threshold<<10)|(class<<6)|spec).
            uint32 packed = (k.itemset_id << 16) ^
                            (uint32(k.threshold) << 10) ^
                            (uint32(k.class_id) << 6) ^
                            uint32(k.spec_id);
            return std::hash<uint32>()(packed);
        }
    };

    struct BonusRow
    {
        uint32 spell_id;
        uint8 bracket_min;
        uint8 bracket_max;
        std::string display_name;
    };

    using BonusMap = std::unordered_map<BonusKey, BonusRow, BonusKeyHash>;

    // True iff this itemset ID is in the bracket-sets managed range (90101..90199).
    bool IsBracketSet(uint32 itemset_id);

    // Lookup a single bonus row. Returns nullptr if no row exists for the key.
    // Caller does not own the returned pointer; it points into the cached map.
    BonusRow const* Lookup(BonusKey const& key);

    // Find the first bonus row whose spell_id matches. Returns the display
    // name from that row, or empty string if no match. Used by the SpellScript
    // placeholder to surface a friendly name on aura apply/remove without
    // having to thread the bonus context through the engine's call stack.
    std::string LookupDisplayNameBySpell(uint32 spell_id);

    // Singleton-style accessors. Registry is loaded once at world boot.
    BonusMap const& Bonuses();
    void LoadBonusMap();
    void ReloadBonusMap();
    size_t BonusCount();
}

#endif // MOD_BRACKET_SETS_REGISTRY_H
