cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(serial-mcp-test-fw)

target_include_directories(app PRIVATE src)
target_compile_definitions(app PRIVATE _GNU_SOURCE)

# Override the hardcoded USBIP_PORT (3240) to avoid conflicts with
# system USB/IP servers (e.g. usbip-wsl2-attach on port 3240).  The
# usbip client uses --tcp-port 3241 to match.
zephyr_compile_definitions(USBIP_PORT=3241)

FILE(GLOB app_sources src/*.c)
target_sources(app PRIVATE ${app_sources})