# SPDX-License-Identifier: Apache-2.0
# While another process holds OUTPUT.parts/lock, --resume on the same
# OUTPUT fails with exit 7 and leaves the directory's contents alone.
file(REMOVE_RECURSE "${WORK}")
file(MAKE_DIRECTORY "${WORK}/out.mp4.parts")
file(WRITE "${WORK}/out.mp4.parts/seg-000000.mp4" "someone else's segment")
# flock(1) takes an exclusive flock on the lock file and runs the render
# while holding it.
execute_process(COMMAND ${FLOCK} -x "${WORK}/out.mp4.parts/lock"
                ${CMD} --scene ${SCENE} --output ${WORK}/out.mp4 --resume
                --segment-frames 5
                RESULT_VARIABLE rc OUTPUT_VARIABLE o ERROR_VARIABLE e)
if(NOT rc STREQUAL "7")
  message(FATAL_ERROR "exit ${rc}, expected 7\n${o}\n${e}")
endif()
if(NOT e MATCHES "another render is using")
  message(FATAL_ERROR "no lock diagnostic:\n${e}")
endif()
file(READ "${WORK}/out.mp4.parts/seg-000000.mp4" kept)
if(NOT kept STREQUAL "someone else's segment" OR EXISTS "${WORK}/out.mp4")
  message(FATAL_ERROR "the locked-out run touched the other render's files")
endif()
