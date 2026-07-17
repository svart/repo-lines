#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitInterval {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl CommitInterval {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            "yearly" => Some(Self::Yearly),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub date: bool,
    pub non_blank: bool,
    pub commits: Option<CommitInterval>,
    pub revision: String,
    pub path: String,
}

pub fn parse_options(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut arguments = arguments.into_iter();
    let mut options = Options {
        date: false,
        non_blank: false,
        commits: None,
        revision: "HEAD".to_owned(),
        path: ".".to_owned(),
    };

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--date" => options.date = true,
            "--non-blank" => options.non_blank = true,
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
    if options.commits.is_some() && (options.date || options.non_blank) {
        return Err("--commits cannot be combined with --date or --non-blank".to_owned());
    }
    Ok(options)
}

pub fn usage() -> &'static str {
    "Usage: repo-lines [OPTIONS]\n\nPlot line count or commit frequency along a Git revision's first-parent history.\n\nOptions:\n  --rev <REVISION>                         Revision to inspect [default: HEAD]\n  --path <PATH>                            Repository path [default: .]\n  --date                                   Print commit date and time\n  --non-blank                              Overlay non-blank lines in grey\n  --commits <daily|weekly|monthly|yearly>  Plot commit frequency instead of lines\n  -h, --help                               Print help\n  -V, --version                            Print version\n"
}
