//! Rule 5: the approximate Keplerian elements of Standish and Williams (JPL, "Keplerian elements for approximate
//! positions of the major planets"): a (au), e, I, L, ϖ, Ω (degrees) and their rates per Julian century, mean
//! ecliptic and equinox of J2000.

use super::time::{calendar, J2000_JD};
use super::{SpaceError, OBLIQUITY_J2000};

/// Kepler's equation's iteration bound.
pub const MAX_KEPLER_ITERATIONS: usize = 50;
/// The astronomical unit in km.
pub const AU_KM: f64 = 149597870.7;

/// Which of the two published tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementTable {
    /// Table 1, 1800 AD to 2050 AD.
    Y1800To2050,
    /// Tables 2a and 2b, 3000 BC to 3000 AD.
    Bc3000ToAd3000,
}

type Row = [f64; 12];

/// Mercury, Venus, the Earth-Moon barycentre, Mars, Jupiter, Saturn, Uranus, Neptune (NAIF ids 1 to 8).
const TABLE_1: [Row; 8] = [
    [
        0.38709927,
        0.20563593,
        7.00497902,
        252.25032350,
        77.45779628,
        48.33076593,
        0.00000037,
        0.00001906,
        -0.00594749,
        149472.67411175,
        0.16047689,
        -0.12534081,
    ],
    [
        0.72333566,
        0.00677672,
        3.39467605,
        181.97909950,
        131.60246718,
        76.67984255,
        0.00000390,
        -0.00004107,
        -0.00078890,
        58517.81538729,
        0.00268329,
        -0.27769418,
    ],
    [
        1.00000261,
        0.01671123,
        -0.00001531,
        100.46457166,
        102.93768193,
        0.0,
        0.00000562,
        -0.00004392,
        -0.01294668,
        35999.37244981,
        0.32327364,
        0.0,
    ],
    [
        1.52371034,
        0.09339410,
        1.84969142,
        -4.55343205,
        -23.94362959,
        49.55953891,
        0.00001847,
        0.00007882,
        -0.00813131,
        19140.30268499,
        0.44441088,
        -0.29257343,
    ],
    [
        5.20288700,
        0.04838624,
        1.30439695,
        34.39644051,
        14.72847983,
        100.47390909,
        -0.00011607,
        -0.00013253,
        -0.00183714,
        3034.74612775,
        0.21252668,
        0.20469106,
    ],
    [
        9.53667594,
        0.05386179,
        2.48599187,
        49.95424423,
        92.59887831,
        113.66242448,
        -0.00125060,
        -0.00050991,
        0.00193609,
        1222.49362201,
        -0.41897216,
        -0.28867794,
    ],
    [
        19.18916464,
        0.04725744,
        0.77263783,
        313.23810451,
        170.95427630,
        74.01692503,
        -0.00196176,
        -0.00004397,
        -0.00242939,
        428.48202785,
        0.40805281,
        0.04240589,
    ],
    [
        30.06992276,
        0.00859048,
        1.77004347,
        -55.12002969,
        44.96476227,
        131.78422574,
        0.00026291,
        0.00005105,
        0.00035372,
        218.45945325,
        -0.32241464,
        -0.00508664,
    ],
];

const TABLE_2A: [Row; 8] = [
    [
        0.38709843,
        0.20563661,
        7.00559432,
        252.25166724,
        77.45771895,
        48.33961819,
        0.00000000,
        0.00002123,
        -0.00590158,
        149472.67486623,
        0.15940013,
        -0.12214182,
    ],
    [
        0.72332102,
        0.00676399,
        3.39777545,
        181.97970850,
        131.76755713,
        76.67261496,
        -0.00000026,
        -0.00005107,
        0.00043494,
        58517.81560260,
        0.05679648,
        -0.27274174,
    ],
    [
        1.00000018,
        0.01673163,
        -0.00054346,
        100.46691572,
        102.93005885,
        -5.11260389,
        -0.00000003,
        -0.00003661,
        -0.01337178,
        35999.37306329,
        0.31795260,
        -0.24123856,
    ],
    [
        1.52371243,
        0.09336511,
        1.85181869,
        -4.56813164,
        -23.91744784,
        49.71320984,
        0.00000097,
        0.00009149,
        -0.00724757,
        19140.29934243,
        0.45223625,
        -0.26852431,
    ],
    [
        5.20248019,
        0.04853590,
        1.29861416,
        34.33479152,
        14.27495244,
        100.29282654,
        -0.00002864,
        0.00018026,
        -0.00322699,
        3034.90371757,
        0.18199196,
        0.13024619,
    ],
    [
        9.54149883,
        0.05550825,
        2.49424102,
        50.07571329,
        92.86136063,
        113.63998702,
        -0.00003065,
        -0.00032044,
        0.00451969,
        1222.11494724,
        0.54179478,
        -0.25015002,
    ],
    [
        19.18797948,
        0.04685740,
        0.77298127,
        314.20276625,
        172.43404441,
        73.96250215,
        -0.00020455,
        -0.00001550,
        -0.00180155,
        428.49512595,
        0.09266985,
        0.05739699,
    ],
    [
        30.06952752,
        0.00895439,
        1.77005520,
        304.22289287,
        46.68158724,
        131.78635853,
        0.00006447,
        0.00000818,
        0.00022400,
        218.46515314,
        0.01009938,
        -0.00606302,
    ],
];

