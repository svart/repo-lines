//! Per-language line history along a revision's first-parent path.

use std::collections::BTreeMap;
use std::path::Path;

use crate::commit::CommitRef;
use crate::git::{
    BlobReader, diff_history, first_parent_log, grep, mode_has_blob, needs_attribute_aware_count,
};
use crate::language::{Language, classify_path};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LanguageSnapshot {
    pub(crate) at: CommitRef,
    pub(crate) lines: BTreeMap<Language, u64>,
}

/// Walks the first-parent history oldest to newest, keeping a running per-language
/// total. Mirrors `line_history::collect_history`, including its `git grep`
/// fallback for attribute-sensitive repositories.
pub(crate) fn collect_language_history(
    repo: &Path,
    revision: &str,
) -> Result<Vec<LanguageSnapshot>, String> {
    let commits = first_parent_log(repo, revision)?;
    let commit_ids: Vec<&str> = commits.iter().map(|commit| commit.id.as_str()).collect();
    let changes = diff_history(repo, &commit_ids)?;
    drop(commit_ids);
    if needs_attribute_aware_count(repo, &changes)? {
        return collect_with_grep(repo, commits);
    }

    let mut blobs = BlobReader::open(repo, false)?;
    let mut totals: BTreeMap<Language, u64> = BTreeMap::new();
    let mut history = Vec::with_capacity(commits.len());
    for (at, changes) in commits.into_iter().zip(changes) {
        for change in changes {
            let language = classify_path(&change.path);
            if mode_has_blob(&change.old_mode) {
                let removed = blobs.line_count(&change.old_oid)?.all;
                let total = totals.entry(language).or_insert(0);
                *total = total.checked_sub(removed).ok_or_else(|| {
                    format!(
                        "{} line count underflow while removing {}",
                        language.name(),
                        change.old_oid
                    )
                })?;
                if *total == 0 {
                    totals.remove(&language);
                }
            }
            if mode_has_blob(&change.new_mode) {
                let added = blobs.line_count(&change.new_oid)?.all;
                let total = totals.entry(language).or_insert(0);
                *total = total.checked_add(added).ok_or_else(|| {
                    format!(
                        "{} line count overflow while adding {}",
                        language.name(),
                        change.new_oid
                    )
                })?;
            }
        }
        history.push(LanguageSnapshot {
            at,
            lines: totals.clone(),
        });
    }
    blobs.finish()?;
    Ok(history)
}

fn collect_with_grep(
    repo: &Path,
    commits: Vec<CommitRef>,
) -> Result<Vec<LanguageSnapshot>, String> {
    commits
        .into_iter()
        .map(|at| {
            let lines = grep_language_counts(repo, &at.id)?;
            Ok(LanguageSnapshot { at, lines })
        })
        .collect()
}

/// Counts lines per language at one commit with `git grep -z`, whose records
/// are `<commit>:<path>\0<count>\n`.
fn grep_language_counts(repo: &Path, commit: &str) -> Result<BTreeMap<Language, u64>, String> {
    let args = ["grep", "-I", "-c", "-z", "^", commit, "--"];
    let output = grep(repo, &args)?;

    let mut totals = BTreeMap::new();
    let mut cursor = 0;
    let mut prefix = commit.as_bytes().to_vec();
    prefix.push(b':');
    while cursor < output.stdout.len() {
        let nul = output.stdout[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| cursor + offset)
            .ok_or_else(|| "malformed git grep language path".to_owned())?;
        let path = output.stdout[cursor..nul]
            .strip_prefix(prefix.as_slice())
            .ok_or_else(|| "unexpected git grep language path".to_owned())?;
        let count_start = nul + 1;
        let newline = output.stdout[count_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| count_start + offset)
            .ok_or_else(|| "malformed git grep language count".to_owned())?;
        let count = std::str::from_utf8(&output.stdout[count_start..newline])
            .map_err(|error| format!("git grep returned a non-UTF-8 count: {error}"))?
            .parse::<u64>()
            .map_err(|_| "git grep returned an invalid language count".to_owned())?;
        let total = totals.entry(classify_path(path)).or_insert(0_u64);
        *total = total
            .checked_add(count)
            .ok_or_else(|| "language line count overflowed u64".to_owned())?;
        cursor = newline + 1;
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_repo::TempRepo;

    #[test]
    fn collects_language_fractions_across_file_changes() {
        let repo = TempRepo::new();
        repo.write("main.rs", b"one\ntwo\n");
        repo.write("README.md", b"intro\n");
        repo.commit("add rust and markdown");
        repo.write("main.rs", b"one\ntwo\nthree\nfour\n");
        repo.run(&["mv", "README.md", "notes.py"]);
        repo.commit("grow rust and reclassify markdown");

        let snapshots = collect_language_history(repo.path(), "HEAD").unwrap();

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].lines.get(&Language::Rust), Some(&2));
        assert_eq!(snapshots[0].lines.get(&Language::Markdown), Some(&1));
        assert_eq!(snapshots[1].lines.get(&Language::Rust), Some(&4));
        assert_eq!(snapshots[1].lines.get(&Language::Python), Some(&1));
        assert_eq!(snapshots[1].lines.get(&Language::Markdown), None);
    }

    #[test]
    fn honors_historical_attribute_overrides() {
        let repo = TempRepo::new();
        repo.write(".gitattributes", b"*.rs -diff\n*.data diff\n");
        repo.write("ignored.rs", b"one\ntwo\n");
        repo.write("included.data", b"three\0four\n");
        repo.commit("add attribute overrides");

        let snapshots = collect_language_history(repo.path(), "HEAD").unwrap();

        assert_eq!(snapshots[0].lines.get(&Language::Rust), None);
        assert_eq!(snapshots[0].lines.get(&Language::Other), Some(&3));
        assert_eq!(snapshots[0].lines.values().sum::<u64>(), 3);
    }
}
