# bluebus for C and C++ with CMake (Linux).
#
# In your project, either use an installed / prebuilt bluebus:
#   find_package(bluebus 0.1 REQUIRED)
#   target_link_libraries(app PRIVATE bluebus::bluebus)
# or build it from source as part of your project:
#   add_subdirectory(path/to/bluebus/capi bluebus)
#   target_link_libraries(app PRIVATE bluebus::bluebus)
#
# Builds the Rust library with cargo (stable Rust). Options:
#   BLUEBUS_STATIC    bluebus::bluebus links the static library (default OFF)
#   BLUEBUS_TESTS     build the C/C++ tests and examples, run them with ctest
#                     (needs dbus-daemon; the tests use a mock BlueZ)
#   BLUEBUS_EXAMPLES  build the examples (scan, central, peripheral)
# `cmake --install` installs the header, both libraries, the CMake package
# (lib/cmake/bluebus) and pkg-config file (lib/pkgconfig/bluebus.pc).
cmake_minimum_required(VERSION 3.16)
project(bluebus_c VERSION 0.1.0 LANGUAGES C CXX)

if(NOT CMAKE_SYSTEM_NAME STREQUAL "Linux")
  message(FATAL_ERROR "bluebus supports Linux only (BlueZ over D-Bus)")
endif()

option(BLUEBUS_STATIC "Link bluebus::bluebus statically" OFF)
option(BLUEBUS_TESTS "Build the bluebus C/C++ tests (ctest)" OFF)
option(BLUEBUS_EXAMPLES "Build the bluebus C/C++ examples" ${BLUEBUS_TESTS})

find_program(CARGO cargo HINTS $ENV{HOME}/.cargo/bin REQUIRED)
include(GNUInstallDirs)

