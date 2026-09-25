# SPDX-License-Identifier: Apache-2.0
# --physics-cache DIR: the first run simulates and stores one cache file in
# DIR (instead of the XML <physics cache> path); the second restores it
# without simulating and produces identical frames.
file(REMOVE_RECURSE "${WORK}")
file(MAKE_DIRECTORY "${WORK}")
set(args --scene ${SCENE} --hash --frame-range 0:6 --metrics --physics-cache ${WORK}/cache)
execute_process(COMMAND ${CMD} ${args} RESULT_VARIABLE rc OUTPUT_VARIABLE h1 ERROR_VARIABLE m1)
if(NOT rc EQUAL 0 OR NOT m1 MATCHES "physics: steps=[1-9][0-9]* cache_hit=0")
  message(FATAL_ERROR "first run: exit ${rc}\n${m1}")
endif()
file(GLOB files "${WORK}/cache/physics-*.bin")
list(LENGTH files n)
if(NOT n EQUAL 1)
  message(FATAL_ERROR "expected one cache file in ${WORK}/cache, found ${n}")
endif()
execute_process(COMMAND ${CMD} ${args} RESULT_VARIABLE rc OUTPUT_VARIABLE h2 ERROR_VARIABLE m2)
if(NOT rc EQUAL 0 OR NOT m2 MATCHES "physics: steps=0 cache_hit=1")
  message(FATAL_ERROR "second run did not hit the cache: exit ${rc}\n${m2}")
endif()
if(NOT h1 STREQUAL h2)
  message(FATAL_ERROR "cached frames differ from simulated frames")
endif()
