//! Calendar arithmetic over the interval labels Git produces.
//!
//! Commit-frequency data is keyed by a formatted date rather than a timestamp,
//! so filling the gaps between active intervals means stepping those labels
//! forward. Keeping that here leaves `chart` responsible only for drawing.

/// The calendar grouping selected by `--commits`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitInterval {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl CommitInterval {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "yearly" => Some(Self::Yearly),
            _ => None,
        }
    }

    /// The `git log --date=format:` specifier that labels one interval.
    pub(crate) fn date_format(self) -> &'static str {
        match self {
            Self::Daily => "%Y-%m-%d",
            Self::Weekly => "%G-W%V",
            Self::Monthly => "%Y-%m",
            Self::Yearly => "%Y",
        }
    }
}

/// Expands `counts` so that every interval between the first and the last is
/// present, giving untouched intervals a zero count.
pub(crate) fn fill_empty_intervals(
    counts: &[(String, u64)],
    interval: CommitInterval,
) -> Result<Vec<(String, u64)>, String> {
    let Some((first, _)) = counts.first() else {
        return Ok(Vec::new());
    };
    let last = &counts.last().expect("first exists").0;
    let mut current = first.clone();
    let mut index = 0;
    let mut result = Vec::new();
    loop {
        let count = if counts
            .get(index)
            .is_some_and(|(label, _)| label == &current)
        {
            let value = counts[index].1;
            index += 1;
            value
        } else {
            0
        };
        result.push((current.clone(), count));
        if &current == last {
            break;
        }
        current = next_interval(&current, interval)
            .ok_or_else(|| format!("unrecognized {interval:?} interval label: {current}"))?;
    }
    Ok(result)
}

fn next_interval(value: &str, interval: CommitInterval) -> Option<String> {
    match interval {
        CommitInterval::Daily => {
            let (year, month, day) = parse_date(value)?;
            let (year, month, day) = if day < days_in_month(year, month) {
                (year, month, day + 1)
            } else if month < 12 {
                (year, month + 1, 1)
            } else {
                (year + 1, 1, 1)
            };
            Some(format!("{year:04}-{month:02}-{day:02}"))
        }
        CommitInterval::Weekly => {
            let (year, week) = value.split_once("-W")?;
            let year = year.parse::<i32>().ok()?;
            let week = week.parse::<u8>().ok()?;
            if week == 0 || week > iso_weeks_in_year(year) {
                return None;
            }
            if week < iso_weeks_in_year(year) {
                Some(format!("{year:04}-W{:02}", week + 1))
            } else {
                Some(format!("{:04}-W01", year + 1))
            }
        }
        CommitInterval::Monthly => {
            let (year, month) = value.split_once('-')?;
            let year = year.parse::<i32>().ok()?;
            let month = month.parse::<u8>().ok()?;
            if !(1..=12).contains(&month) {
                return None;
            }
            if month < 12 {
                Some(format!("{year:04}-{:02}", month + 1))
            } else {
                Some(format!("{:04}-01", year + 1))
            }
        }
        CommitInterval::Yearly => Some(format!("{:04}", value.parse::<i32>().ok()? + 1)),
    }
}

fn parse_date(value: &str) -> Option<(i32, u8, u8)> {
    let mut parts = value.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    (parts.next().is_none()
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day))
    .then_some((year, month, day))
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// An ISO-8601 year has 53 weeks when it starts on a Thursday, or on a
/// Wednesday in a leap year.
fn iso_weeks_in_year(year: i32) -> u8 {
    let jan_first = weekday(year, 1, 1);
    if jan_first == 5 || (jan_first == 4 && is_leap_year(year)) {
        53
    } else {
        52
    }
}

/// Zeller's congruence: 0 is Saturday, 1 Sunday, ... 5 Thursday, 6 Friday.
fn weekday(year: i32, month: u8, day: u8) -> i32 {
    let (year, month) = if month < 3 {
        (year - 1, month as i32 + 12)
    } else {
        (year, month as i32)
    };
    (day as i32
        + (13 * (month + 1)) / 5
        + year % 100
        + (year % 100) / 4
        + year / 100 / 4
        + 5 * (year / 100))
        % 7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interval_names() {
        assert_eq!(
            CommitInterval::parse("weekly"),
            Some(CommitInterval::Weekly)
        );
        assert_eq!(CommitInterval::parse("hourly"), None);
    }

    #[test]
    fn steps_days_across_month_and_year_boundaries() {
        let next = |value| next_interval(value, CommitInterval::Daily).unwrap();

        assert_eq!(next("2026-07-15"), "2026-07-16");
        assert_eq!(next("2026-07-31"), "2026-08-01");
        assert_eq!(next("2026-12-31"), "2027-01-01");
        assert_eq!(next("2024-02-28"), "2024-02-29");
        assert_eq!(next("2026-02-28"), "2026-03-01");
        assert_eq!(next("2000-02-28"), "2000-02-29");
        assert_eq!(next("1900-02-28"), "1900-03-01");
    }

    #[test]
    fn steps_months_and_years() {
        assert_eq!(
            next_interval("2026-11", CommitInterval::Monthly).unwrap(),
            "2026-12"
        );
        assert_eq!(
            next_interval("2026-12", CommitInterval::Monthly).unwrap(),
            "2027-01"
        );
        assert_eq!(
            next_interval("2026", CommitInterval::Yearly).unwrap(),
            "2027"
        );
    }

    #[test]
    fn steps_iso_weeks_including_long_years() {
        let next = |value| next_interval(value, CommitInterval::Weekly).unwrap();

        assert_eq!(next("2026-W01"), "2026-W02");
        // 2026 starts on a Thursday, so it has 53 ISO weeks.
        assert_eq!(iso_weeks_in_year(2026), 53);
        assert_eq!(next("2026-W52"), "2026-W53");
        assert_eq!(next("2026-W53"), "2027-W01");
        // 2020 is a leap year starting on a Wednesday: also 53 weeks.
        assert_eq!(iso_weeks_in_year(2020), 53);
        // 2025 is an ordinary 52-week year.
        assert_eq!(iso_weeks_in_year(2025), 52);
        assert_eq!(next("2025-W52"), "2026-W01");
    }

    #[test]
    fn rejects_labels_that_do_not_match_the_interval() {
        assert_eq!(next_interval("2026-13", CommitInterval::Monthly), None);
        assert_eq!(next_interval("2026-02-30", CommitInterval::Daily), None);
        assert_eq!(next_interval("2026-W00", CommitInterval::Weekly), None);
        assert_eq!(next_interval("nonsense", CommitInterval::Yearly), None);
    }

    #[test]
    fn fills_gaps_between_active_intervals() {
        assert_eq!(
            fill_empty_intervals(
                &[("2026-07-14".to_owned(), 2), ("2026-07-16".to_owned(), 1)],
                CommitInterval::Daily,
            )
            .unwrap(),
            vec![
                ("2026-07-14".to_owned(), 2),
                ("2026-07-15".to_owned(), 0),
                ("2026-07-16".to_owned(), 1),
            ]
        );
        assert_eq!(
            fill_empty_intervals(&[], CommitInterval::Daily).unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn reports_an_error_instead_of_panicking_on_a_bad_label() {
        let error = fill_empty_intervals(
            &[("nonsense".to_owned(), 1), ("2026-07-16".to_owned(), 1)],
            CommitInterval::Daily,
        )
        .unwrap_err();

        assert!(error.contains("nonsense"), "{error}");
    }
}
