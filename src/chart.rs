use std::fmt::Write as _;

use crate::Snapshot;
use crate::cli::CommitInterval;

const FRACTIONAL_BLOCKS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

pub fn render_chart(
    history: &[Snapshot],
    width: usize,
    date: bool,
    non_blank: bool,
    colors: bool,
) -> String {
    let maximum = history
        .iter()
        .map(|snapshot| snapshot.lines)
        .max()
        .unwrap_or(0);
    let sequence_width = history.len().max(1).to_string().len();
    let label_width = history
        .iter()
        .map(|snapshot| snapshot.label(date, sequence_width).len())
        .max()
        .unwrap_or(15);
    let mut chart = String::from("        0 LoC\n");
    for snapshot in history {
        let label = snapshot.label(date, sequence_width);
        let bar = if non_blank {
            render_layered_bar(
                snapshot.non_blank_lines,
                snapshot.lines,
                maximum,
                width,
                colors,
            )
        } else {
            render_bar(snapshot.lines, maximum, width)
        };
        let _ = writeln!(
            chart,
            "{label:>label_width$}  {bar}{}{}",
            if bar.is_empty() { "" } else { " " },
            snapshot.lines
        );
    }
    chart
}

pub fn render_commit_chart(
    counts: &[(String, u64)],
    interval: CommitInterval,
    width: usize,
) -> String {
    let counts = fill_empty_intervals(counts, interval);
    let maximum = counts.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let label_width = counts
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(0);
    let mut chart = String::from("    0 commits\n");
    for (label, count) in counts {
        let bar = render_bar(count, maximum, width);
        let _ = writeln!(
            chart,
            "{label:>label_width$}  {bar}{}{}",
            if bar.is_empty() { "" } else { " " },
            count
        );
    }
    chart
}

fn render_bar(value: u64, maximum: u64, width: usize) -> String {
    if value == 0 || maximum == 0 || width == 0 {
        return String::new();
    }
    let eighths = scaled_eighths(value, maximum, width);
    let mut bar = "█".repeat((eighths / 8) as usize);
    let fraction = (eighths % 8) as usize;
    if fraction != 0 {
        bar.push(FRACTIONAL_BLOCKS[fraction]);
    }
    bar
}

fn scaled_eighths(value: u64, maximum: u64, width: usize) -> u128 {
    if maximum == 0 || width == 0 {
        return 0;
    }
    (u128::from(value) * width as u128 * 8) / u128::from(maximum)
}

pub(crate) fn render_layered_bar(
    non_blank: u64,
    all: u64,
    maximum: u64,
    width: usize,
    colors: bool,
) -> String {
    let all_bar = render_bar(all, maximum, width);
    if !colors || all_bar.is_empty() {
        return all_bar;
    }
    let overlay_width = (((scaled_eighths(non_blank, maximum, width) + 4) / 8) as usize)
        .min(all_bar.chars().count());
    let grey_overlay: String = all_bar.chars().take(overlay_width).collect();
    let white_remainder: String = all_bar.chars().skip(overlay_width).collect();
    format!(
        "{}{}{}{}\x1b[0m",
        if grey_overlay.is_empty() {
            ""
        } else {
            "\x1b[90m"
        },
        grey_overlay,
        if white_remainder.is_empty() {
            ""
        } else {
            "\x1b[97m"
        },
        white_remainder
    )
}

fn fill_empty_intervals(counts: &[(String, u64)], interval: CommitInterval) -> Vec<(String, u64)> {
    let Some((first, _)) = counts.first() else {
        return Vec::new();
    };
    let mut current = first.clone();
    let last = &counts.last().expect("first exists").0;
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
        current = next_interval(&current, interval).expect("Git emitted a valid interval");
    }
    result
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

fn iso_weeks_in_year(year: i32) -> u8 {
    let jan_first = weekday(year, 1, 1);
    if jan_first == 5 || (jan_first == 4 && is_leap_year(year)) {
        53
    } else {
        52
    }
}

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
        + 5 * (year / 100)
        + 5)
        % 7
}
