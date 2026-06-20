cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(serial-mcp-test-fw)

target_include_directories(app PRIVATE src)
target_compile_definitions(app PRIVATE _GNU_SOURCE)

FILE(GLOB app_sources src/*.c)
target_sources(app PRIVATE ${app_sources})

# ── Silence native_simulator host-side build warnings ──────────────────────
#
# The native_simulator runner is built by a separate GNU Makefile
# (scripts/native_simulator/Makefile), fed by nsi_config generated from the
# `native_simulator` INTERFACE target properties (natsim_config.cmake).
#
# 1. _FORTIFY_SOURCE requires optimization (-O):
#    NixOS' gcc wrapper unconditionally injects -D_FORTIFY_SOURCE=3, but the NSI
#    Makefile defaults to NSI_OPT=-O0 "to ease debugging", so glibc emits a
#    #warning for every host-side source. Append -O2 *after* NSI_OPT so the
#    last -O flag wins and fortify is satisfied.
#
# 2. "skipping incompatible ... when searching for -lX" linker noise:
#    The 32-bit (-m32) runner link scans default (64-bit) library search paths
#    first and skips the 64-bit .so entries before finding the 32-bit ones.
#    --no-warn-search-mismatch suppresses this cosmetic noise; the correct
#    32-bit libraries are still linked.
if(NOT CONFIG_64BIT)
  target_compile_options(native_simulator INTERFACE -O2)
  target_link_options(native_simulator INTERFACE -Wl,--no-warn-search-mismatch)
endif()