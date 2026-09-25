# Dependencies and licenses

Every library below is linked through its C API and runs inside the
`scene-render` process: XML validation and parsing, image/video/audio
decoding, encoding and muxing, and text layout. The engine never starts
another program. OpenCL is loaded with `dlopen` only when `--renderer gpu`
is requested, so it is optional at build time and at run time.

The versions are the ones the port was verified with: the Freedesktop SDK
25.08 Flatpak runtime (`org.freedesktop.Sdk//25.08`, x86-64), as reported by
`pkg-config --modversion` and by the libraries themselves at run time. The
minimums are the ones `CMakeLists.txt` and `Makefile` require.

## Linked libraries

| Component (pkg-config name) | Required | Verified version | Use | License |
|---|---|---|---|---|
| Expat (`expat`) | CMake `find_package(EXPAT 2.5)` | 2.7.1 | streaming XML tokenizer for the scene loader (`src/xml*.c`) | MIT |
| libxml2 (`libxml-2.0`) | ≥ 2.9.14 (older releases can load external parameter entities) | 2.14.6 | runtime XSD validation against the embedded `schema/scene-v1.xsd` (`src/xml_schema.c`); no network, no external entities, no DTD loading | MIT |
| libavformat (`libavformat`) | ≥ 60 | 61.7.100 (FFmpeg 7.1.3) | demuxing (images, video, audio), MP4/QuickTime/Matroska muxing | see FFmpeg below |
| libavcodec (`libavcodec`) | ≥ 60 | 61.19.101 | decoding; H.264 (libx264), H.265 (libx265), FFV1, AAC and PNG encoding | see FFmpeg below |
| libavutil (`libavutil`) | ≥ 58 | 59.39.100 | frames, options, spherical side data, logging | see FFmpeg below |
| libswscale (`libswscale`) | ≥ 7 | 8.3.100 | image resampling (Lanczos), RGB ↔ Y'CbCr with explicit matrix and range | see FFmpeg below |
| libswresample (`libswresample`) | ≥ 4 | 5.3.100 | audio resampling and channel layout conversion to the mix format | see FFmpeg below |
| FreeType (`freetype2`) | any | 26.6.20 (FreeType 2.14.3) | unhinted outline loading, 8-bit anti-aliased glyph rasterization | FreeType License (FTL) or GPL-2.0-only |
| HarfBuzz (`harfbuzz`) | ≥ 2.8.2 | 11.4.5 | OpenType shaping (kerning, ligatures, complex-script forms), font metrics | Old MIT |
| FriBidi (`fribidi`) | ≥ 1.0 | 1.0.16 (Unicode 16.0.0) | UAX #9 bidirectional levels | LGPL-2.1-or-later |
| Fontconfig (`fontconfig`) | ≥ 2.13 | 2.17.1 | `font` family lookup (`FcNameParse`, `FcConfigSubstitute`, `FcFontMatch`) | Fontconfig license (MIT-style) |
| glibc (libc, libm, libpthread, libdl) | POSIX.1-2008 | 2.42 | allocation, threads, clocks, files, `dlopen` | LGPL-2.1-or-later |
| OpenCL ICD loader (`OpenCL`, optional, `dlopen`) | OpenCL 1.2 | ocl-icd, `OpenCL.pc` 3.0 | optional GPU device probe and color-conversion kernel | BSD-2-Clause |

Libraries pulled in indirectly by the ones above (zlib 1.3.1 and libpng
1.6.58 through libavcodec, FreeType and Fontconfig; libx264 and libx265
through libavcodec) are not linked by the engine directly.

### FFmpeg libraries, libx264 and libx265

The FFmpeg libraries are LGPL-2.1-or-later as a project, but the license of
a given build is set by its configure flags: a build configured with
`--enable-gpl --enable-libx264 --enable-libx265` is GPL-2.0-or-later as a
whole, and so is a program distributed together with it. libavcodec reports
the terms of the build in use through `avcodec_license()` and its flags
through `avcodec_configuration()`.

The Freedesktop SDK 25.08 has two FFmpeg 7.1 builds. The one the engine is
compiled against (`/usr/lib/x86_64-linux-gnu`) has no H.264/H.265 encoders;
at run time the dynamic linker resolves the `codecs-extra` extension
(`/usr/lib/x86_64-linux-gnu/codecs-extra/lib`), configured with
`--enable-gpl --enable-libx264 --enable-libx265`; its `avcodec_license()`
is "GPL version 2 or later". That build provides `libx264` (libx264.so.165,
GPL-2.0-or-later), `libx265` (libx265.so.216, GPL-2.0-or-later), `ffv1`,
`aac` and `png`. With an LGPL FFmpeg build without those libraries, `codec`
values `h264` and `h265` fail when the encoder opens (exit code 6) and
`ffv1` still works.

The engine does not implement or embed a production codec. Distributors
must review the license and patent obligations of the FFmpeg build and the
encoders they ship.

### Text

Rendered text depends on the font files and on the exact FreeType and
HarfBuzz versions; it is identical for identical inputs and versions. For
`font` families it also depends on the installed fonts and the Fontconfig
configuration (in the SDK, `sans` resolves to
`/usr/share/fonts/dejavu/DejaVuSans.ttf`). A missing requested family or
font file is a render error, not a silent substitution.

## Build and test tools

| Tool | Verified version | Use | License |
|---|---|---|---|
| GCC | 15.2.0 | C17 compiler; `gcov` for `tools/coverage.py` | GPL-3.0-or-later with the GCC Runtime Library Exception |
| GNU Binutils (ld) | 2.47 | linking, `--wrap` for the test fault wrappers | GPL-3.0-or-later |
| CMake | 4.4.3 (≥ 3.20 required) | primary build and CTest | BSD-3-Clause |
| GNU Make | 4.4.1 | alternative build (`Makefile`) | GPL-3.0-or-later |
| pkg-config (pkgconf) | 2.5.1 | locates libav, text and libxml2 libraries | ISC |
| Python | 3.13.15 | `tools/coverage.py`; example-scene generators under `scripts/` | PSF-2.0 |
| `sr-probe` (`tests/tools/sr-probe.c`) | built with the tests | stream inspection in `tests/run-integration.sh` | Apache-2.0 (this project) |
| `xmllint` (libxml2) | optional | cross-check of the example scenes against the XSD in the integration script | MIT |
| `od`, `sed` | POSIX | the Makefile embeds the XSD with them | — |

## Vendored assets

No third-party source code is copied into this repository. Two third-party
asset sets are vendored with their license, source URL, version and
checksums beside the files:

- `assets/third-party/inter`: the Inter typeface, SIL Open Font License 1.1
  (used by example scenes and by the golden text scenes);
- `assets/third-party/dusk-parallax`: CC0 pixel art (used by
  `examples/dusk-parallax.xml` and `tests/golden/images.xml`).

Media under `examples/assets` and `tests/` (MP4, PNG, PPM, WAV, OBJ) is
generated by the project from test signals and original geometry.

## Original code

All original source, schema, tests, examples and documentation are licensed
under the Apache License 2.0 (`LICENSE`).
