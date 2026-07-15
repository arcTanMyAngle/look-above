//! The normalized vocabulary the whole pipeline speaks.
//!
//! Every source adapter converts into these types and nothing downstream knows
//! that sources exist (docs/09). Newtypes carry their invariants: an [`Icao24`]
//! is always 6 hex digits, a [`CallSign`] is always non-empty and trimmed.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Seconds since the Unix epoch, as reported by the source (never receipt time).
///
/// Signed to match `SQLite`'s `INTEGER` columns (docs/08) and to keep subtraction
/// total — position ages are routinely computed as differences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixSeconds(pub i64);

impl UnixSeconds {
    /// Seconds elapsed from `self` to `later`; negative if `later` precedes `self`.
    pub const fn seconds_until(self, later: Self) -> i64 {
        later.0 - self.0
    }
}

impl fmt::Display for UnixSeconds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A 24-bit ICAO aircraft address — the pipeline's identity key.
///
/// Stored as raw bytes rather than text so equality and hashing are case-safe:
/// feeds disagree on hex casing for the same aircraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Icao24([u8; 3]);

/// Why a string could not be read as an ICAO24 address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum Icao24ParseError {
    /// Not exactly 6 hex digits (after trimming surrounding whitespace).
    #[error("ICAO24 address must be 6 hex digits, got {got}")]
    InvalidLength { got: usize },
    /// A character outside `[0-9a-fA-F]`.
    #[error("ICAO24 address contains non-hex character {found:?}")]
    InvalidHexDigit { found: char },
}

impl Icao24 {
    /// Parses the canonical 6-hex-digit form, case-insensitively; surrounding
    /// whitespace is trimmed (`OpenSky` right-pads some fields).
    ///
    /// Deliberately strict otherwise: readsb feeds prefix non-ICAO (TIS-B / ADS-R)
    /// addresses with `~`, and those are *not* aircraft addresses. Rejecting them
    /// here forces each adapter to decide explicitly (M1) rather than silently
    /// minting an identity for a synthetic target.
    pub fn from_hex(s: &str) -> Result<Self, Icao24ParseError> {
        let s = s.trim();

        let len = s.chars().count();
        if len != 6 {
            return Err(Icao24ParseError::InvalidLength { got: len });
        }

        // `len == 6` guarantees the zip fills every slot.
        let mut digits = [0u8; 6];
        for (slot, c) in digits.iter_mut().zip(s.chars()) {
            *slot = hex_digit(c)?;
        }

        Ok(Self([
            (digits[0] << 4) | digits[1],
            (digits[2] << 4) | digits[3],
            (digits[4] << 4) | digits[5],
        ]))
    }

    /// The address as big-endian bytes.
    pub const fn as_bytes(self) -> [u8; 3] {
        self.0
    }
}

fn hex_digit(c: char) -> Result<u8, Icao24ParseError> {
    let err = || Icao24ParseError::InvalidHexDigit { found: c };
    let value = c.to_digit(16).ok_or_else(err)?;
    // `to_digit(16)` yields 0..=15, so this conversion cannot fail.
    u8::try_from(value).map_err(|_| err())
}

impl fmt::Display for Icao24 {
    /// Lower-case hex — the canonical form for the `aircraft.icao24` key (docs/08).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c] = self.0;
        write!(f, "{a:02x}{b:02x}{c:02x}")
    }
}

impl FromStr for Icao24 {
    type Err = Icao24ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

/// A flight callsign, trimmed and guaranteed non-empty.
///
/// Absence is modelled by `Option<CallSign>` at the use site rather than by an
/// empty `CallSign`, so "no identity" cannot be confused with "blank identity"
/// (privacy rule 2.2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallSign(String);

impl CallSign {
    /// Trims `raw` and returns `None` when nothing is left.
    ///
    /// Feeds pad callsigns to 8 characters and send all-blank fields for targets
    /// broadcasting no identity, which is exactly the `None` case.
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CallSign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which authorized feed a record came from.
///
/// A closed enum, not a string: adding a source means adding a variant, which
/// forces the allowlist test (docs/10) and the budget logic to be updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceId {
    OpenSky,
    AirplanesLive,
    AdsbLol,
}

/// Rejects a `source` string that names no known feed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown source id {0:?}")]
pub struct UnknownSourceId(String);

impl SourceId {
    /// The stable wire/DB spelling stored in `positions.source` (docs/08).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenSky => "opensky",
            Self::AirplanesLive => "airplaneslive",
            Self::AdsbLol => "adsblol",
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceId {
    type Err = UnknownSourceId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "opensky" => Ok(Self::OpenSky),
            "airplaneslive" => Ok(Self::AirplanesLive),
            "adsblol" => Ok(Self::AdsbLol),
            other => Err(UnknownSourceId(other.to_owned())),
        }
    }
}

