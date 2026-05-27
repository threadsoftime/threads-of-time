# modules/mod-bracket-sets/tests/CMakeLists.txt
#
# Standalone doctest target — builds independent of AzerothCore so it can
# run on the laptop without spinning up AC headers. Pattern from
# mod-harness-bridge/tests/.

cmake_minimum_required(VERSION 3.16)
cmake_policy(VERSION 3.16)
project(mod_bracket_sets_tests CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(FetchContent)
FetchContent_Declare(doctest
    GIT_REPOSITORY https://github.com/doctest/doctest.git
    GIT_TAG v2.4.11
)
# doctest v2.4.11 declares cmake_minimum_required(2.8); CMake 4.x rejects
# this without the policy floor override.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
FetchContent_MakeAvailable(doctest)

add_executable(bracket_sets_tests
    test_itemset_resolver.cpp
    ../src/BracketSetsItemsetResolver.cpp
)

target_include_directories(bracket_sets_tests PRIVATE ../src)
target_link_libraries(bracket_sets_tests PRIVATE doctest::doctest)

enable_testing()
add_test(NAME bracket_sets_tests COMMAND bracket_sets_tests)
