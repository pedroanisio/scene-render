# SPDX-License-Identifier: Apache-2.0
execute_process(COMMAND "${CMD}" --print-schema
                RESULT_VARIABLE rc OUTPUT_VARIABLE embedded ERROR_VARIABLE err)
if(NOT rc EQUAL 0 OR err MATCHES "runtime error:|ERROR: AddressSanitizer")
    message(FATAL_ERROR "cannot print schema: ${rc}: ${err}")
endif()
file(READ "${SCHEMA}" source)
if(NOT embedded STREQUAL source)
    message(FATAL_ERROR "embedded schema differs from the 1.1 contract")
endif()
