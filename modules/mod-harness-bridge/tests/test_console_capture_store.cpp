#include <doctest/doctest.h>

#include "ConsoleCaptureStore.h"

#include <chrono>
#include <string>

using HarnessBridge::ConsoleCaptureStore;

TEST_CASE("ConsoleCaptureStore: insert/append/mark/read happy path") {
    ConsoleCaptureStore store;
    store.Insert("req_abc");
    store.Append("req_abc", "hello\n");
    store.Append("req_abc", "world\n");
    store.MarkDone("req_abc", true);

    auto cap = store.Read("req_abc");
    REQUIRE(cap.has_value());
    CHECK(cap->buffer == "hello\nworld\n");
    CHECK(cap->done == true);
    CHECK(cap->success == true);
    CHECK(cap->truncated == false);
}

TEST_CASE("ConsoleCaptureStore: Read returns nullopt for unknown id") {
    ConsoleCaptureStore store;
    auto cap = store.Read("req_missing");
    CHECK_FALSE(cap.has_value());
}

TEST_CASE("ConsoleCaptureStore: Insert is idempotent") {
    ConsoleCaptureStore store;
    store.Insert("req_x");
    store.Append("req_x", "a");
    store.Insert("req_x");  // should NOT reset the buffer
    store.Append("req_x", "b");
    auto cap = store.Read("req_x");
    REQUIRE(cap.has_value());
    CHECK(cap->buffer == "ab");
}

TEST_CASE("ConsoleCaptureStore: Append past 64 KiB truncates and flags") {
    ConsoleCaptureStore store;
    store.Insert("req_big");
    std::string chunk(8 * 1024, 'x');  // 8 KiB
    for (int i = 0; i < 10; ++i)        // 80 KiB total
        store.Append("req_big", chunk);
    store.MarkDone("req_big", true);

    auto cap = store.Read("req_big");
    REQUIRE(cap.has_value());
    CHECK(cap->buffer.size() == 64 * 1024);
    CHECK(cap->truncated == true);
    CHECK(cap->done == true);
}

TEST_CASE("ConsoleCaptureStore: EvictExpired drops captures older than TTL") {
    ConsoleCaptureStore store;
    store.Insert("req_old");
    store.MarkDone("req_old", true);

    auto far_future = std::chrono::steady_clock::now() + std::chrono::seconds(120);
    store.EvictExpired(far_future);

    CHECK_FALSE(store.Read("req_old").has_value());
}

TEST_CASE("ConsoleCaptureStore: EvictExpired keeps not-yet-done captures") {
    ConsoleCaptureStore store;
    store.Insert("req_pending");
    store.Append("req_pending", "partial");
    // intentionally no MarkDone

    auto far_future = std::chrono::steady_clock::now() + std::chrono::seconds(120);
    store.EvictExpired(far_future);

    auto cap = store.Read("req_pending");
    REQUIRE(cap.has_value());
    CHECK(cap->done == false);
    CHECK(cap->buffer == "partial");
}

TEST_CASE("ConsoleCaptureStore: hard cap on entry count via EvictExpired") {
    ConsoleCaptureStore store;
    // Insert 260 captures (cap is 256); after EvictExpired the oldest
    // 4 should be evicted regardless of done-state.
    for (int i = 0; i < 260; ++i)
        store.Insert("req_" + std::to_string(i));
    store.EvictExpired(std::chrono::steady_clock::now());

    // First 4 should be gone (FIFO).
    for (int i = 0; i < 4; ++i)
        CHECK_FALSE(store.Read("req_" + std::to_string(i)).has_value());
    // Most recent should still be present.
    CHECK(store.Read("req_259").has_value());
}