set(_root ${CMAKE_CURRENT_SOURCE_DIR}/..)
set(_target_dir ${CMAKE_CURRENT_BINARY_DIR}/cargo)
set(_out ${_target_dir}/release)
set(_shared ${_out}/libbluebus.so)
set(_static ${_out}/libbluebus.a)
file(GLOB _sources CONFIGURE_DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/src/*.rs ${_root}/src/*.rs
     ${_root}/src/ble/*.rs)

add_custom_command(
  OUTPUT ${_shared} ${_static}
  COMMAND ${CARGO} build --release -p bluebus-c --target-dir ${_target_dir}
  WORKING_DIRECTORY ${_root}
  DEPENDS ${_sources} ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml ${_root}/Cargo.toml
  COMMENT "Building bluebus (cargo)"
  USES_TERMINAL)
add_custom_target(bluebus_cargo ALL DEPENDS ${_shared} ${_static})

# System libraries the Rust static library needs.
set(BLUEBUS_STATIC_LIBS pthread dl m rt)

add_library(bluebus_shared SHARED IMPORTED GLOBAL)
set_target_properties(bluebus_shared PROPERTIES
  IMPORTED_LOCATION ${_shared}
  IMPORTED_SONAME libbluebus.so
  INTERFACE_INCLUDE_DIRECTORIES ${CMAKE_CURRENT_SOURCE_DIR}/include
  INTERFACE_LINK_OPTIONS "LINKER:-rpath,${_out}")
add_dependencies(bluebus_shared bluebus_cargo)

add_library(bluebus_static STATIC IMPORTED GLOBAL)
set_target_properties(bluebus_static PROPERTIES
  IMPORTED_LOCATION ${_static}
  INTERFACE_INCLUDE_DIRECTORIES ${CMAKE_CURRENT_SOURCE_DIR}/include
  INTERFACE_LINK_LIBRARIES "${BLUEBUS_STATIC_LIBS}")
add_dependencies(bluebus_static bluebus_cargo)

if(BLUEBUS_STATIC)
  add_library(bluebus::bluebus ALIAS bluebus_static)
else()
  add_library(bluebus::bluebus ALIAS bluebus_shared)
endif()
add_library(bluebus::shared ALIAS bluebus_shared)
add_library(bluebus::static ALIAS bluebus_static)

# The shared library file, e.g. to copy next to an executable.
set(BLUEBUS_SHARED_LIBRARY ${_shared} CACHE INTERNAL "")

# ---------------------------------------------------------------------------
# Install: header, libraries, CMake package, pkg-config
# ---------------------------------------------------------------------------
install(FILES include/bluebus.h include/bluebus.hpp DESTINATION ${CMAKE_INSTALL_INCLUDEDIR})
install(FILES ${_shared} ${_static} DESTINATION ${CMAKE_INSTALL_LIBDIR})

include(CMakePackageConfigHelpers)
set(BLUEBUS_INSTALL_LIBDIR ${CMAKE_INSTALL_LIBDIR})
set(BLUEBUS_INSTALL_INCLUDEDIR ${CMAKE_INSTALL_INCLUDEDIR})
configure_package_config_file(cmake/bluebusConfig.cmake.in
  ${CMAKE_CURRENT_BINARY_DIR}/bluebusConfig.cmake
  INSTALL_DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/bluebus
  PATH_VARS BLUEBUS_INSTALL_LIBDIR BLUEBUS_INSTALL_INCLUDEDIR)
write_basic_package_version_file(${CMAKE_CURRENT_BINARY_DIR}/bluebusConfigVersion.cmake
  COMPATIBILITY SameMinorVersion)
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/bluebusConfig.cmake
              ${CMAKE_CURRENT_BINARY_DIR}/bluebusConfigVersion.cmake
        DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/bluebus)

string(REPLACE ";" " -l" _pc_static "-l${BLUEBUS_STATIC_LIBS}")
set(BLUEBUS_PC_LIBDIR ${CMAKE_INSTALL_LIBDIR})
set(BLUEBUS_PC_INCLUDEDIR ${CMAKE_INSTALL_INCLUDEDIR})
set(BLUEBUS_PC_LIBS_PRIVATE ${_pc_static})
# Relocatable: the prefix relative to the installed .pc file.
file(RELATIVE_PATH BLUEBUS_PC_PREFIX_REL ${CMAKE_INSTALL_FULL_LIBDIR}/pkgconfig ${CMAKE_INSTALL_PREFIX})
configure_file(cmake/bluebus.pc.in ${CMAKE_CURRENT_BINARY_DIR}/bluebus.pc @ONLY)
install(FILES ${CMAKE_CURRENT_BINARY_DIR}/bluebus.pc DESTINATION ${CMAKE_INSTALL_LIBDIR}/pkgconfig)

# ---------------------------------------------------------------------------
# Examples and tests
# ---------------------------------------------------------------------------
if(BLUEBUS_EXAMPLES)
  add_executable(bluebus_scan examples/scan.c)
  add_executable(bluebus_peripheral examples/peripheral.c)
  add_executable(bluebus_central examples/central.cpp)
  foreach(t bluebus_scan bluebus_peripheral bluebus_central)
    target_link_libraries(${t} PRIVATE bluebus::bluebus)
  endforeach()
  set_target_properties(bluebus_scan bluebus_peripheral PROPERTIES C_STANDARD 99)
  set_target_properties(bluebus_central PROPERTIES CXX_STANDARD 17)
endif()

if(BLUEBUS_TESTS)
  enable_testing()
  add_executable(bluebus_test_c tests/test.c)
  add_executable(bluebus_test_c_static tests/test.c)
  add_executable(bluebus_test_cpp tests/test.cpp)
  target_link_libraries(bluebus_test_c PRIVATE bluebus::shared)
  target_link_libraries(bluebus_test_c_static PRIVATE bluebus::static)
  target_link_libraries(bluebus_test_cpp PRIVATE bluebus::bluebus)
  set_target_properties(bluebus_test_c bluebus_test_c_static PROPERTIES C_STANDARD 99
                        C_STANDARD_REQUIRED ON C_EXTENSIONS OFF)
  set_target_properties(bluebus_test_cpp PROPERTIES CXX_STANDARD 17 CXX_STANDARD_REQUIRED ON)
  foreach(t bluebus_test_c bluebus_test_c_static bluebus_test_cpp)
    target_compile_options(${t} PRIVATE -Wall -Wextra -Wpedantic -Werror)
  endforeach()

  # The mock BlueZ runner (a Rust program from capi/tests/support).
  set(_mock ${_target_dir}/debug/examples/mock_bluez)
  add_custom_command(
    OUTPUT ${_mock}
    COMMAND ${CARGO} build -p bluebus-c --example mock_bluez --target-dir ${_target_dir}
    WORKING_DIRECTORY ${_root}
    DEPENDS ${_sources} ${CMAKE_CURRENT_SOURCE_DIR}/tests/support/mod.rs
            ${CMAKE_CURRENT_SOURCE_DIR}/tests/support/mock_bluez.rs ${_root}/tests/common/mod.rs
    COMMENT "Building the mock BlueZ (cargo)"
    USES_TERMINAL)
  add_custom_target(bluebus_mock ALL DEPENDS ${_mock})

  foreach(t bluebus_test_c bluebus_test_c_static bluebus_test_cpp)
    add_test(NAME ${t} COMMAND ${_mock} $<TARGET_FILE:${t}>)
  endforeach()
  add_test(NAME bluebus_example_scan COMMAND ${_mock} $<TARGET_FILE:bluebus_scan> hci0 300)
  set_tests_properties(bluebus_example_scan PROPERTIES PASS_REGULAR_EXPRESSION "found +11:22:33:44:55:66")
endif()