/// A geographic bounding box in degrees, with validated bounds.
///
/// v1 does not model antimeridian wrap: `lon_min <= lon_max` always holds, so a
/// box spanning ±180° must be split by the caller. Global queries are expressed
/// as `RegionQuery { bbox: None }`, not as a whole-world box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    lat_min: f64,
    lon_min: f64,
    lat_max: f64,
    lon_max: f64,
}

/// Why a set of corners is not a usable bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum BBoxError {
    #[error("latitude {value} is outside [-90, 90]")]
    LatitudeOutOfRange { value: f64 },
    #[error("longitude {value} is outside [-180, 180]")]
    LongitudeOutOfRange { value: f64 },
    #[error("latitude bounds inverted: min {min} exceeds max {max}")]
    InvertedLatitude { min: f64, max: f64 },
    #[error(
        "longitude bounds inverted: min {min} exceeds max {max} (antimeridian spans must be split)"
    )]
    InvertedLongitude { min: f64, max: f64 },
}

impl BBox {
    /// Builds a box from its corners, in the `lamin, lomin, lamax, lomax` order
    /// `OpenSky`'s `states/all` query uses (docs/09).
    ///
    /// NaN bounds are rejected: the range checks below are false for NaN.
    pub fn new(lat_min: f64, lon_min: f64, lat_max: f64, lon_max: f64) -> Result<Self, BBoxError> {
        for value in [lat_min, lat_max] {
            if !(-90.0..=90.0).contains(&value) {
                return Err(BBoxError::LatitudeOutOfRange { value });
            }
        }
        for value in [lon_min, lon_max] {
            if !(-180.0..=180.0).contains(&value) {
                return Err(BBoxError::LongitudeOutOfRange { value });
            }
        }
        if lat_min > lat_max {
            return Err(BBoxError::InvertedLatitude {
                min: lat_min,
                max: lat_max,
            });
        }
        if lon_min > lon_max {
            return Err(BBoxError::InvertedLongitude {
                min: lon_min,
                max: lon_max,
            });
        }
        Ok(Self {
            lat_min,
            lon_min,
            lat_max,
            lon_max,
        })
    }

    pub const fn lat_min(self) -> f64 {
        self.lat_min
    }

    pub const fn lon_min(self) -> f64 {
        self.lon_min
    }

    pub const fn lat_max(self) -> f64 {
        self.lat_max
    }

    pub const fn lon_max(self) -> f64 {
        self.lon_max
    }

    /// Inclusive containment test on both axes.
    pub fn contains(self, lat_deg: f64, lon_deg: f64) -> bool {
        (self.lat_min..=self.lat_max).contains(&lat_deg)
            && (self.lon_min..=self.lon_max).contains(&lon_deg)
    }
}

/// A normalized live position report — the only shape the pipeline ever sees.
///
/// Optional fields are genuinely absent upstream rather than defaulted: every
/// authorized feed nulls out velocity/heading/altitude for some records, and a
/// zero heading is not the same fact as an unknown heading.
#[derive(Debug, Clone, PartialEq)]
pub struct StateVector {
    pub icao24: Icao24,
    /// `None` when the feed reports no identity, or when the target is anonymous.
    pub callsign: Option<CallSign>,
    /// Source-reported time of applicability.
    pub ts: UnixSeconds,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub baro_alt_m: Option<f32>,
    pub velocity_ms: Option<f32>,
    /// True track, degrees clockwise from north.
    pub heading_deg: Option<f32>,
    pub vert_rate_ms: Option<f32>,
    pub on_ground: bool,
    /// PIA / blocked target: gates all enrichment (privacy rule 2.2). Sticky for
    /// a session — once set, a later identified record must not clear it (docs/10).
    pub anonymous: bool,
    pub source: SourceId,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Icao24 {
        Icao24::from_hex(s).expect("valid ICAO24 in test")
    }

    #[test]
    fn from_hex_parses_canonical_lowercase() {
        assert_eq!(hex("a1b2c3").as_bytes(), [0xa1, 0xb2, 0xc3]);
    }

    #[test]
    fn from_hex_is_case_insensitive() {
        assert_eq!(hex("A1B2C3"), hex("a1b2c3"));
        assert_eq!(hex("aB1c2D"), hex("Ab1C2d"));
    }

    #[test]
    fn from_hex_accepts_boundary_addresses() {
        assert_eq!(hex("000000").as_bytes(), [0x00, 0x00, 0x00]);
        assert_eq!(hex("ffffff").as_bytes(), [0xff, 0xff, 0xff]);
    }

    #[test]
    fn from_hex_trims_surrounding_whitespace() {
        assert_eq!(hex("  a1b2c3 "), hex("a1b2c3"));
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert_eq!(
            Icao24::from_hex("a1b2c"),
            Err(Icao24ParseError::InvalidLength { got: 5 })
        );
        assert_eq!(
            Icao24::from_hex("a1b2c3d"),
            Err(Icao24ParseError::InvalidLength { got: 7 })
        );
        assert_eq!(
            Icao24::from_hex(""),
            Err(Icao24ParseError::InvalidLength { got: 0 })
        );
        assert_eq!(
            Icao24::from_hex("   "),
            Err(Icao24ParseError::InvalidLength { got: 0 })
        );
    }

