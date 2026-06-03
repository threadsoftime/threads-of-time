// DOCTEST_CONFIG_IMPLEMENT_WITH_MAIN lives in test_config.cpp (one TU only).
#include <doctest/doctest.h>

#include "LfgIntentStore.h"

// ---------------------------------------------------------------------------
// Cancel-queue tests
// ---------------------------------------------------------------------------
// These tests exercise RecordLfgCancel / DrainLfgCancels in isolation.
// The cancel queue is world-thread-only (no mutex, matching the join queue).
//
// Each TEST_CASE is self-contained: because gCancelQueue is a translation-unit
// static, we drain it fully at the start of any case that cares about ordering
// to avoid inter-test bleed.
// ---------------------------------------------------------------------------

TEST_CASE("LfgCancelQueue: record then drain returns guid, second drain is empty")
{
    // Drain any residual from prior cases.
    HarnessBridge::DrainLfgCancels(1024);

    HarnessBridge::RecordLfgCancel(42ULL);

    auto first = HarnessBridge::DrainLfgCancels(10);
    REQUIRE(first.size() == 1);
    CHECK(first[0] == 42ULL);

    auto second = HarnessBridge::DrainLfgCancels(10);
    CHECK(second.empty());
}

TEST_CASE("LfgCancelQueue: max cap is respected and FIFO order preserved")
{
    HarnessBridge::DrainLfgCancels(1024);

    HarnessBridge::RecordLfgCancel(1ULL);
    HarnessBridge::RecordLfgCancel(2ULL);
    HarnessBridge::RecordLfgCancel(3ULL);

    // Request only 2; should get the first two in insertion order.
    auto batch = HarnessBridge::DrainLfgCancels(2);
    REQUIRE(batch.size() == 2);
    CHECK(batch[0] == 1ULL);
    CHECK(batch[1] == 2ULL);

    // The third entry remains.
    auto tail = HarnessBridge::DrainLfgCancels(10);
    REQUIRE(tail.size() == 1);
    CHECK(tail[0] == 3ULL);
}

TEST_CASE("LfgCancelQueue: drain(0) returns empty without consuming entries")
{
    HarnessBridge::DrainLfgCancels(1024);

    HarnessBridge::RecordLfgCancel(99ULL);

    auto empty = HarnessBridge::DrainLfgCancels(0);
    CHECK(empty.empty());

    // Entry is still there.
    auto real = HarnessBridge::DrainLfgCancels(10);
    REQUIRE(real.size() == 1);
    CHECK(real[0] == 99ULL);
}

TEST_CASE("LfgCancelQueue: drain of empty queue is harmless")
{
    HarnessBridge::DrainLfgCancels(1024);  // ensure empty

    auto result = HarnessBridge::DrainLfgCancels(10);
    CHECK(result.empty());
}
