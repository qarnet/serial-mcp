cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(serial-mcp-test-fw)

target_include_directories(app PRIVATE src)
target_compile_definitions(app PRIVATE _GNU_SOURCE)

FILE(GLOB app_sources src/*.c)
target_sources(app PRIVATE ${app_sources})