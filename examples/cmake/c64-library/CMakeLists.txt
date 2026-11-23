cmake_minimum_required(VERSION 3.16)
project(c64_library NONE)

list(APPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_SOURCE_DIR}/..")
include(RetroClawd)

set(OBJ_DIR "${CMAKE_CURRENT_BINARY_DIR}")

# Each source becomes an object; the screen routines go into a library; rcld links the program from the object
# and pulls library members in only when they are needed.
retroclawd_object(${OBJ_DIR}/main.rco   SOURCE ${CMAKE_CURRENT_SOURCE_DIR}/src/main.asm   CPU 6502)
retroclawd_object(${OBJ_DIR}/screen.rco SOURCE ${CMAKE_CURRENT_SOURCE_DIR}/src/screen.asm CPU 6502)
retroclawd_library(${OBJ_DIR}/screen.rca OBJECTS ${OBJ_DIR}/screen.rco)

configure_file(layout.toml.in ${OBJ_DIR}/layout.toml @ONLY)
retroclawd_image(hello_prg
  LAYOUT  ${OBJ_DIR}/layout.toml
  OUTPUT  ${OBJ_DIR}/hello.prg
  MAP     ${OBJ_DIR}/hello.map
  SYMBOLS ${OBJ_DIR}/hello.sym
  DEPENDS ${OBJ_DIR}/main.rco ${OBJ_DIR}/screen.rca)
