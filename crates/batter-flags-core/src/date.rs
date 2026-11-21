//! The UTC calendar date that expiry is declared and compared against.
//!
//! This crate reads no clock. A root passes the date it read, and tests pass a
//! fixed one, so expiry never depends on where or when a check runs.

use std::{fmt, str::FromStr, time::Duration};

use thiserror::Error;

/// A proleptic Gregorian calendar date, year 1 to 9999, with no time or zone.
///
/// Displays and parses as `YYYY-MM-DD`. With the `serde` feature it is a
/// string in that form.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CivilDate {
    year: u16,
    month: u8,
    day: u8,
}

impl CivilDate {
    /// Validates and constructs a date.
    ///
    /// # Errors
    ///
    /// Returns an error when the year is outside 1..=9999, the month outside
    /// 1..=12 or the day outside the month's length in that year.
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self, CivilDateError> {
        if !(1..=9999).contains(&year) {
            return Err(CivilDateError::Year { year });
        }
        if !(1..=12).contains(&month) {
            return Err(CivilDateError::Month { month });
        }
        let last = days_in_month(year, month);
        if day == 0 || day > last {
            return Err(CivilDateError::Day { year, month, day });
        }
        Ok(Self { year, month, day })
    }

    /// The year, 1..=9999.
    pub fn year(self) -> u16 {
        self.year
    }

    /// The month, 1..=12.
    pub fn month(self) -> u8 {
        self.month
    }

    /// The day of the month, 1..=31.
    pub fn day(self) -> u8 {
        self.day
    }

    /// The UTC date of an instant given as its distance from the Unix epoch.
    ///
    /// A pure conversion: this crate reads no clock, so a root that wants
    /// today's date reads the system clock itself and passes the result.
    ///
    /// ```
    /// use std::time::{Duration, SystemTime, UNIX_EPOCH};
    ///
    /// use batter_flags_core::CivilDate;
    ///
    /// let date = CivilDate::from_unix_time(Duration::from_secs(1_793_750_400)).unwrap();
    /// assert_eq!(date.to_string(), "2026-11-04");
    ///
    /// // At a root:
    /// let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    /// let _today = CivilDate::from_unix_time(since_epoch).unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CivilDateError::OutOfRange`] for an instant after 9999-12-31.
    pub fn from_unix_time(since_epoch: Duration) -> Result<Self, CivilDateError> {
        const SECONDS_PER_DAY: u64 = 86_400;
        // Days from 0000-03-01 to 1970-01-01 in the proleptic Gregorian calendar.
        const EPOCH_SHIFT: u64 = 719_468;
        const DAYS_PER_ERA: u64 = 146_097;
        let days = since_epoch.as_secs() / SECONDS_PER_DAY;
        let shifted = days
            .checked_add(EPOCH_SHIFT)
            .ok_or(CivilDateError::OutOfRange)?;
        let era = shifted / DAYS_PER_ERA;
        let day_of_era = shifted % DAYS_PER_ERA;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };
        let year = era * 400 + year_of_era + u64::from(month <= 2);
        let year = u16::try_from(year).map_err(|_| CivilDateError::OutOfRange)?;
        if year > 9999 {
            return Err(CivilDateError::OutOfRange);
        }
        let month = u8::try_from(month).map_err(|_| CivilDateError::OutOfRange)?;
        let day = u8::try_from(day).map_err(|_| CivilDateError::OutOfRange)?;
        Self::new(year, month, day)
    }
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
    }
}

impl fmt::Display for CivilDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

impl fmt::Debug for CivilDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CivilDate({self})")
    }
}

impl FromStr for CivilDate {
    type Err = CivilDateError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(CivilDateError::Format);
        }
        let number = |range: std::ops::Range<usize>| -> Result<u16, CivilDateError> {
            let digits = &text[range];
            if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(CivilDateError::Format);
            }
            digits.parse().map_err(|_| CivilDateError::Format)
        };
        let year = number(0..4)?;
        let month = u8::try_from(number(5..7)?).map_err(|_| CivilDateError::Format)?;
        let day = u8::try_from(number(8..10)?).map_err(|_| CivilDateError::Format)?;
        Self::new(year, month, day)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for CivilDate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for CivilDate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// An invalid calendar date.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CivilDateError {
    /// The year was outside 1..=9999.
    #[error("year {year} is outside 1..=9999")]
    Year {
        /// Supplied year.
        year: u16,
    },
    /// The month was outside 1..=12.
    #[error("month {month} is outside 1..=12")]
    Month {
        /// Supplied month.
        month: u8,
    },
    /// The day was outside the month.
    #[error("{year:04}-{month:02} has no day {day}")]
    Day {
        /// Supplied year.
        year: u16,
        /// Supplied month.
        month: u8,
        /// Supplied day.
        day: u8,
    },
    /// The text was not `YYYY-MM-DD`.
    #[error("a civil date is written YYYY-MM-DD")]
    Format,
    /// The instant lies after 9999-12-31.
    #[error("the instant lies after 9999-12-31")]
    OutOfRange,
}

