//! `ts` values: whole-second UTC, written `2026-10-10T18:30:00Z` and nothing else
//! (DESIGN.md section 9.1). One instant has one spelling, so one document has one id.

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An instant at second precision.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ts(Timestamp);

impl Ts {
    /// Truncate `t` to the second. The writer's path. Refuses what the reader would refuse,
    /// so a written `ts` always reads back: years before 0000 have no four-digit spelling.
    pub fn from_timestamp(t: Timestamp) -> Result<Self, TsError> {
        let ts = Self(Timestamp::from_second(t.as_second()).map_err(|_| TsError)?);
        ts.to_string().parse()
    }

    /// The instant.
    pub fn timestamp(&self) -> Timestamp {
        self.0
    }
}

impl fmt::Display for Ts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.strftime("%Y-%m-%dT%H:%M:%SZ"))
    }
}

impl fmt::Debug for Ts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The text was not the one spelling v1 writes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("expected a whole-second UTC timestamp like 2026-10-10T18:30:00Z")]
pub struct TsError;

impl FromStr for Ts {
    type Err = TsError;

    fn from_str(s: &str) -> Result<Self, TsError> {
        let t: Timestamp = s.parse().map_err(|_| TsError)?;
        let ts = Self(t);
        // jiff accepts offsets, fractions, lowercase and more; only the exact spelling passes.
        if t.subsec_nanosecond() != 0 || ts.to_string() != s {
            return Err(TsError);
        }
        Ok(ts)
    }
}

impl Serialize for Ts {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Ts {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "2026-10-10T18:30:00Z";

    #[test]
    fn the_one_spelling_round_trips() {
        let ts: Ts = ONE.parse().unwrap();
        assert_eq!(ts.to_string(), ONE);
        assert_eq!(format!("{ts:?}"), ONE);
        assert_eq!(serde_json::to_string(&ts).unwrap(), format!("{ONE:?}"));
        assert_eq!(serde_json::from_str::<Ts>(&format!("{ONE:?}")).unwrap(), ts);
        assert_eq!(ts.timestamp().as_second(), 1791657000);
    }

    #[test]
    fn every_other_spelling_of_the_same_instant_is_refused() {
        let table = [
            "2026-10-10T20:30:00+02:00",
            "2026-10-10T18:30:00+00:00",
            "2026-10-10T18:30:00-00:00",
            "2026-10-10T18:30:00.000Z",
            "2026-10-10T18:30:00.5Z",
            "2026-10-10t18:30:00z",
            "2026-10-10 18:30:00Z",
            "2026-10-10T18:30Z",
            "2026-10-10T18:30:00Z[UTC]",
            "+002026-10-10T18:30:00Z",
            "20261010T183000Z",
            "2026-10-10T23:59:60Z",
            "2016-12-31T23:59:60Z",
            "2026-13-01T00:00:00Z",
            "2026-00-01T00:00:00Z",
            "2026-02-29T00:00:00Z",
            "2026-10-10T24:00:00Z",
            "2026-10-10T18:60:00Z",
            "-0001-10-10T18:30:00Z",
            "10000-01-01T00:00:00Z",
            "",
        ];
        for input in table {
            assert_eq!(input.parse::<Ts>(), Err(TsError), "{input:?} was accepted");
        }
    }

    #[test]
    fn from_timestamp_truncates_to_the_second_and_writes_only_what_reads_back() {
        let t: Timestamp = "2026-10-10T18:30:00.999999999Z".parse().unwrap();
        assert_eq!(Ts::from_timestamp(t).unwrap().to_string(), ONE);
        assert_eq!(Ts::from_timestamp(Timestamp::MIN), Err(TsError));
        let year_zero: Timestamp = "0000-01-01T00:00:00Z".parse().unwrap();
        assert_eq!(
            Ts::from_timestamp(year_zero).unwrap().to_string(),
            "0000-01-01T00:00:00Z"
        );
        assert!(Ts::from_timestamp(Timestamp::MAX).is_ok());
    }
}
