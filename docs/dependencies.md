# Dependencies and licenses

These are the exact versions used for the v1.1 verification on Ubuntu 24.04
x86-64. Expat, libxml2, the FFmpeg libraries (libavformat, libavcodec, libavutil,
libswscale, libswresample), pthreads, `libm`, and `libdl` are linked through
their C APIs; all video/audio/image decoding and encoding runs in-process.
Text uses FreeType, HarfBuzz, FriBidi and Fontconfig through their C APIs,
also in-process; the engine starts no child processes. OpenCL is loaded
dynamically, so it is optional at runtime.

## Runtime and linked components

| Component | Exact verified version | Use | License |
|---|---:|---|---|
| Expat / `libexpat1-dev` | 2.6.1-2ubuntu0.3 | XML tokenization through the C API | MIT |
| libxml2 (`libxml-2.0`) | ≥ 2.9.14 (pkg-config and a compile-time check; older releases can load external parameter entities); verified with 2.14.6 (Freedesktop SDK 25.08); callbacks handle both the pre-2.12 `xmlErrorPtr` and the 2.12+ `const xmlError *` signatures | runtime XSD validation of scene documents against the embedded `schema/scene-v1.xsd` (`src/xml_schema.c`); no network, no external entities | MIT |
| FFmpeg libraries | libavformat ≥ 60, libavcodec ≥ 60, libavutil ≥ 58, libswscale ≥ 7, libswresample ≥ 4 (pkg-config); P1/P2 verified with 61.7.100 / 61.19.101 / 59.39.100 / 8.3.100 / 5.3.100 (Freedesktop SDK 25.08) | in-process demux/decode, swscale color conversion, swresample, H.264/H.265/FFV1/AAC encode, PNG preview encode, MP4/Matroska mux | LGPL-2.1-or-later normally; GPL when built with libx264/libx265 |
| libx264 | 0.164.3108+git31e19f9-1 | H.264 encoder exposed by libavcodec | GPL-2.0-or-later |
| libx265 | 3.5-2build1 | H.265 encoder exposed by libavcodec | GPL-2.0-or-later |
| OpenCL ICD loader | 2.3.2-1build1 | optional OpenCL 1.2 GPU discovery/kernel dispatch | BSD-2-Clause |
| FreeType (`freetype2`) | pkg-config any; P3 verified with 26.6.20 (FreeType 2.14, Freedesktop SDK 25.08) | unhinted outline loading and 8-bit anti-aliased glyph rasterization | FreeType License (FTL) or GPL-2.0-only |
| HarfBuzz | ≥ 2.8.2; P3 verified with 11.4.5 (Freedesktop SDK 25.08) | OpenType shaping: kerning, ligatures, complex-script forms, font metrics | Old MIT |
| FriBidi | ≥ 1.0; P3 verified with 1.0.16 (Freedesktop SDK 25.08) | UAX #9 bidirectional embedding levels | LGPL-2.1-or-later |
| Fontconfig | ≥ 2.13; P3 verified with 2.17.1 (Freedesktop SDK 25.08) | `font` family lookup (FcNameParse, FcConfigSubstitute, FcFontMatch) | Fontconfig license (MIT-style) |
| glibc / pthreads | 2.39-0ubuntu8.6 | allocation, threads, clocks, dynamic loading | LGPL-2.1-or-later plus system-library terms |

The engine never implements or embeds a production codec; it links the
FFmpeg libraries dynamically. Their legal terms depend on their build
configuration and enabled codecs (`libx264`/`libx265` make the build GPL);
distributors must review the libraries and patent obligations they ship.

The text engine (`src/text.c`) links FreeType, HarfBuzz, FriBidi and
Fontconfig directly. Rendered text depends on the fonts installed (for
`font` families) and on the exact FreeType/HarfBuzz versions; it is identical
for identical inputs and versions. A missing requested font or font file
produces a render error rather than a silent substitution.

## Build and test tools

| Component | Exact verified version | License / note |
|---|---:|---|
| GCC | 13.3.0 | GPL-3.0-or-later with GCC Runtime Library Exception |
| GNU Make | 4.3 | GPL-3.0-or-later |
| pkg-config / pkgconf | any | locates the FFmpeg, text and libxml2 libraries for CMake and Make |
| CMake | 3.30.5 | BSD-3-Clause; generated and test builds verified |
| Python | 3.12.14 | PSF-2.0; integration-test driver only |
| `sr-probe` (`tests/tools/sr-probe.c`) | built with the tests | stream inspection for the integration test; replaces `ffprobe` |
| lxml | 6.1.1 | BSD-3-Clause; XSD integration test only |
| `xmllint` (libxml2) | optional | cross-check of examples against the XSD in the integration script |
| `od`, `sed` | POSIX | the Makefile embeds the XSD with them (no CMake or `xxd` needed) |

No third-party source is copied into this repository. Generated sample MP4,
PNG, PPM, and WAV files contain only project-generated geometry or FFmpeg test
signals and no third-party creative media.

## Original code

All original source, schema, tests, examples, and documentation are licensed
under Apache License 2.0 in `LICENSE`.
