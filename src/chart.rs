//! Rendering. Every function here turns already-collected data into text and
//! never queries the repository or reshapes the series it is given.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::language::Language;
use crate::language_history::LanguageSnapshot;
use crate::line_history::Snapshot;

const FRACTIONAL_BLOCKS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
const LANGUAGE_SYMBOLS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const LANGUAGE_COLORS: [u8; 36] = [
    39, 208, 70, 135, 220, 45, 203, 33, 112, 214, 171, 81, 196, 118, 207, 44, 178, 69, 141, 215,
    77, 204, 38, 172, 62, 210, 36, 180, 75, 168, 48, 202, 99, 114, 217, 51,
];

/// Both palettes are indexed modulo their length, so a language set larger than
/// either one repeats entries instead of panicking. Keeping symbols at least as
/// long as colors means plain-text output stays distinct wherever color does.
const _: () = assert!(LANGUAGE_SYMBOLS.len() >= LANGUAGE_COLORS.len());

/// The default bar width used when the terminal size is irrelevant or unknown.
const DEFAULT_BARS: usize = 50;

/// How much horizontal space the chart may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChartWidth {
    /// Draw bars exactly this many columns wide.
    Bars(usize),
    /// Give bars whatever the terminal leaves after labels and values.
    Terminal(usize),
}

impl ChartWidth {
    pub(crate) const DEFAULT: Self = Self::Bars(DEFAULT_BARS);
}

/// Presentation flags shared by the snapshot-based charts. `non_blank` is
/// meaningful only for the line chart; the CLI rejects it in other modes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChartStyle {
    pub(crate) date: bool,
    pub(crate) non_blank: bool,
    pub(crate) colors: bool,
}

/// Column budget for one chart: how wide the right-aligned label column is and
/// how many columns are left for bars.
struct Layout {
    label: usize,
    bar: usize,
}

impl Layout {
    /// `value` is the width of the trailing numeric column, or `None` for
    /// charts that print no value after the bar. A row is laid out as
    /// `label`, two spaces, the bar, then a space and the value.
    fn new(label: usize, value: Option<usize>, width: ChartWidth) -> Self {
        let reserved = label + 2 + value.map_or(0, |value| value + 1);
        let bar = match width {
            ChartWidth::Bars(bars) => bars,
            ChartWidth::Terminal(columns) => columns.saturating_sub(reserved),
        };
        Self { label, bar }
    }
}

fn sequence_width(rows: usize) -> usize {
    rows.max(1).to_string().len()
}

fn digits(value: u64) -> usize {
    value.to_string().len()
}

pub(crate) fn render_chart(history: &[Snapshot], width: ChartWidth, style: ChartStyle) -> String {
    let sequence_width = sequence_width(history.len());
    let layout = Layout::new(
        label_width(
            history
                .iter()
                .map(|s| s.at.label(style.date, sequence_width)),
        ),
        Some(
            history
                .iter()
                .map(|snapshot| digits(snapshot.lines.all))
                .max()
                .unwrap_or(1),
        ),
        width,
    );
    let maximum = history
        .iter()
        .map(|snapshot| snapshot.lines.all)
        .max()
        .unwrap_or(0);

    let mut chart = String::from("        0 LoC\n");
    for snapshot in history {
        let label = snapshot.at.label(style.date, sequence_width);
        let bar = if style.non_blank {
            render_layered_bar(
                snapshot.lines.non_blank,
                snapshot.lines.all,
                maximum,
                layout.bar,
                style.colors,
            )
        } else {
            render_bar(snapshot.lines.all, maximum, layout.bar)
        };
        write_row(&mut chart, &label, layout.label, &bar, snapshot.lines.all);
    }
    chart
}

