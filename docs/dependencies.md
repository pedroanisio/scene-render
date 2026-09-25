# Dependencies and licenses

These are the exact versions used for the v1.1 verification on Ubuntu 24.04
x86-64. Expat, the FFmpeg libraries (libavformat, libavcodec, libavutil,
libswscale, libswresample), pthreads, `libm`, and `libdl` are linked through
their C APIs; all video/audio/image decoding and encoding runs in-process. The
`ffmpeg` executable is still run as a child process for text assets only
(drawtext). OpenCL is loaded dynamically, so it is optional at runtime.

## Runtime and linked components

| Component | Exact verified version | Use | License |
|---|---:|---|---|
| Expat / `libexpat1-dev` | 2.6.1-2ubuntu0.3 | XML tokenization through the C API | MIT |
| FFmpeg libraries | libavformat ≥ 60, libavcodec ≥ 60, libavutil ≥ 58, libswscale ≥ 7, libswresample ≥ 4 (pkg-config); P1/P2 verified with 61.7.100 / 61.19.101 / 59.39.100 / 8.3.100 / 5.3.100 (Freedesktop SDK 25.08) | in-process demux/decode, swscale color conversion, swresample, H.264/H.265/FFV1/AAC encode, PNG preview encode, MP4/Matroska mux | LGPL-2.1-or-later normally; GPL when built with libx264/libx265 |
| FFmpeg executable | 6.1.1-3ubuntu5 | drawtext text rendering only | as above |
| libx264 | 0.164.3108+git31e19f9-1 | H.264 encoder exposed by libavcodec | GPL-2.0-or-later |
| libx265 | 3.5-2build1 | H.265 encoder exposed by libavcodec | GPL-2.0-or-later |
| OpenCL ICD loader | 2.3.2-1build1 | optional OpenCL 1.2 GPU discovery/kernel dispatch | BSD-2-Clause |
| Fontconfig | 2.15.0-1.1ubuntu2 | FFmpeg drawtext font matching | Fontconfig license (MIT-style) |
| FreeType | 2.13.2+dfsg-1ubuntu0.1 | FFmpeg drawtext glyph rasterization | FreeType License or GPL-2.0-only |
| FriBidi | 1.0.13-3build1 | FFmpeg drawtext bidirectional layout | LGPL-2.1-or-later |
| HarfBuzz | 8.3.0-2build2 | FFmpeg drawtext shaping | Old MIT |
| glibc / pthreads | 2.39-0ubuntu8.6 | allocation, threads, the drawtext child process, clocks, dynamic loading | LGPL-2.1-or-later plus system-library terms |

The engine never implements or embeds a production codec; it links the
FFmpeg libraries dynamically. Their legal terms depend on their build
configuration and enabled codecs (`libx264`/`libx265` make the build GPL);
distributors must review the libraries and patent obligations they ship.

The text engine invokes FFmpeg's drawtext filter. UTF-8 shaping therefore
depends on the Fontconfig/FreeType/FriBidi/HarfBuzz capabilities in that FFmpeg
build. A missing requested font produces a render error rather than silently
changing the XML font file.

## Build and test tools

| Component | Exact verified version | License / note |
|---|---:|---|
| GCC | 13.3.0 | GPL-3.0-or-later with GCC Runtime Library Exception |
| GNU Make | 4.3 | GPL-3.0-or-later |
| pkg-config / pkgconf | any | locates the FFmpeg libraries for CMake and Make |
| CMake | 3.30.5 | BSD-3-Clause; generated and test builds verified |
| Python | 3.12.14 | PSF-2.0; integration-test driver only |
| `sr-probe` (`tests/tools/sr-probe.c`) | built with the tests | stream inspection for the integration test; replaces `ffprobe` |
| lxml | 6.1.1 | BSD-3-Clause; XSD integration test only |

No third-party source is copied into this repository. Generated sample MP4,
PNG, PPM, and WAV files contain only project-generated geometry or FFmpeg test
signals and no third-party creative media.

## Original code

All original source, schema, tests, examples, and documentation are licensed
under Apache License 2.0 in `LICENSE`.
