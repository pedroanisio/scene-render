# SPDX-License-Identifier: Apache-2.0
# Segmented --resume: an interrupted run (SR_TEST_ABORT_AFTER_SEGMENTS kills
# the process after its first committed segment) is completed by a rerun
# that renders only the missing segments, and the result is byte-identical
# to an uninterrupted --resume render. Editing the scene, touching an asset
# or changing a setting discards every kept segment.
file(REMOVE_RECURSE "${WORK}")
file(MAKE_DIRECTORY "${WORK}/assets")
# A private copy of the scene and its assets, so both can be edited.
file(COPY "${ROOT}/examples/assets/clip.mp4" "${ROOT}/examples/assets/tone.wav"
     DESTINATION "${WORK}/assets")
file(READ "${SCENE}" xml)
string(REPLACE "../examples/assets/" "assets/" xml "${xml}")
file(WRITE "${WORK}/scene.xml" "${xml}")

function(render out expect_rc)
  execute_process(COMMAND ${CMAKE_COMMAND} -E env ${ENV_PREFIX}
                  ${CMD} --scene ${WORK}/scene.xml --output ${out} --resume
                  --segment-frames 5 --threads 2 --metrics --verbose ${ARGN}
                  RESULT_VARIABLE rc OUTPUT_VARIABLE o ERROR_VARIABLE e)
  if(NOT rc STREQUAL "${expect_rc}")
    message(FATAL_ERROR "${out}: exit ${rc}, expected ${expect_rc}\n${o}\n${e}")
  endif()
  set(log "${e}" PARENT_SCOPE)
endfunction()
function(expect_segments rendered reused)
  if(NOT log MATCHES "segments_rendered=${rendered} segments_reused=${reused}")
    message(FATAL_ERROR "expected ${rendered} rendered / ${reused} reused:\n${log}")
  endif()
endfunction()
function(same a b)
  execute_process(COMMAND ${CMAKE_COMMAND} -E compare_files "${a}" "${b}"
                  RESULT_VARIABLE diff)
  if(NOT diff EQUAL 0)
    message(FATAL_ERROR "${a} and ${b} differ")
  endif()
endfunction()

# Uninterrupted reference: 12 frames in segments of 5 -> 3 segments.
set(ENV_PREFIX)
render(${WORK}/ref.mp4 0)
expect_segments(3 0)
if(EXISTS "${WORK}/ref.mp4.parts")
  message(FATAL_ERROR "parts directory left behind after success")
endif()

# Interrupted after the first segment.
set(out ${WORK}/out.mp4)
execute_process(COMMAND ${CMAKE_COMMAND} -E env SR_TEST_ABORT_AFTER_SEGMENTS=1
                ${CMD} --scene ${WORK}/scene.xml --output ${out} --resume
                --segment-frames 5 --threads 2 RESULT_VARIABLE rc)
if(rc STREQUAL "0")
  message(FATAL_ERROR "the interrupted run succeeded")
endif()
file(GLOB committed "${out}.parts/seg-*.mp4")
list(LENGTH committed n)
if(NOT n EQUAL 1 OR NOT EXISTS "${out}.parts/seg-000000.mp4" OR
   NOT EXISTS "${out}.parts/manifest")
  message(FATAL_ERROR "unexpected parts after interruption: ${committed}")
endif()
# A half-written segment left by a crash is ignored and replaced.
file(WRITE "${out}.parts/seg-000001.part.mp4" "truncated")
set(ENV_PREFIX)
render(${out} 0 --keep-parts)
expect_segments(2 1)
same(${out} ${WORK}/ref.mp4)
if(EXISTS "${out}.parts/seg-000001.part.mp4")
  message(FATAL_ERROR "stale partial segment survived")
endif()
# Everything reused; still the same bytes.
render(${out} 0 --keep-parts)
expect_segments(0 3)
same(${out} ${WORK}/ref.mp4)

# Editing the scene discards every segment.
string(REPLACE "seed=\"7\"" "seed=\"8\"" edited "${xml}")
string(REPLACE "volume=\"0.25\"" "volume=\"0.5\"" edited "${edited}")
file(WRITE "${WORK}/scene.xml" "${edited}")
render(${out} 0 --keep-parts)
expect_segments(3 0)
if(NOT log MATCHES "manifest '[^']*' changed; discarded 3 old segment")
  message(FATAL_ERROR "no discard diagnostic:\n${log}")
endif()
# A touched asset (mtime) discards them too.
render(${out} 0 --keep-parts)
expect_segments(0 3)
execute_process(COMMAND ${CMAKE_COMMAND} -E sleep 1)
file(TOUCH "${WORK}/assets/clip.mp4")
render(${out} 0 --keep-parts)
expect_segments(3 0)
# So does a changed render setting.
render(${out} 0 --keep-parts --resolution 80x46)
expect_segments(3 0)
# Without --keep-parts the directory goes away after success.
render(${out} 0 --resolution 80x46)
expect_segments(0 3)
if(EXISTS "${out}.parts")
  message(FATAL_ERROR "parts directory left behind after success")
endif()
