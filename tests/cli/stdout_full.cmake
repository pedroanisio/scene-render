# SPDX-License-Identifier: Apache-2.0
# Output that cannot be delivered is an I/O error (exit 7), not success:
# validation summary, --hash lines, the preview line and --version, each
# with standard output on /dev/full.
if(NOT EXISTS /dev/full)
  message(STATUS "no /dev/full; skipped")
  return()
endif()
file(REMOVE_RECURSE "${WORK}")
file(MAKE_DIRECTORY "${WORK}")
foreach(case IN ITEMS validate hash preview version)
  if(case STREQUAL "validate")
    set(args --scene ${SCENE} --validate)
  elseif(case STREQUAL "hash")
    set(args --scene ${SCENE} --hash --frame-range 0:2)
  elseif(case STREQUAL "preview")
    set(args --scene ${SCENE} --frame 1 --preview-out ${WORK}/p.png)
  else()
    set(args --version)
  endif()
  execute_process(COMMAND ${CMD} ${args} OUTPUT_FILE /dev/full
                  RESULT_VARIABLE rc ERROR_VARIABLE e)
  if(NOT rc STREQUAL "7")
    message(FATAL_ERROR "${case}: exit ${rc}, expected 7\n${e}")
  endif()
endforeach()
