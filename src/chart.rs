use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::Snapshot;
use crate::cli::CommitInterval;
use crate::language::Language;
use crate::language_history::LanguageSnapshot;

const FRACTIONAL_BLOCKS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
const LANGUAGE_SYMBOLS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const LANGUAGE_COLORS: [u8; 36] = [
    39, 208, 70, 135, 220, 45, 203, 33, 112, 214, 171, 81, 196, 118, 207, 44, 178, 69, 141, 215,
    77, 204, 38, 172, 62, 210, 36, 180, 75, 168, 48, 202, 99, 114, 217, 51,
];

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
    let label_width = line_chart_label_width(history, date, sequence_width);
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

pub fn line_chart_reserved_width(history: &[Snapshot], date: bool) -> usize {
    let sequence_width = history.len().max(1).to_string().len();
    let label_width = line_chart_label_width(history, date, sequence_width);
    let value_width = history
        .iter()
        .map(|snapshot| snapshot.lines.to_string().len())
        .max()
        .unwrap_or(1);
    label_width + 3 + value_width
}

fn line_chart_label_width(history: &[Snapshot], date: bool, sequence_width: usize) -> usize {
    history
        .iter()
        .map(|snapshot| snapshot.label(date, sequence_width).len())
        .max()
        .unwrap_or(15)
}

pub fn render_commit_chart(
    counts: &[(String, u64)],
    interval: CommitInterval,
    width: usize,
) -> String {
    let counts = fill_empty_intervals(counts, interval);
    let maximum = counts.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let label_width = commit_chart_label_width(&counts);
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

pub fn commit_chart_reserved_width(counts: &[(String, u64)]) -> usize {
    let label_width = commit_chart_label_width(counts);
    let value_width = counts
        .iter()
        .map(|(_, count)| count.to_string().len())
        .max()
        .unwrap_or(1);
    label_width + 3 + value_width
}

fn commit_chart_label_width(counts: &[(String, u64)]) -> usize {
    counts
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(0)
}

pub fn render_language_chart(
    history: &[LanguageSnapshot],
    width: usize,
    date: bool,
    colors: bool,
) -> String {
    let mut languages: Vec<Language> = history
        .iter()
        .flat_map(|snapshot| snapshot.lines.keys().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    languages.sort_by_key(|language| (*language == Language::Other, language.name()));

    let sequence_width = history.len().max(1).to_string().len();
    let label_width = language_chart_label_width(history, date, sequence_width);
    let mut chart = format!(
        "{:>label_width$}  0%{:>marker_width$}\n",
        "",
        "100%",
        marker_width = width.saturating_sub(2)
    );
    for snapshot in history {
        let label = snapshot.label(date, sequence_width);
        let widths = language_widths(snapshot, &languages, width);
        if widths.iter().all(|segment| *segment == 0) {
            let _ = writeln!(chart, "{label:>label_width$}  0 lines");
            continue;
        }
        let bar = render_language_bar(&widths, colors);
        let _ = writeln!(chart, "{label:>label_width$}  {bar}");
    }

    if !languages.is_empty() {
        chart.push('\n');
        chart.push_str("Legend: ");
        for (index, language) in languages.iter().enumerate() {
            if index != 0 {
                chart.push_str(", ");
            }
            if colors {
                let _ = write!(
                    chart,
                    "\x1b[38;5;{}m█\x1b[0m {}",
                    LANGUAGE_COLORS[index % LANGUAGE_COLORS.len()],
                    language.name()
                );
            } else {
                let _ = write!(
                    chart,
                    "{} {}",
                    char::from(LANGUAGE_SYMBOLS[index]),
                    language.name()
                );
            }
        }
        chart.push('\n');
    }
    chart
}

pub fn language_chart_reserved_width(history: &[LanguageSnapshot], date: bool) -> usize {
    let sequence_width = history.len().max(1).to_string().len();
    language_chart_label_width(history, date, sequence_width) + 2
}

fn language_chart_label_width(
    history: &[LanguageSnapshot],
    date: bool,
    sequence_width: usize,
) -> usize {
    history
        .iter()
        .map(|snapshot| snapshot.label(date, sequence_width).len())
        .max()
        .unwrap_or(15)
}

fn language_widths(
    snapshot: &LanguageSnapshot,
    languages: &[Language],
    width: usize,
) -> Vec<usize> {
    let total: u128 = snapshot
        .lines
        .values()
        .map(|value| u128::from(*value))
        .sum();
    if total == 0 || width == 0 {
        return vec![0; languages.len()];
    }

    let mut widths = Vec::with_capacity(languages.len());
    let mut remainders = Vec::with_capacity(languages.len());
    for language in languages {
        let scaled = u128::from(snapshot.lines.get(language).copied().unwrap_or(0)) * width as u128;
        widths.push((scaled / total) as usize);
        remainders.push(scaled % total);
    }
    let allocated: usize = widths.iter().sum();
    let mut order: Vec<usize> = (0..languages.len()).collect();
    order.sort_by_key(|index| (std::cmp::Reverse(remainders[*index]), *index));
    for index in order.into_iter().take(width - allocated) {
        widths[index] += 1;
    }
    widths
}

fn render_language_bar(widths: &[usize], colors: bool) -> String {
    let mut bar = String::new();
    for (index, width) in widths.iter().enumerate() {
        if *width == 0 {
            continue;
        }
        if colors {
            let _ = write!(
                bar,
                "\x1b[38;5;{}m{}\x1b[0m",
                LANGUAGE_COLORS[index % LANGUAGE_COLORS.len()],
                "█".repeat(*width)
            );
        } else {
            bar.extend(std::iter::repeat_n(
                char::from(LANGUAGE_SYMBOLS[index]),
                *width,
            ));
        }
    }
    bar
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