/// Table 2b: b, c, s, f of the mean anomaly of Jupiter to Neptune.
const TABLE_2B: [[f64; 4]; 4] = [
    [-0.00012452, 0.06064060, -0.35635438, 38.35125000],
    [0.00025899, -0.13434469, 0.87320147, 38.35125000],
    [0.00058331, -0.97731848, 0.17689245, 7.67025000],
    [-0.00041348, 0.68346318, -0.10162547, 7.67025000],
];

/// E − e* sin E = M in degrees, Newton's iterations from E₀ = M + e* sin M, bounded.
pub fn solve_kepler(mean_anomaly: f64, e: f64) -> Result<f64, SpaceError> {
    if !(0.0..1.0).contains(&e) {
        return Err(SpaceError::Elements(format!("the eccentricity is in [0, 1), not {e}")));
    }
    let m = (mean_anomaly + 180.0).rem_euclid(360.0) - 180.0;
    let e_star = e.to_degrees();
    let mut big_e = m + e_star * m.to_radians().sin();
    for _ in 0..MAX_KEPLER_ITERATIONS {
        let dm = m - (big_e - e_star * big_e.to_radians().sin());
        let de = dm / (1.0 - e * big_e.to_radians().cos());
        big_e += de;
        if de.abs() < 1e-11 {
            return Ok(big_e);
        }
    }
    Err(SpaceError::Elements("Kepler's equation did not converge".into()))
}

/// Heliocentric ICRF position (km) of a planet (1 to 8, 3 the Earth-Moon barycentre) at ephemeris time et.
pub fn approximate_position(body: i32, et: f64, table: ElementTable) -> Result<[f64; 3], SpaceError> {
    if !(1..=8).contains(&body) {
        return Err(SpaceError::Elements(format!("the approximate elements cover bodies 1 to 8, not {body}")));
    }
    let (rows, y0, y1) = match table {
        ElementTable::Y1800To2050 => (&TABLE_1, 1800, 2050),
        ElementTable::Bc3000ToAd3000 => (&TABLE_2A, -3000, 3000),
    };
    let year = calendar(J2000_JD + et / 86400.0).0;
    if !(y0..=y1).contains(&year) {
        return Err(SpaceError::Elements(format!("the element table {table:?} does not cover the year {year}")));
    }
    let row = &rows[body as usize - 1];
    let t = et / (86400.0 * 36525.0);
    let el = |i: usize| row[i] + row[i + 6] * t;
    let (a, e, inc, big_l, peri, node) = (el(0), el(1), el(2), el(3), el(4), el(5));
    let omega = peri - node;
    let mut m = big_l - peri;
    if table == ElementTable::Bc3000ToAd3000 && body >= 5 {
        let [b, c, s, f] = TABLE_2B[body as usize - 5];
        m += b * t * t + c * (f * t).to_radians().cos() + s * (f * t).to_radians().sin();
    }
    let big_e = solve_kepler(m, e)?.to_radians();
    let xp = a * (big_e.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * big_e.sin();
    let (sw, cw) = omega.to_radians().sin_cos();
    let (so, co) = node.to_radians().sin_cos();
    let (si, ci) = inc.to_radians().sin_cos();
    let x = (cw * co - sw * so * ci) * xp + (-sw * co - cw * so * ci) * yp;
    let y = (cw * so + sw * co * ci) * xp + (-sw * so + cw * co * ci) * yp;
    let z = (sw * si) * xp + (cw * si) * yp;
    let (se, ce) = OBLIQUITY_J2000.to_radians().sin_cos();
    Ok([x * AU_KM, (ce * y - se * z) * AU_KM, (se * y + ce * z) * AU_KM])
}

/// The table rule 5 reads at ephemeris time et: Table 1 for the years 1800 to 2050, Tables 2a and 2b elsewhere from
/// −3000 to 3000; a year outside both is an error.
pub fn element_table(et: f64) -> Result<ElementTable, SpaceError> {
    let year = calendar(J2000_JD + et / 86400.0).0;
    if (1800..=2050).contains(&year) {
        Ok(ElementTable::Y1800To2050)
    } else if (-3000..=3000).contains(&year) {
        Ok(ElementTable::Bc3000ToAd3000)
    } else {
        Err(SpaceError::Elements(format!("the approximate elements cover the years -3000 to 3000, not {year}")))
    }
}

/// A body's ICRF position (km) about an origin from the approximate elements at ephemeris time et: the heliocentric
/// difference, the Sun (10) and the barycentre (0, taken as the Sun) at zero for either term, a planet 1 to 8
/// otherwise, with the table [`element_table`] picks for et.
pub fn approximate_relative(body: i32, origin: i32, et: f64) -> Result<[f64; 3], SpaceError> {
    let table = element_table(et)?;
    let helio = |b: i32| match b {
        0 | 10 => Ok([0.0; 3]),
        _ => approximate_position(b, et, table),
    };
    let (p, o) = (helio(body)?, helio(origin)?);
    Ok([p[0] - o[0], p[1] - o[1], p[2] - o[2]])
}
