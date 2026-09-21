//! `repo-lines` plots how a Git repository changes along its first-parent
//! history.
//!
//! Modules are layered: `git` is the only code that shells out, the history
//! collectors (`line_history`, `language_history`, `commit_frequency`) turn its
//! output into series, and `chart` renders those series as text. This file only
//! picks a mode and wires the two halves together.

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

use terminal_size::{Width, terminal_size};

mod calendar;
mod chart;
mod cli;
mod commit;
mod commit_frequency;
mod git;
mod language;
mod language_history;
mod line_count;
mod line_history;
#[cfg(test)]
mod test_repo;

use chart::{ChartStyle, ChartWidth, render_chart, render_commit_chart, render_language_chart};
use cli::{Invocation, Options, parse, usage};
use commit_frequency::collect_commit_counts;
use language_history::collect_language_history;
use line_history::collect_history;

fn main() -> ExitCode {
    let outcome = match parse(std::env::args().skip(1)) {
        Err(error) => Err(format!("{error}\n\n{}", usage())),
        Ok(Invocation::Help) => {
            print!("{}", usage());
            Ok(())
        }
        Ok(Invocation::Version) => {
            println!("repo-lines {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Ok(Invocation::Run(options)) => draw(&options),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("repo-lines: {error}");
            ExitCode::FAILURE
        }
    }
}

fn draw(options: &Options) -> Result<(), String> {
    let repo = Path::new(&options.path);
    let width = chart_width(options.full_width);
    let style = ChartStyle {
        date: options.date,
        non_blank: options.non_blank,
        colors: std::io::stdout().is_terminal(),
    };

    let chart = if let Some(interval) = options.commits {
        render_commit_chart(
            &collect_commit_counts(repo, &options.revision, interval)?,
            width,
        )
    } else if options.languages {
        render_language_chart(
            &collect_language_history(repo, &options.revision)?,
            width,
            style,
        )
    } else {
        render_chart(
            &collect_history(repo, &options.revision, options.non_blank)?,
            width,
            style,
        )
    };
    print!("{chart}");
    Ok(())
}

/// `--full-width` only takes effect when the terminal reports a size; otherwise
/// the chart keeps its default bar width.
fn chart_width(full_width: bool) -> ChartWidth {
    if !full_width {
        return ChartWidth::DEFAULT;
    }
    terminal_size()
        .map(|(Width(columns), _)| ChartWidth::Terminal(usize::from(columns)))
        .unwrap_or(ChartWidth::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_is_opt_in() {
        assert_eq!(chart_width(false), ChartWidth::DEFAULT);
    }
}
