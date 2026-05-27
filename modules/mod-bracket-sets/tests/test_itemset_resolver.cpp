// modules/mod-bracket-sets/tests/test_itemset_resolver.cpp
#define DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN
#include <doctest/doctest.h>
#include "BracketSetsItemsetResolver.h"
#include "BracketSetsConstants.h"

using BracketSets::ItemsetResolver;
using BracketSets::BRACKET_1;
using BracketSets::SENTINEL_ITEMSET_ID;

TEST_CASE("ItemsetResolver: empty -> Resolve returns 0") {
    ItemsetResolver r;
    CHECK(r.Resolve(1, 1, BRACKET_1) == 0);
    CHECK(r.Size() == 0);
}

TEST_CASE("ItemsetResolver: single Insert is retrievable") {
    ItemsetResolver r;
    r.Insert(1, 1, BRACKET_1, 90111);
    CHECK(r.Resolve(1, 1, BRACKET_1) == 90111);
    CHECK(r.Size() == 1);
}

TEST_CASE("ItemsetResolver: full Bracket 1 seed maps correctly") {
    ItemsetResolver r;
    // Warrior
    r.Insert(1, 1, BRACKET_1, 90111); r.Insert(1, 2, BRACKET_1, 90112); r.Insert(1, 3, BRACKET_1, 90113);
    // Paladin
    r.Insert(2, 1, BRACKET_1, 90121); r.Insert(2, 2, BRACKET_1, 90122); r.Insert(2, 3, BRACKET_1, 90123);
    // Hunter
    r.Insert(3, 1, BRACKET_1, 90131); r.Insert(3, 2, BRACKET_1, 90132); r.Insert(3, 3, BRACKET_1, 90133);
    // Rogue
    r.Insert(4, 1, BRACKET_1, 90141); r.Insert(4, 2, BRACKET_1, 90142); r.Insert(4, 3, BRACKET_1, 90143);
    // Priest
    r.Insert(5, 1, BRACKET_1, 90151); r.Insert(5, 2, BRACKET_1, 90152); r.Insert(5, 3, BRACKET_1, 90153);
    // Shaman
    r.Insert(7, 1, BRACKET_1, 90161); r.Insert(7, 2, BRACKET_1, 90162); r.Insert(7, 3, BRACKET_1, 90163);
    // Mage
    r.Insert(8, 1, BRACKET_1, 90171); r.Insert(8, 2, BRACKET_1, 90172); r.Insert(8, 3, BRACKET_1, 90173);
    // Warlock
    r.Insert(9, 1, BRACKET_1, 90181); r.Insert(9, 2, BRACKET_1, 90182); r.Insert(9, 3, BRACKET_1, 90183);
    // Druid
    r.Insert(11, 1, BRACKET_1, 90191); r.Insert(11, 2, BRACKET_1, 90192); r.Insert(11, 3, BRACKET_1, 90193);

    CHECK(r.Size() == 27);

    // Spot-check every class's spec 1
    CHECK(r.Resolve(1,  1, BRACKET_1) == 90111);
    CHECK(r.Resolve(2,  1, BRACKET_1) == 90121);
    CHECK(r.Resolve(3,  1, BRACKET_1) == 90131);
    CHECK(r.Resolve(4,  1, BRACKET_1) == 90141);
    CHECK(r.Resolve(5,  1, BRACKET_1) == 90151);
    CHECK(r.Resolve(7,  1, BRACKET_1) == 90161);
    CHECK(r.Resolve(8,  1, BRACKET_1) == 90171);
    CHECK(r.Resolve(9,  1, BRACKET_1) == 90181);
    CHECK(r.Resolve(11, 1, BRACKET_1) == 90191);

    // Spec 2 / spec 3 distinct from spec 1
    CHECK(r.Resolve(1, 2, BRACKET_1) == 90112);
    CHECK(r.Resolve(1, 3, BRACKET_1) == 90113);

    // Unmapped class (Death Knight = class 6)
    CHECK(r.Resolve(6, 1, BRACKET_1) == 0);

    // Out-of-range class
    CHECK(r.Resolve(99, 1, BRACKET_1) == 0);

    // Out-of-range spec
    CHECK(r.Resolve(1, 4, BRACKET_1) == 0);
    CHECK(r.Resolve(1, 0, BRACKET_1) == 0);  // caller is expected to default 0 -> 1 before calling

    // Wrong bracket
    CHECK(r.Resolve(1, 1, 2) == 0);
}

TEST_CASE("ItemsetResolver: last write wins on duplicate key") {
    ItemsetResolver r;
    r.Insert(1, 1, BRACKET_1, 99999);
    r.Insert(1, 1, BRACKET_1, 90111);
    CHECK(r.Resolve(1, 1, BRACKET_1) == 90111);
    CHECK(r.Size() == 1);
}

TEST_CASE("ItemsetResolver: Clear empties the map") {
    ItemsetResolver r;
    r.Insert(1, 1, BRACKET_1, 90111);
    r.Clear();
    CHECK(r.Size() == 0);
    CHECK(r.Resolve(1, 1, BRACKET_1) == 0);
}

TEST_CASE("ItemsetResolver: SENTINEL constant has expected value") {
    CHECK(SENTINEL_ITEMSET_ID == 90101);
}
