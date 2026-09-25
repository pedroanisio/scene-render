# SPDX-License-Identifier: Apache-2.0
# Runs CMD with ARGS ("|"-separated) and fails unless the exit code equals
# EXPECT and, when given, stdout+stderr match OUTPUT_REGEX.
string(REPLACE "|" ";" argv "${ARGS}")
execute_process(COMMAND ${CMD} ${argv} RESULT_VARIABLE rc OUTPUT_VARIABLE out
                ERROR_VARIABLE err)
if(NOT rc STREQUAL "${EXPECT}")
  message(FATAL_ERROR "exit ${rc}, expected ${EXPECT}\nstdout: ${out}\nstderr: ${err}")
endif()
if(DEFINED OUTPUT_REGEX AND NOT "${out}${err}" MATCHES "${OUTPUT_REGEX}")
  message(FATAL_ERROR "output does not match '${OUTPUT_REGEX}'\nstdout: ${out}\nstderr: ${err}")
endif()