pub(crate) fn render_commit_chart(counts: &[(String, u64)], width: ChartWidth) -> String {
    let layout = Layout::new(
        counts
            .iter()
            .map(|(label, _)| label.len())
            .max()
            .unwrap_or(0),
        Some(
            counts
                .iter()
                .map(|(_, count)| digits(*count))
                .max()
                .unwrap_or(1),
        ),
        width,
    );
    let maximum = counts.iter().map(|(_, count)| *count).max().unwrap_or(0);

    let mut chart = String::from("    0 commits\n");
    for (label, count) in counts {
        let bar = render_bar(*count, maximum, layout.bar);
        write_row(&mut chart, label, layout.label, &bar, *count);
    }
    chart
}

/// Writes `label  bar value`, collapsing the separating space when the bar is
/// empty so a zero row reads `label  0`.
fn write_row(chart: &mut String, label: &str, label_width: usize, bar: &str, value: u64) {
    let separator = if bar.is_empty() { "" } else { " " };
    let _ = writeln!(chart, "{label:>label_width$}  {bar}{separator}{value}");
}

pub(crate) fn render_language_chart(
    history: &[LanguageSnapshot],
    width: ChartWidth,
    style: ChartStyle,
) -> String {
    let sequence_width = sequence_width(history.len());
    let layout = Layout::new(
        label_width(
            history
                .iter()
                .map(|s| s.at.label(style.date, sequence_width)),
        ),
        None,
        width,
    );

    let (languages, rows) = visible_language_widths(history, layout.bar);

    let label_width = layout.label;
    let mut chart = format!(
        "{:>label_width$}  0%{:>marker_width$}\n",
        "",
        "100%",
        marker_width = layout.bar.saturating_sub(2)
    );
    for (snapshot, widths) in history.iter().zip(rows) {
        let label = snapshot.at.label(style.date, sequence_width);
        if widths.iter().all(|segment| *segment == 0) {
            let _ = writeln!(chart, "{label:>label_width$}  0 lines");
            continue;
        }
        let bar = render_language_bar(&widths, style.colors);
        let _ = writeln!(chart, "{label:>label_width$}  {bar}");
    }

    if !languages.is_empty() {
        chart.push('\n');
        chart.push_str("Legend: ");
        for (index, language) in languages.iter().enumerate() {
            if index != 0 {
                chart.push_str(", ");
            }
            if style.colors {
                let _ = write!(chart, "{} {}", color_block(index), language.name());
            } else {
                let _ = write!(chart, "{} {}", symbol(index), language.name());
            }
        }
        chart.push('\n');
    }
    chart
}

/// Per-row segment widths restricted to languages that get at least one column
/// in some row, so the legend lists exactly what the bars draw. Widths are
/// allocated against every language first, so dropping invisible ones does not
/// change the rounding of the rest.
fn visible_language_widths(
    history: &[LanguageSnapshot],
    width: usize,
) -> (Vec<Language>, Vec<Vec<usize>>) {
    let languages = legend_order(history);
    let rows: Vec<Vec<usize>> = history
        .iter()
        .map(|snapshot| language_widths(snapshot, &languages, width))
        .collect();
    let visible: Vec<usize> = (0..languages.len())
        .filter(|index| rows.iter().any(|row| row[*index] != 0))
        .collect();
    (
        visible.iter().map(|index| languages[*index]).collect(),
        rows.iter()
            .map(|row| visible.iter().map(|index| row[*index]).collect())
            .collect(),
    )
}

