use std::collections::BTreeMap;
use std::path::Path;

use crate::cli::CommitInterval;
use crate::{git, git_failure};

pub(crate) fn collect_commit_counts(
    repo: &Path,
    revision: &str,
    interval: CommitInterval,
) -> Result<Vec<(String, u64)>, String> {
    let date_format = match interval {
        CommitInterval::Daily => "%Y-%m-%d",
        CommitInterval::Weekly => "%G-W%V",
        CommitInterval::Monthly => "%Y-%m",
        CommitInterval::Yearly => "%Y",
    };
    let date_argument = format!("--date=format:{date_format}");
    let args = [
        "log",
        "--format=%cd",
        date_argument.as_str(),
        "--reverse",
        "--first-parent",
        revision,
    ];
    let output = git(repo, &args)?;
    if !output.status.success() {
        return Err(git_failure(&args, &output));
    }
    let dates = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("git log returned non-UTF-8 output: {error}"))?;
    let mut counts = BTreeMap::new();
    for date in dates.lines() {
        *counts.entry(date.to_owned()).or_insert(0) += 1;
    }
    Ok(counts.into_iter().collect())
}
