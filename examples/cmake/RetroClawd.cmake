# RetroClawd.cmake: build retro-machine images with the retroclawd tools from CMake.
#
#   list(APPEND CMAKE_MODULE_PATH ".../examples/cmake")
#   include(RetroClawd)
#
# The tools (rcasm, rcar, rcld, rcb) are found on PATH, or in RETROCLAWD_TOOLS_DIR (a cache variable or environment variable).
#
#   retroclawd_object(<out.rco> SOURCE <file> CPU <6502|z80|huc6280|65816|spc700> [DEFINES A=1 B=2] [INCLUDES dir...])
#   retroclawd_library(<out.rca> OBJECTS <a.rco> <b.rco> ...)
#   retroclawd_image(<target> LAYOUT <layout.toml> OUTPUT <image> [DEPENDS files...] [MAP file] [SYMBOLS file])
#   retroclawd_project(<target> DIRECTORY <dir with rcb.toml> OUTPUT <image> [DEPENDS files...])
#
# Each makes a custom command (re-run when its inputs change) and the *_image / *_project forms a target of that name.

if(NOT DEFINED RETROCLAWD_TOOLS_DIR AND DEFINED ENV{RETROCLAWD_TOOLS_DIR})
  set(RETROCLAWD_TOOLS_DIR "$ENV{RETROCLAWD_TOOLS_DIR}")
endif()
set(RETROCLAWD_TOOLS_DIR "${RETROCLAWD_TOOLS_DIR}" CACHE PATH "Folder with rcasm, rcar, rcld and rcb (default: the PATH)")

foreach(_tool rcasm rcar rcld rcb)
  string(TOUPPER "${_tool}" _var)
  find_program(RETROCLAWD_${_var} ${_tool} HINTS "${RETROCLAWD_TOOLS_DIR}")
endforeach()

function(retroclawd_object OUT)
  cmake_parse_arguments(A "" "SOURCE;CPU" "DEFINES;INCLUDES" ${ARGN})
  if(NOT RETROCLAWD_RCASM)
    message(FATAL_ERROR "rcasm not found: install the retroclawd tools or set RETROCLAWD_TOOLS_DIR")
  endif()
  set(_args -c ${A_CPU} -f object ${A_SOURCE} -o ${OUT} --force)
  foreach(_d ${A_DEFINES})
    list(APPEND _args -D ${_d})
  endforeach()
  foreach(_i ${A_INCLUDES})
    list(APPEND _args -I ${_i})
  endforeach()
  add_custom_command(OUTPUT ${OUT}
    COMMAND ${RETROCLAWD_RCASM} ${_args}
    DEPENDS ${A_SOURCE}
    COMMENT "rcasm ${A_SOURCE}"
    VERBATIM)
endfunction()

function(retroclawd_library OUT)
  cmake_parse_arguments(A "" "" "OBJECTS" ${ARGN})
  add_custom_command(OUTPUT ${OUT}
    COMMAND ${RETROCLAWD_RCAR} create ${OUT} ${A_OBJECTS} --force
    DEPENDS ${A_OBJECTS}
    COMMENT "rcar ${OUT}"
    VERBATIM)
endfunction()

function(retroclawd_image TARGET)
  cmake_parse_arguments(A "" "LAYOUT;OUTPUT;MAP;SYMBOLS" "DEPENDS" ${ARGN})
  set(_extra)
  if(A_MAP)
    list(APPEND _extra --map ${A_MAP})
  endif()
  if(A_SYMBOLS)
    list(APPEND _extra --symbols ${A_SYMBOLS})
  endif()
  add_custom_command(OUTPUT ${A_OUTPUT}
    COMMAND ${RETROCLAWD_RCLD} build ${A_LAYOUT} -o ${A_OUTPUT} ${_extra} --force
    DEPENDS ${A_LAYOUT} ${A_DEPENDS}
    COMMENT "rcld ${A_LAYOUT}"
    VERBATIM)
  add_custom_target(${TARGET} ALL DEPENDS ${A_OUTPUT})
endfunction()

function(retroclawd_project TARGET)
  cmake_parse_arguments(A "" "DIRECTORY;OUTPUT" "DEPENDS" ${ARGN})
  file(GLOB_RECURSE _sources CONFIGURE_DEPENDS "${A_DIRECTORY}/src/*")
  add_custom_command(OUTPUT ${A_OUTPUT}
    COMMAND ${RETROCLAWD_RCB} -C ${A_DIRECTORY} build
    DEPENDS ${A_DIRECTORY}/rcb.toml ${_sources} ${A_DEPENDS}
    COMMENT "rcb ${A_DIRECTORY}"
    VERBATIM)
  add_custom_target(${TARGET} ALL DEPENDS ${A_OUTPUT})
endfunction()