/// Every language the history touches, alphabetically, with `Other` last so the
/// catch-all does not sit between real languages.
fn legend_order(history: &[LanguageSnapshot]) -> Vec<Language> {
    let mut languages: Vec<Language> = history
        .iter()
        .flat_map(|snapshot| snapshot.lines.keys().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    languages.sort_by_key(|language| (*language == Language::Other, language.name()));
    languages
}

/// Falls back to a plausible width so an empty history still leaves room for a
/// label column.
fn label_width(labels: impl Iterator<Item = String>) -> usize {
    labels.map(|label| label.len()).max().unwrap_or(15)
}

fn symbol(index: usize) -> char {
    char::from(LANGUAGE_SYMBOLS[index % LANGUAGE_SYMBOLS.len()])
}

fn color(index: usize) -> u8 {
    LANGUAGE_COLORS[index % LANGUAGE_COLORS.len()]
}

fn color_block(index: usize) -> String {
    format!("\x1b[38;5;{}m█\x1b[0m", color(index))
}

/// Distributes `width` columns across `languages` in proportion to their line
/// counts, handing the leftover columns to the largest remainders so the bar is
/// always exactly `width` wide.
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
                color(index),
                "█".repeat(*width)
            );
        } else {
            bar.extend(std::iter::repeat_n(symbol(index), *width));
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

/// Draws the total-lines bar, then recolors its leading portion grey to show how
/// much of it is non-blank. Splitting an already-drawn bar keeps the boundary
/// continuous even when it lands inside a fractional block.
fn render_layered_bar(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::CommitRef;
    use crate::line_count::LineCount;
    use std::collections::BTreeMap;

    fn snapshot(sequence: usize, id: &str, all: u64, non_blank: u64) -> Snapshot {
        Snapshot {
            at: CommitRef::new(sequence, id, "2026-07-15 10:00:00"),
            lines: LineCount { all, non_blank },
        }
    }

    fn language_snapshot(
        sequence: usize,
        id: &str,
        lines: BTreeMap<Language, u64>,
    ) -> LanguageSnapshot {
        LanguageSnapshot {
            at: CommitRef::new(sequence, id, "2026-07-15 10:00:00"),
            lines,
        }
    }

    #[test]
    fn layout_splits_terminal_columns_between_labels_values_and_bars() {
        // 12 label + 2 gutter + 1 space + 2 value = 17 reserved of 120.
        assert_eq!(Layout::new(12, Some(2), ChartWidth::Terminal(120)).bar, 103);
        // Without a value column only the two-space gutter is reserved.
        assert_eq!(Layout::new(10, None, ChartWidth::Terminal(120)).bar, 108);
        // A fixed width ignores the terminal entirely.
        assert_eq!(Layout::new(12, Some(2), ChartWidth::Bars(50)).bar, 50);
        // A terminal narrower than the reserved columns yields no bar.
        assert_eq!(Layout::new(12, Some(2), ChartWidth::Terminal(10)).bar, 0);
    }

    #[test]
    fn renders_scaled_bars_in_history_order() {
        let history = vec![
            snapshot(1, "0123456789abcdef", 5, 4),
            snapshot(2, "fedcba9876543210", 10, 8),
            snapshot(3, "aabbccddeeff0011", 0, 0),
        ];

        assert_eq!(
            render_chart(&history, ChartWidth::Bars(10), ChartStyle::default()),
            "        0 LoC\n\
             1:01234567  █████ 5\n\
             2:fedcba98  ██████████ 10\n\
             3:aabbccdd  0\n"
        );

        assert_eq!(
            render_chart(
                &history,
                ChartWidth::Bars(10),
                ChartStyle {
                    date: true,
                    ..ChartStyle::default()
                },
            ),
            "        0 LoC\n\
             2026-07-15 10:00:00:1:01234567  █████ 5\n\
             2026-07-15 10:00:00:2:fedcba98  ██████████ 10\n\
             2026-07-15 10:00:00:3:aabbccdd  0\n"
        );
    }

    #[test]
    fn leaves_the_default_chart_uncolored() {
        let history = vec![snapshot(1, "0123456789abcdef", 10, 5)];

        assert_eq!(
            render_chart(
                &history,
                ChartWidth::Bars(10),
                ChartStyle {
                    colors: true,
                    ..ChartStyle::default()
                },
            ),
            "        0 LoC\n1:01234567  ██████████ 10\n"
        );
    }

    #[test]
    fn layers_grey_non_blank_lines_over_the_white_total() {
        assert_eq!(
            render_layered_bar(5, 10, 10, 10, true),
            "\x1b[90m█████\x1b[97m█████\x1b[0m"
        );
        assert_eq!(render_layered_bar(5, 10, 10, 10, false), "██████████");
    }

    #[test]
    fn keeps_fractional_bars_continuous_at_the_color_boundary() {
        assert_eq!(
            render_layered_bar(73, 89, 100, 10, true),
            "\x1b[90m███████\x1b[97m█▉\x1b[0m"
        );
    }

    #[test]
    fn renders_daily_commit_counts() {
        let commits = vec![
            ("2026-07-14".to_owned(), 2),
            ("2026-07-15".to_owned(), 0),
            ("2026-07-16".to_owned(), 1),
        ];

        assert_eq!(
            render_commit_chart(&commits, ChartWidth::Bars(10)),
            "    0 commits\n\
             2026-07-14  ██████████ 2\n\
             2026-07-15  0\n\
             2026-07-16  █████ 1\n"
        );
    }

    #[test]
    fn renders_language_fractions_with_stable_plain_text_symbols() {
        let history = vec![
            language_snapshot(
                1,
                "0123456789abcdef",
                BTreeMap::from([(Language::Rust, 3), (Language::Markdown, 1)]),
            ),
            language_snapshot(
                2,
                "fedcba9876543210",
                BTreeMap::from([(Language::Rust, 2), (Language::Python, 2)]),
            ),
        ];

        let chart = render_language_chart(&history, ChartWidth::Bars(10), ChartStyle::default());

        assert!(chart.contains("1:01234567  AAACCCCCCC\n"));
        assert!(chart.contains("2:fedcba98  BBBBBCCCCC\n"));
        assert!(chart.ends_with("Legend: A Markdown, B Python, C Rust\n"));
    }

    #[test]
    fn legend_lists_only_languages_that_get_a_column_in_some_revision() {
        let history = vec![
            language_snapshot(
                1,
                "0123456789abcdef",
                BTreeMap::from([(Language::Rust, 100), (Language::Python, 1)]),
            ),
            language_snapshot(
                2,
                "fedcba9876543210",
                BTreeMap::from([
                    (Language::Rust, 90),
                    (Language::Markdown, 10),
                    (Language::Python, 1),
                ]),
            ),
        ];

        let chart = render_language_chart(&history, ChartWidth::Bars(10), ChartStyle::default());

        // Python never earns a column; Markdown earns one only in revision 2.
        assert!(chart.contains("1:01234567  BBBBBBBBBB\n"));
        assert!(chart.contains("2:fedcba98  ABBBBBBBBB\n"));
        assert!(chart.ends_with("Legend: A Markdown, B Rust\n"));
    }

    #[test]
    fn language_fraction_rounding_always_fills_the_bar() {
        let history = vec![language_snapshot(
            1,
            "0123456789abcdef",
            BTreeMap::from([
                (Language::Rust, 1),
                (Language::Python, 1),
                (Language::Markdown, 1),
            ]),
        )];

        let chart = render_language_chart(&history, ChartWidth::Bars(10), ChartStyle::default());
        let bar = chart
            .lines()
            .find(|line| line.contains("1:01234567"))
            .and_then(|line| line.rsplit_once("  ").map(|(_, bar)| bar))
            .unwrap();

        assert_eq!(bar.chars().count(), 10);
    }

    #[test]
    fn language_palettes_wrap_instead_of_panicking() {
        assert_eq!(color(LANGUAGE_COLORS.len() + 3), color(3));
        assert_eq!(symbol(LANGUAGE_SYMBOLS.len() + 3), symbol(3));
        // Symbols outlast colors, so plain-text output stays distinct longer.
        assert_ne!(symbol(LANGUAGE_COLORS.len() + 3), symbol(3));
    }

    #[test]
    fn empty_histories_render_only_a_header() {
        assert_eq!(
            render_chart(&[], ChartWidth::Bars(10), ChartStyle::default()),
            "        0 LoC\n"
        );
        assert_eq!(
            render_commit_chart(&[], ChartWidth::Bars(10)),
            "    0 commits\n"
        );
    }
}