/// The last valid UTC day of a release flag.
///
/// Expiry is manifest metadata for the people who clean release flags up. It
/// never changes a decision and never fails production startup:
/// [`Catalog::expired`](crate::Catalog::expired) lists the flags whose last
/// valid day lies before the date a caller passes, and declared mode reports
/// them once at WARN.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Expiry(CivilDate);

impl Expiry {
    /// The flag is valid through `last_valid_day` inclusive.
    pub fn on(last_valid_day: CivilDate) -> Self {
        Self(last_valid_day)
    }

    /// The last valid day.
    pub fn last_valid_day(self) -> CivilDate {
        self.0
    }

    /// Whether the flag has expired on `date`: its last valid day lies before it.
    pub fn is_expired_on(self, date: CivilDate) -> bool {
        self.0 < date
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_validate_month_lengths_and_leap_years() {
        assert!(CivilDate::new(2024, 2, 29).is_ok());
        assert_eq!(
            CivilDate::new(2023, 2, 29),
            Err(CivilDateError::Day {
                year: 2023,
                month: 2,
                day: 29
            })
        );
        assert!(CivilDate::new(2000, 2, 29).is_ok());
        assert!(CivilDate::new(1900, 2, 29).is_err());
        assert!(CivilDate::new(2026, 4, 31).is_err());
        assert_eq!(
            CivilDate::new(0, 1, 1),
            Err(CivilDateError::Year { year: 0 })
        );
        assert_eq!(
            CivilDate::new(2026, 13, 1),
            Err(CivilDateError::Month { month: 13 })
        );
    }

    #[test]
    fn dates_order_and_round_trip_as_text() {
        let earlier = CivilDate::new(2026, 11, 4).unwrap();
        let later = CivilDate::new(2027, 1, 1).unwrap();
        assert!(earlier < later);
        assert_eq!(earlier.to_string(), "2026-11-04");
        assert_eq!("2026-11-04".parse::<CivilDate>(), Ok(earlier));
        assert_eq!(
            "2026-11-4".parse::<CivilDate>(),
            Err(CivilDateError::Format)
        );
        assert_eq!(
            "2026/11/04".parse::<CivilDate>(),
            Err(CivilDateError::Format)
        );
        assert_eq!(format!("{earlier:?}"), "CivilDate(2026-11-04)");
    }

    #[test]
    fn unix_time_converts_to_the_utc_date() {
        let date = |seconds| CivilDate::from_unix_time(Duration::from_secs(seconds));
        assert_eq!(date(0), CivilDate::new(1970, 1, 1));
        assert_eq!(date(86_399), CivilDate::new(1970, 1, 1));
        assert_eq!(date(86_400), CivilDate::new(1970, 1, 2));
        assert_eq!(date(951_782_400), CivilDate::new(2000, 2, 29));
        assert_eq!(date(1_709_164_800), CivilDate::new(2024, 2, 29));
        assert_eq!(date(1_793_750_400), CivilDate::new(2026, 11, 4));
        assert_eq!(date(1_793_836_799), CivilDate::new(2026, 11, 4));
        assert_eq!(date(253_402_214_400), CivilDate::new(9999, 12, 31));
        assert_eq!(date(253_402_300_800), Err(CivilDateError::OutOfRange));
        assert_eq!(date(u64::MAX), Err(CivilDateError::OutOfRange));
    }

    #[test]
    fn expiry_is_inclusive_of_its_last_day() {
        let last = CivilDate::new(2026, 12, 31).unwrap();
        let expiry = Expiry::on(last);
        assert!(!expiry.is_expired_on(last));
        assert!(!expiry.is_expired_on(CivilDate::new(2026, 6, 1).unwrap()));
        assert!(expiry.is_expired_on(CivilDate::new(2027, 1, 1).unwrap()));
    }
}
