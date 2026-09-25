# SPDX-License-Identifier: Apache-2.0
# The interruption hook exists only in scene-render-testhooks: the shipped
# binary must not even contain the name of its environment variable.
file(STRINGS "${CMD}" shipped REGEX "SR_TEST_ABORT_AFTER_SEGMENTS")
if(shipped)
  message(FATAL_ERROR "${CMD} contains the test hook: ${shipped}")
endif()
file(STRINGS "${HOOKS}" hooked REGEX "SR_TEST_ABORT_AFTER_SEGMENTS")
if(NOT hooked)
  message(FATAL_ERROR "${HOOKS} lacks the test hook (the check above proves nothing)")
endif()
