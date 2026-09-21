use std::collections::BTreeMap;
use std::path::Path;

use crate::calendar::{CommitInterval, fill_empty_intervals};
use crate::git::git_checked;

/// Counts first-parent commits per calendar interval, oldest first. Intervals
/// without commits between the first and the last are present with a count of
/// zero, so the caller receives a gap-free series ready to draw.
pub(crate) fn collect_commit_counts(
    repo: &Path,
    revision: &str,
    interval: CommitInterval,
) -> Result<Vec<(String, u64)>, String> {
    let date_argument = format!("--date=format:{}", interval.date_format());
    let args = [
        "log",
        "--format=%cd",
        date_argument.as_str(),
        "--reverse",
        "--first-parent",
        revision,
    ];
    let output = git_checked(repo, &args)?;
    let dates = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("git log returned non-UTF-8 output: {error}"))?;
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for date in dates.lines() {
        *counts.entry(date.to_owned()).or_insert(0) += 1;
    }
    let counts: Vec<(String, u64)> = counts.into_iter().collect();
    fill_empty_intervals(&counts, interval)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::TempRepo;

    #[test]
    fn counts_first_parent_commits_in_calendar_intervals() {
        let repo = TempRepo::new();
        repo.commit_at("first", "2026-01-01T12:00:00+0000");
        repo.commit_at("second", "2026-01-01T13:00:00+0000");
        repo.commit_at("third", "2026-01-03T12:00:00+0000");

        assert_eq!(
            collect_commit_counts(repo.path(), "HEAD", CommitInterval::Daily).unwrap(),
            vec![
                ("2026-01-01".to_owned(), 2),
                ("2026-01-02".to_owned(), 0),
                ("2026-01-03".to_owned(), 1),
            ]
        );
    }
}
