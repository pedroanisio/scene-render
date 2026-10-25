//! The numeric table of the time scales and ephemerides draft (sr-core srep/srep-0000-time-frames.md,
//! Conformance), row by row, against `sr_space`, and the failure paths of its rules.
//!
//! `fixtures/de440s-excerpt.bsp` and `fixtures/horizons-vectors.json` are byte-equal to
//! conformance/assets/ in sr-core at f6cf12f (MIT, same author; the excerpt is written from NAIF's
//! public-domain de440s.bsp, the vectors are JPL Horizons output). The row "meridian minus gast" compares
//! the IAU Earth with the physical sky draft's sidereal time and is checked with that draft's code.

mod reference;
mod spk_files;
