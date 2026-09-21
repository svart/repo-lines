use crate::calendar::CommitInterval;

/// What the command line asked for. Help and version short-circuit the rest of
/// parsing, so they are outcomes rather than fields on `Options`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invocation {
    Help,
    Version,
    Run(Options),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) date: bool,
    pub(crate) non_blank: bool,
    pub(crate) languages: bool,
    pub(crate) full_width: bool,
    pub(crate) commits: Option<CommitInterval>,
    pub(crate) revision: String,
    pub(crate) path: String,
}

impl Options {
    /// The options implied by an empty command line.
    fn new() -> Self {
        Self {
            date: false,
            non_blank: false,
            languages: false,
            full_width: false,
            commits: None,
            revision: "HEAD".to_owned(),
            path: ".".to_owned(),
        }
    }
}

pub(crate) fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Invocation, String> {
    let arguments: Vec<String> = arguments.into_iter().collect();
    if arguments
        .iter()
        .any(|a| matches!(a.as_str(), "-h" | "--help"))
    {
        return Ok(Invocation::Help);
    }
    if arguments
        .iter()
        .any(|a| matches!(a.as_str(), "-V" | "--version"))
    {
        return Ok(Invocation::Version);
    }
    parse_options(arguments).map(Invocation::Run)
}

fn parse_options(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut arguments = arguments.into_iter();
    let mut options = Options::new();

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--date" => options.date = true,
            "--non-blank" => options.non_blank = true,
            "--languages" => options.languages = true,
            "--full-width" => options.full_width = true,
            "--commits" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "missing value for --commits".to_owned())?;
                options.commits = Some(CommitInterval::parse(&value).ok_or_else(|| {
                    format!("invalid value for --commits: {value} (expected daily, weekly, monthly, or yearly)")
                })?);
            }
            "--rev" => {
                options.revision = arguments
                    .next()
                    .ok_or_else(|| "missing value for --rev".to_owned())?
            }
            "--path" => {
                options.path = arguments
                    .next()
                    .ok_or_else(|| "missing value for --path".to_owned())?
            }
            _ => return Err(format!("unexpected argument: {argument}")),
        }
    }
    if options.languages && (options.commits.is_some() || options.non_blank) {
        return Err("--languages cannot be combined with --commits or --non-blank".to_owned());
    }
    if options.commits.is_some() && (options.date || options.non_blank) {
        return Err("--commits cannot be combined with --date or --non-blank".to_owned());
    }
    Ok(options)
}

pub(crate) fn usage() -> &'static str {
    "Usage: repo-lines [OPTIONS]\n\nPlot line count, language fractions, or commit frequency along a Git revision's first-parent history.\n\nOptions:\n  --rev <REVISION>                         Revision to inspect [default: HEAD]\n  --path <PATH>                            Repository path [default: .]\n  --date                                   Print commit date and time\n  --non-blank                              Overlay non-blank lines in grey\n  --languages                              Plot the fraction of lines by language\n  --commits <daily|weekly|monthly|yearly>  Plot commit frequency instead of lines\n  --full-width                             Use the full available terminal width\n  -h, --help                               Print help\n  -V, --version                            Print version\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arguments: &[&str]) -> Result<Options, String> {
        match parse(arguments.iter().copied().map(str::to_owned))? {
            Invocation::Run(options) => Ok(options),
            other => panic!("expected a run invocation, got {other:?}"),
        }
    }

    #[test]
    fn parses_default_options() {
        assert_eq!(run(&[]).unwrap(), Options::new());
    }

    #[test]
    fn parses_revision_path_and_date_options() {
        assert_eq!(
            run(&[
                "--path",
                "/tmp/project",
                "--date",
                "--non-blank",
                "--rev",
                "main"
            ])
            .unwrap(),
            Options {
                date: true,
                non_blank: true,
                revision: "main".to_owned(),
                path: "/tmp/project".to_owned(),
                ..Options::new()
            }
        );
    }

    #[test]
    fn recognizes_help_and_version_before_validating_anything_else() {
        let help = ["--help", "-h"];
        for flag in help {
            assert_eq!(
                parse([flag, "--languages", "--non-blank"].map(str::to_owned)).unwrap(),
                Invocation::Help
            );
        }
        for flag in ["--version", "-V"] {
            assert_eq!(
                parse([flag, "nonsense"].map(str::to_owned)).unwrap(),
                Invocation::Version
            );
        }
    }

    #[test]
    fn parses_commit_interval_and_rejects_incompatible_chart_options() {
        assert_eq!(
            run(&["--commits", "monthly"]).unwrap(),
            Options {
                commits: Some(CommitInterval::Monthly),
                ..Options::new()
            }
        );
        assert_eq!(
            run(&["--commits", "hourly"]).unwrap_err(),
            "invalid value for --commits: hourly (expected daily, weekly, monthly, or yearly)"
        );
        assert_eq!(
            run(&["--commits", "daily", "--date"]).unwrap_err(),
            "--commits cannot be combined with --date or --non-blank"
        );
    }

    #[test]
    fn parses_language_chart_and_rejects_incompatible_modes() {
        assert_eq!(
            run(&["--languages", "--date"]).unwrap(),
            Options {
                date: true,
                languages: true,
                ..Options::new()
            }
        );
        assert_eq!(
            run(&["--languages", "--non-blank"]).unwrap_err(),
            "--languages cannot be combined with --commits or --non-blank"
        );
        assert_eq!(
            run(&["--languages", "--commits", "daily"]).unwrap_err(),
            "--languages cannot be combined with --commits or --non-blank"
        );
    }

    #[test]
    fn parses_full_width_independently_of_chart_mode() {
        let options = run(&["--full-width", "--languages", "--date"]).unwrap();

        assert!(options.full_width);
        assert!(options.languages);
        assert!(options.date);
    }

    #[test]
    fn rejects_missing_option_values_and_positional_revision() {
        assert_eq!(run(&["--rev"]).unwrap_err(), "missing value for --rev");
        assert_eq!(run(&["--path"]).unwrap_err(), "missing value for --path");
        assert_eq!(run(&["main"]).unwrap_err(), "unexpected argument: main");
    }
}
