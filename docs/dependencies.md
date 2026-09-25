# Dependencies and licenses

## Required

| Component | Exact verified version | Use | License |
|---|---:|---|---|
| Expat / `libexpat1-dev` | 2.6.1-2ubuntu0.3 | XML tokenizer through its C API | MIT |
| FFmpeg | 6.1.1-3ubuntu5 | image/video/audio decode, scaling, H.264/H.265/FFV1/AAC encode, MP4/Matroska mux | LGPL-2.1-or-later normally; the verified binary is GPL-enabled |
| libx264 | 0.164.3108+git31e19f9-1 | H.264 encoder exposed by verified FFmpeg | GPL-2.0-or-later |
| libx265 | 3.5-2build1 | H.265 encoder exposed by verified FFmpeg | GPL-2.0-or-later |
| POSIX threads/libc | Ubuntu 24.04 system ABI | deterministic viewport workers, processes, pipes, clocks | system-library terms (glibc LGPL-2.1-or-later) |

FFmpeg is executed as a separate supervised process and is not linked into the
engine. The actual legal terms of an FFmpeg installation depend on its build
configuration and enabled codecs. The verified binary reports `--enable-gpl`,
`--enable-libx264`, and `--enable-libx265`; distributors must review their own
binary and patent obligations.

## Build and test tools

| Component | Exact verified version | License / note |
|---|---:|---|
| GCC | 13.3.0 | GPL-3.0-or-later with GCC Runtime Library Exception |
| GNU Make | 4.3-4.1build2 | GPL-3.0-or-later |
| CMake | minimum 3.20 | BSD-3-Clause; configuration supplied, unavailable in the verification container |
| Python | system Python 3 | PSF-2.0; integration-test driver only |
| lxml | 6.1.1 | BSD-3-Clause; XSD integration test only |

No third-party source is copied into the repository. The generated sample MP4
and WAV contain only FFmpeg `testsrc2` and `sine` signals and no third-party
creative media.

## Original code

All original project source, schema, tests, examples, and documentation are
licensed under Apache License 2.0 in the repository `LICENSE` file.
