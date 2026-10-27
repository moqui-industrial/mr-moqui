//! Relative dashboard times, evaluated once against the caller's clock and time zone.
use jiff::{Span, Timestamp};
use moqui_lib::error::Error;

pub fn parse(text: &str, now: i64) -> Result<i64, Error> {
    if let Ok(number) = text.parse::<i64>() {
        return Timestamp::from_millisecond(number)
            .map(|t| t.as_millisecond())
            .map_err(|e| Error::new(e.to_string()));
    }
    if !text.starts_with("now") {
        return text
            .parse::<Timestamp>()
            .map(|t| t.as_millisecond())
            .map_err(|e| Error::new(format!("invalid report time {text}: {e}")));
    }
    let zone = crate::l10n::time_zone();
    let mut time = Timestamp::from_millisecond(now)
        .map_err(|e| Error::new(e.to_string()))?
        .to_zoned(zone.clone());
    let mut rest = &text[3..];
    while let Some(sign) = rest.chars().next().filter(|c| *c == '+' || *c == '-') {
        rest = &rest[1..];
        let n = rest.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return Err(Error::new(format!("invalid relative report time {text}")));
        }
        let amount =
            rest[..n].parse::<i64>().map_err(|e| Error::new(e.to_string()))? * if sign == '-' { -1 } else { 1 };
        rest = &rest[n..];
        let unit = rest
            .chars()
            .next()
            .ok_or_else(|| Error::new("relative time needs a unit"))?;
        rest = &rest[unit.len_utf8()..];
        let span = match unit {
            's' => Span::new().try_seconds(amount),
            'm' => Span::new().try_minutes(amount),
            'h' => Span::new().try_hours(amount),
            'd' => Span::new().try_days(amount),
            'w' => Span::new().try_weeks(amount),
            'M' => Span::new().try_months(amount),
            'y' => Span::new().try_years(amount),
            _ => return Err(Error::new(format!("unknown report time unit {unit}"))),
        }
        .map_err(|e| Error::new(e.to_string()))?;
        time = time.checked_add(span).map_err(|e| Error::new(e.to_string()))?;
    }
    if let Some(round) = rest.strip_prefix('/') {
        let date = time.date();
        let date = match round {
            "d" => date,
            "w" => date
                .checked_sub(
                    Span::new().days(i64::from(
                        if crate::context::ExecutionContext::with_current(|ec| {
                            ec.factory().conf_number::<i64>("report", "week-start", 1)
                        })
                        .unwrap_or(1)
                            == 0
                        {
                            date.weekday().to_sunday_zero_offset()
                        } else {
                            date.weekday().to_monday_zero_offset()
                        },
                    )),
                )
                .map_err(|e| Error::new(e.to_string()))?,
            "M" => date.with().day(1).build().map_err(|e| Error::new(e.to_string()))?,
            "y" => date
                .with()
                .month(1)
                .day(1)
                .build()
                .map_err(|e| Error::new(e.to_string()))?,
            "fy" => {
                let (month, day) = crate::context::ExecutionContext::with_current(|ec| {
                    (
                        ec.factory().conf_number::<i8>("report", "fiscal-start-month", 1),
                        ec.factory().conf_number::<i8>("report", "fiscal-start-day", 1),
                    )
                })
                .unwrap_or((1, 1));
                let mut start =
                    jiff::civil::Date::new(date.year(), month, day).map_err(|e| Error::new(e.to_string()))?;
                if start > date {
                    start = start
                        .checked_sub(Span::new().years(1))
                        .map_err(|e| Error::new(e.to_string()))?;
                }
                start
            }
            "h" | "m" => {
                return time
                    .round(
                        jiff::ZonedRound::new()
                            .smallest(if round == "h" {
                                jiff::Unit::Hour
                            } else {
                                jiff::Unit::Minute
                            })
                            .mode(jiff::RoundMode::Floor),
                    )
                    .map(|t| t.timestamp().as_millisecond())
                    .map_err(|e| Error::new(e.to_string()))
            }
            _ => return Err(Error::new(format!("unknown report time rounding {round}"))),
        };
        time = date
            .at(0, 0, 0, 0)
            .to_zoned(zone)
            .map_err(|e| Error::new(e.to_string()))?;
    } else if !rest.is_empty() {
        return Err(Error::new(format!("invalid report time {text}")));
    }
    Ok(time.timestamp().as_millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_ranges_are_anchored() {
        let now = 1_700_000_000_000;
        assert_eq!(parse("now-6h", now).unwrap(), now - 21_600_000);
        assert_eq!(parse("now+1m-30s", now).unwrap(), now + 30_000);
        assert!(parse("now-whatever", now).is_err());
    }
    #[test]
    fn accepts_absolute_times() {
        assert_eq!(parse("2024-01-01T00:00:00Z", 0).unwrap(), 1_704_067_200_000);
    }
}