    #[test]
    fn from_hex_rejects_non_hex_characters() {
        assert_eq!(
            Icao24::from_hex("a1b2cg"),
            Err(Icao24ParseError::InvalidHexDigit { found: 'g' })
        );
        assert_eq!(
            Icao24::from_hex("0x1234"),
            Err(Icao24ParseError::InvalidHexDigit { found: 'x' })
        );
    }

    #[test]
    fn from_hex_rejects_tis_b_prefixed_addresses() {
        // readsb marks non-ICAO (TIS-B / ADS-R) targets with a `~` prefix; those
        // are not aircraft addresses and must not parse as one.
        assert_eq!(
            Icao24::from_hex("~ab1234"),
            Err(Icao24ParseError::InvalidLength { got: 7 })
        );
    }

    #[test]
    fn from_hex_rejects_non_ascii_digits_without_panicking() {
        // Multi-byte chars: byte length is 6 but this is 2 chars, not 6 digits.
        assert_eq!(
            Icao24::from_hex("ＡＢ"),
            Err(Icao24ParseError::InvalidLength { got: 2 })
        );
        // Six chars, one of them non-ASCII.
        assert!(matches!(
            Icao24::from_hex("a1b2c√"),
            Err(Icao24ParseError::InvalidHexDigit { found: '√' })
        ));
    }

    #[test]
    fn display_round_trips_through_from_hex_in_lowercase() {
        let addr = hex("A1B2C3");
        assert_eq!(addr.to_string(), "a1b2c3");
        assert_eq!(hex(&addr.to_string()), addr);
    }

    #[test]
    fn display_zero_pads_each_byte() {
        assert_eq!(hex("000a0b").to_string(), "000a0b");
    }

    #[test]
    fn from_str_matches_from_hex() {
        assert_eq!("a1b2c3".parse::<Icao24>(), Icao24::from_hex("a1b2c3"));
    }

    #[test]
    fn callsign_trims_and_rejects_blank() {
        assert_eq!(
            CallSign::new("DAL123  ").map(|c| c.as_str().to_owned()),
            Some("DAL123".to_owned())
        );
        assert_eq!(CallSign::new(""), None);
        assert_eq!(CallSign::new("        "), None);
    }

    #[test]
    fn source_id_round_trips_through_its_db_spelling() {
        for source in [
            SourceId::OpenSky,
            SourceId::AirplanesLive,
            SourceId::AdsbLol,
        ] {
            assert_eq!(source.as_str().parse(), Ok(source));
        }
        assert!("flightradar24".parse::<SourceId>().is_err());
    }

    #[test]
    fn bbox_accepts_a_valid_region_and_tests_containment() {
        let bbox = BBox::new(40.0, -75.0, 41.0, -73.0).expect("valid bbox in test");
        assert!(bbox.contains(40.5, -74.0));
        assert!(bbox.contains(40.0, -75.0), "bounds are inclusive");
        assert!(!bbox.contains(39.9, -74.0));
        assert!(!bbox.contains(40.5, -72.9));
    }

    #[test]
    fn bbox_rejects_out_of_range_and_inverted_bounds() {
        assert!(matches!(
            BBox::new(-91.0, 0.0, 10.0, 1.0),
            Err(BBoxError::LatitudeOutOfRange { .. })
        ));
        assert!(matches!(
            BBox::new(0.0, -181.0, 10.0, 1.0),
            Err(BBoxError::LongitudeOutOfRange { .. })
        ));
        assert!(matches!(
            BBox::new(10.0, 0.0, 0.0, 1.0),
            Err(BBoxError::InvertedLatitude { .. })
        ));
        // An antimeridian-spanning box must be split by the caller, not accepted.
        assert!(matches!(
            BBox::new(0.0, 170.0, 10.0, -170.0),
            Err(BBoxError::InvertedLongitude { .. })
        ));
    }

    #[test]
    fn bbox_rejects_nan_bounds() {
        assert!(matches!(
            BBox::new(f64::NAN, 0.0, 10.0, 1.0),
            Err(BBoxError::LatitudeOutOfRange { .. })
        ));
        assert!(matches!(
            BBox::new(0.0, f64::NAN, 10.0, 1.0),
            Err(BBoxError::LongitudeOutOfRange { .. })
        ));
    }

    #[test]
    fn unix_seconds_difference_is_signed() {
        assert_eq!(UnixSeconds(100).seconds_until(UnixSeconds(130)), 30);
        assert_eq!(UnixSeconds(130).seconds_until(UnixSeconds(100)), -30);
    }
}
