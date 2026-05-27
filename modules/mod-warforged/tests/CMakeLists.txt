# Standalone doctest target — builds independent of AzerothCore so it can
# run on the laptop without spinning up AC headers. Pattern from
# modules/mod-harness-bridge/tests/CMakeLists.txt.
#
# Build & run locally:
#   cd modules/mod-warforged/tests
#   cmake -B build .
#   cmake --build build
#   ./build/warforged_tests
#
# What this target covers: only the pure decision functions in
# WarforgedRng.h (defined inline, no AC dependencies). Production
# wrappers (RollWarforged / RollSocket) and the stochastic
# distribution test are deferred to Heimdal-side validation at
# Task 10+ — they need the real urand() and gConfig linkage.

cmake_minimum_required(VERSION 3.16)
cmake_policy(VERSION 3.16)

# Allow the vendored doctest's old `cmake_minimum_required(VERSION 3.0)`
# to keep configuring under CMake 4.x. Without this, CMake 4 refuses to
# process anything below its bumped floor of 3.5. (See doctest issue #803.)
set(CMAKE_POLICY_VERSION_MINIMUM 3.5 CACHE STRING "" FORCE)

project(mod_warforged_tests CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(FetchContent)
FetchContent_Declare(doctest
    GIT_REPOSITORY https://github.com/doctest/doctest.git
    GIT_TAG v2.4.11
)
FetchContent_MakeAvailable(doctest)

add_executable(warforged_tests
    test_warforged_rng.cpp
    test_warforged_picker.cpp
    test_warforged_applier.cpp
)

target_include_directories(warforged_tests PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/..
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(warforged_tests PRIVATE doctest::doctest)

enable_testing()
add_test(NAME warforged_tests COMMAND warforged_tests)
