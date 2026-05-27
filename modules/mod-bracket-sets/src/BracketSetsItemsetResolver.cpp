// modules/mod-bracket-sets/src/BracketSetsItemsetResolver.cpp
#include "BracketSetsItemsetResolver.h"

namespace BracketSets {

void ItemsetResolver::Insert(uint8_t classId, uint8_t specId, uint8_t bracketId, uint32_t itemsetId) {
    m_map[{classId, specId, bracketId}] = itemsetId;
}

uint32_t ItemsetResolver::Resolve(uint8_t classId, uint8_t specId, uint8_t bracketId) const {
    auto it = m_map.find({classId, specId, bracketId});
    if (it == m_map.end()) return 0;
    return it->second;
}

size_t ItemsetResolver::Size() const {
    return m_map.size();
}

void ItemsetResolver::Clear() {
    m_map.clear();
}

}  // namespace BracketSets
