// modules/mod-bracket-sets/src/BracketSetsConstants.h
#ifndef MOD_BRACKET_SETS_CONSTANTS_H
#define MOD_BRACKET_SETS_CONSTANTS_H

#include <cstdint>

namespace BracketSets {

// Server-internal sentinel itemset ID. Stamped into item_template.itemset
// to mark Bracket 1 pieces. Rewritten to per-class+spec itemset_id by the
// item-query response hook before being sent to the client. See spec §4.1.
constexpr uint32_t SENTINEL_ITEMSET_ID = 90101;

// Bracket identifier used by the resolver and itemset map. Scales to
// BRACKET_2..BRACKET_7 when those catalogs come online.
constexpr uint8_t BRACKET_1 = 1;

}  // namespace BracketSets

#endif  // MOD_BRACKET_SETS_CONSTANTS_H
