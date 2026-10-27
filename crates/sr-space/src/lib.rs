//! Time scales, celestial frames and JPL ephemerides: the sr-core draft "Add time scales, celestial frames and
//! JPL ephemerides" (`srep/srep-0000-time-frames.md`, SP1 of SREP 12).
//!
//! * [`time`]: the calendar, UTC to TAI to TT to TDB (NAIF `naif0012.tls`) and the frame clock of a `space`.
//! * [`spk`]: our own reader of JPL SPK files (DAF, type 2 Chebyshev segments, either byte order) pinned by a
//!   sha256 digest; positions through the chain of centres, velocities and light time.
//! * [`elements`]: the approximate Keplerian elements of Standish and Williams, the fallback without a file.
//! * [`sun`] and [`moon`]: the solar position algorithm and the low-precision moon of the physical-sky draft
//!   (`srep/srep-0000-physical-sky.md`).
//! * [`orbits`]: Kepler's problem by universal variables and the `orbit` element of the orbits draft
//!   (`srep/srep-0000-orbits.md`, SP4 of SREP 12).
//! * [`frames`]: the ICRF, the ecliptic of J2000 and the IAU 2009 body-fixed frames, the mapping into scene
//!   pixels and bodies placed at a frame's time.
//!
//! Positions are kilometres in the ICRF axes until [`Space::scene_from_icrf`] maps them; ephemeris time is
//! seconds of TDB past J2000. Every value is a pure function of its inputs, so a frame rendered alone equals the
//! frame reached by playing.

#![forbid(unsafe_code)]

pub mod elements;
pub mod frames;
pub mod moon;
pub mod orbits;
pub mod spk;
pub mod stars;
pub mod sun;
pub mod time;

pub use elements::{
    approximate_position, approximate_relative, element_table, solve_kepler, ElementTable, AU_KM, MAX_KEPLER_ITERATIONS,
};
pub use frames::{
    body_axes, body_fixed_from_icrf, determinant, ecliptic_from_icrf, mat_apply, mat_mul, place_at, place_body,
    pole_icrf, rotation_elements, rotation_known, transpose, Frame, Mat3, Placement, Positions, ROTATION_EARTH_W1,
    SCENE_FROM_ICRF_AXES,
};
pub use spk::{apparent_position, chebyshev, chebyshev_derivative, Segment, Spk, C_KM_S, MAX_LIGHT_TIME_ITERATIONS};
pub use time::{
    calendar, et_from_utc, julian_day, leap_seconds, parse_utc, tdb_minus_tt, Space, TimeScale, J2000_JD, LEAP_SECONDS,
    TT_MINUS_TAI,
};

/// The obliquity of the ecliptic at J2000, degrees.
pub const OBLIQUITY_J2000: f64 = 23.43928;

/// A document value or file the draft refuses.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SpaceError {
    /// An epoch, rate or frame clock out of rule 1 or 2.
    #[error("time: {0}")]
    Time(String),
    /// An ephemeris file or request out of rule 3 or 4.
    #[error("ephemeris: {0}")]
    Ephemeris(String),
    /// A body or year outside the approximate elements (rule 5).
    #[error("approximate elements: {0}")]
    Elements(String),
    /// A frame or mapping out of rule 6.
    #[error("frame: {0}")]
    Frame(String),
    /// A star catalogue or field out of the star-fields draft.
    #[error("stars: {0}")]
    Stars(String),
    /// A date or observer of a physical sky out of the physical-sky draft (SKY2, SKY3).
    #[error("sky: {0}")]
    Sky(String),
    /// An orbit, an N-body system or a step out of the orbits draft (OR1 to OR3).
    #[error("orbit: {0}")]
    Orbit(String),
}
