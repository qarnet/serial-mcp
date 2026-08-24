cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(serial-mcp-test-fw)

target_include_directories(app PRIVATE src)
target_compile_definitions(app PRIVATE _GNU_SOURCE)

FILE(GLOB app_sources src/*.c)
target_sources(app PRIVATE ${app_sources})

# Keep native_simulator host-side build warnings quiet.
#
# The native_simulator runner is built by a separate GNU Makefile in
# scripts/native_simulator/Makefile. It reads nsi_config generated from the
# `native_simulator` INTERFACE target properties in natsim_config.cmake.
#
# The NixOS GCC wrapper unconditionally adds -D_FORTIFY_SOURCE=3. The NSI
# Makefile defaults to NSI_OPT=-O0 for debugging, so glibc emits a warning for
# each host-side source unless an optimization flag is present. Append -O2
# after NSI_OPT so the last -O flag wins and fortify is satisfied.
#
# The 32-bit (-m32) runner link scans default 64-bit library paths first and
# skips those entries before finding the 32-bit libraries. Suppress that
# cosmetic linker warning with --no-warn-search-mismatch; the correct
# 32-bit libraries are still linked.
if(NOT CONFIG_64BIT)
  target_compile_options(native_simulator INTERFACE -O2)
  target_link_options(native_simulator INTERFACE -Wl,--no-warn-search-mismatch)
endif()
