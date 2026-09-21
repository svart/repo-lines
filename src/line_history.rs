//! Total-line history along a revision's first-parent path.

use std::path::Path;

use crate::commit::CommitRef;
use crate::git::{
    BlobReader, Delta, diff_history, first_parent_log, grep_line_count, mode_has_blob,
    needs_attribute_aware_count,
};
use crate::line_count::LineCount;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) at: CommitRef,
    pub(crate) lines: LineCount,
}

/// Walks the first-parent history oldest to newest, carrying a running total
/// that each commit's changed blobs adjust.
///
/// Repositories whose attribute rules can reclassify a blob as text or binary
/// fall back to counting every commit with `git grep`; see
/// `needs_attribute_aware_count`.
pub(crate) fn collect_history(
    repo: &Path,
    revision: &str,
    count_non_blank: bool,
) -> Result<Vec<Snapshot>, String> {
    let commits = first_parent_log(repo, revision)?;
    let commit_ids: Vec<&str> = commits.iter().map(|commit| commit.id.as_str()).collect();
    let changes = diff_history(repo, &commit_ids)?;
    drop(commit_ids);
    if needs_attribute_aware_count(repo, &changes)? {
        return collect_with_grep(repo, commits, count_non_blank);
    }

    let mut blobs = BlobReader::open(repo, count_non_blank)?;
    let mut total = LineCount::default();
    let mut history = Vec::with_capacity(commits.len());
    for (at, changes) in commits.into_iter().zip(changes) {
        for change in changes {
            apply(&mut total, &mut blobs, &change)?;
        }
        history.push(Snapshot { at, lines: total });
    }
    blobs.finish()?;
    Ok(history)
}

fn apply(total: &mut LineCount, blobs: &mut BlobReader, change: &Delta) -> Result<(), String> {
    if mode_has_blob(&change.old_mode) {
        let removed = blobs.line_count(&change.old_oid)?;
        total.all = total
            .all
            .checked_sub(removed.all)
            .ok_or_else(|| format!("line count underflow while removing {}", change.old_oid))?;
        total.non_blank = total
            .non_blank
            .checked_sub(removed.non_blank)
            .ok_or_else(|| {
                format!(
                    "non-blank line count underflow while removing {}",
                    change.old_oid
                )
            })?;
    }
    if mode_has_blob(&change.new_mode) {
        let added = blobs.line_count(&change.new_oid)?;
        total.all = total
            .all
            .checked_add(added.all)
            .ok_or_else(|| format!("line count overflow while adding {}", change.new_oid))?;
        total.non_blank = total
            .non_blank
            .checked_add(added.non_blank)
            .ok_or_else(|| {
                format!(
                    "non-blank line count overflow while adding {}",
                    change.new_oid
                )
            })?;
    }
    Ok(())
}

fn collect_with_grep(
    repo: &Path,
    commits: Vec<CommitRef>,
    count_non_blank: bool,
) -> Result<Vec<Snapshot>, String> {
    commits
        .into_iter()
        .map(|at| {
            let all = grep_line_count(repo, &at.id, "^")?;
            let non_blank = if count_non_blank {
                grep_line_count(repo, &at.id, "[^[:space:]]")?
            } else {
                0
            };
            Ok(Snapshot {
                at,
                lines: LineCount { all, non_blank },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::TempRepo;

    #[test]
    fn collects_non_blank_line_history() {
        let repo = TempRepo::new();
        repo.write("tracked.txt", b"one\n\n  \ntwo\n");
        repo.commit("add blank and non-blank lines");

        let snapshots = collect_history(repo.path(), "HEAD", true).unwrap();

        assert_eq!(snapshots[0].lines.all, 4);
        assert_eq!(snapshots[0].lines.non_blank, 2);
    }

    #[test]
    fn honors_historical_gitattributes_binary_overrides() {
        let repo = TempRepo::new();
        repo.write(
            ".gitattributes",
            b"*.forced-text diff\n*.forced-binary -diff\n",
        );
        repo.write("data.forced-text", b"one\0two\n\n  \n");
        repo.write("data.forced-binary", b"one\ntwo\n");
        repo.commit("add attribute overrides");

        let snapshots = collect_history(repo.path(), "HEAD", true).unwrap();

        assert_eq!(snapshots[0].lines.all, 5);
        assert_eq!(snapshots[0].lines.non_blank, 3);
    }

    #[test]
    fn honors_repository_local_attribute_overrides() {
        let repo = TempRepo::new();
        repo.write_git_info_attributes(b"*.forced-text diff\n*.forced-binary -diff\n");
        repo.write("data.forced-text", b"one\0two\n");
        repo.write("data.forced-binary", b"one\ntwo\n");
        repo.commit("add files with local attribute overrides");

        let snapshots = collect_history(repo.path(), "HEAD", true).unwrap();

        assert_eq!(snapshots[0].lines.all, 1);
        assert_eq!(snapshots[0].lines.non_blank, 1);
    }

    #[test]
    fn collects_oldest_first_snapshots_from_first_parent_only() {
        let repo = TempRepo::new();
        repo.write("tracked.txt", b"one\n");
        repo.write("ignored.bin", b"binary\0data\n");
        repo.commit("initial");
        let first = repo.head();

        repo.run(&["checkout", "-b", "side"]);
        repo.write("side.txt", b"side\nbranch\n");
        repo.commit("side");
        repo.run(&["checkout", "master"]);
        repo.write("tracked.txt", b"one\ntwo\nthree\n");
        repo.commit("main");
        repo.run(&["merge", "--no-ff", "side", "-m", "merge side"]);
        repo.run(&["rm", "tracked.txt"]);
        repo.commit("delete tracked file");

        let snapshots = collect_history(repo.path(), "HEAD", false).unwrap();

        assert_eq!(snapshots.len(), 4);
        assert_eq!(snapshots[0].at.sequence, 1);
        assert_eq!(snapshots[0].at.id, first);
        assert_eq!(snapshots[1].lines.all, 3);
        assert_eq!(snapshots[2].lines.all, 5);
        assert_eq!(snapshots[3].lines.all, 2);
    }
}
