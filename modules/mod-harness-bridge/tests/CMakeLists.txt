# Standalone doctest target — builds independent of AzerothCore so it can
# run on the laptop without spinning up AC headers. Pattern from
# mod-playerbots/tests/llmagent/.

cmake_minimum_required(VERSION 3.16)
cmake_policy(VERSION 3.16)
project(mod_harness_bridge_tests CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(FetchContent)
FetchContent_Declare(doctest
    GIT_REPOSITORY https://github.com/doctest/doctest.git
    GIT_TAG v2.4.11
)
FetchContent_MakeAvailable(doctest)

add_executable(harness_bridge_tests
    test_config.cpp
    test_dispatch.cpp
    test_console_capture_store.cpp
    ../src/ConsoleCaptureStore.cpp
    test_lfg_intent_store.cpp
    ../src/LfgIntentStore.cpp
)

target_include_directories(harness_bridge_tests PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/..
    ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(harness_bridge_tests PRIVATE doctest::doctest)

enable_testing()
add_test(NAME harness_bridge_tests COMMAND harness_bridge_tests)
