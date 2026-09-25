# SPDX-License-Identifier: Apache-2.0
# --hash is thread-invariant: 1 and 3 threads print identical frame and
# audio hashes for the same range.
set(args --scene ${SCENE} --hash --frame-range 2:6)
execute_process(COMMAND ${CMD} ${args} --threads 1 RESULT_VARIABLE rc1 OUTPUT_VARIABLE one)
execute_process(COMMAND ${CMD} ${args} --threads 3 RESULT_VARIABLE rc3 OUTPUT_VARIABLE three)
if(NOT rc1 EQUAL 0 OR NOT rc3 EQUAL 0)
  message(FATAL_ERROR "exit ${rc1}/${rc3}")
endif()
if(NOT one MATCHES "^2 [0-9a-f]+\n3 [0-9a-f]+\n4 [0-9a-f]+\n5 [0-9a-f]+\naudio [0-9a-f]+\n$")
  message(FATAL_ERROR "unexpected hash output:\n${one}")
endif()
if(NOT one MATCHES "^2 [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]\n")
  message(FATAL_ERROR "frame hash is not 16 hex digits:\n${one}")
endif()
if(NOT one STREQUAL three)
  message(FATAL_ERROR "hashes depend on the thread count:\n1: ${one}\n3: ${three}")
endif()
