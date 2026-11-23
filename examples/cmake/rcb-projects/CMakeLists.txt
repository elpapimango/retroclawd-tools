cmake_minimum_required(VERSION 3.16)
project(retroclawd_examples NONE)

list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/..")
include(RetroClawd)

# Every machine's example project is an rcb project (a folder with rcb.toml); CMake only decides when to rebuild.
set(EX ${CMAKE_CURRENT_SOURCE_DIR}/../..)
retroclawd_project(nes_hello     DIRECTORY ${EX}/nes-hello     OUTPUT ${EX}/nes-hello/build/nes-hello.nes)
retroclawd_project(snes_hello    DIRECTORY ${EX}/snes-hello    OUTPUT ${EX}/snes-hello/build/snes-hello.sfc)
retroclawd_project(genesis_hello DIRECTORY ${EX}/genesis-hello OUTPUT ${EX}/genesis-hello/build/genesis-hello.md)
retroclawd_project(pce_hello     DIRECTORY ${EX}/pcengine-hello OUTPUT ${EX}/pcengine-hello/build/pcengine-hello.pce)
