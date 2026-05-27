// modules/mod-bracket-sets/src/BracketSetsItemsetResolver.h
#ifndef MOD_BRACKET_SETS_ITEMSET_RESOLVER_H
#define MOD_BRACKET_SETS_ITEMSET_RESOLVER_H

#include <cstddef>
#include <cstdint>
#include <map>
#include <tuple>

namespace BracketSets {

// Pure-data resolver. No dependency on Player/Unit/AC headers.
// Loaded by BracketSetsManager from bracket_set_itemset_map at startup.
class ItemsetResolver {
public:
    // Insert a (class, spec, bracket) -> itemset mapping. Last write wins.
    void Insert(uint8_t classId, uint8_t specId, uint8_t bracketId, uint32_t itemsetId);

    // Returns the patched itemset_id for (class, spec, bracket).
    // Returns 0 if no mapping exists (unmapped class, e.g. Death Knight).
    // Caller is expected to default spec 0 -> spec 1 BEFORE calling.
    uint32_t Resolve(uint8_t classId, uint8_t specId, uint8_t bracketId) const;

    // Total rows currently held. Used by startup-log assertion.
    size_t Size() const;

    // Clear all mappings. Used by tests; not called in production.
    void Clear();

private:
    std::map<std::tuple<uint8_t, uint8_t, uint8_t>, uint32_t> m_map;
};

}  // namespace BracketSets

#endif  // MOD_BRACKET_SETS_ITEMSET_RESOLVER_H
