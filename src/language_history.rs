use std::collections::BTreeMap;
use std::path::Path;

use crate::language::{Language, classify_path};
use crate::{
    BlobReader, diff_history, git, git_failure, mode_has_blob, uses_external_attributes,
    uses_versioned_attributes,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LanguageSnapshot {
    sequence: usize,
    commit: String,
    datetime: String,
    pub(crate) lines: BTreeMap<Language, u64>,
}

impl LanguageSnapshot {
    pub(crate) fn new(
        sequence: usize,
        commit: &str,
        datetime: &str,
        lines: BTreeMap<Language, u64>,
    ) -> Self {
        Self {
            sequence,
            commit: commit.to_owned(),
            datetime: datetime.to_owned(),
            lines,
        }
    }

    pub(crate) fn label(&self, date: bool, sequence_width: usize) -> String {
        let short_hash = self.commit.get(..8).unwrap_or(&self.commit);
        if date {
            format!(
                "{}:{:0sequence_width$}:{short_hash}",
                self.datetime, self.sequence
            )
        } else {
            format!("{:0sequence_width$}:{short_hash}", self.sequence)
        }
    }
}

fn grep_language_counts(repo: &Path, commit: &str) -> Result<BTreeMap<Language, u64>, String> {
    let args = ["grep", "-I", "-c", "-z", "^", commit, "--"];
    let output = git(repo, &args)?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(git_failure(&args, &output));
    }

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

fn collect_with_grep(
    repo: &Path,
    commits: &[(String, String)],
) -> Result<Vec<LanguageSnapshot>, String> {
    commits
        .iter()
        .enumerate()
        .map(|(index, (commit, datetime))| {
            Ok(LanguageSnapshot::new(
                index + 1,
                commit,
                datetime,
                grep_language_counts(repo, commit)?,
            ))
        })
        .collect()
}

pub(crate) fn collect_language_history(
    repo: &Path,
    revision: &str,
) -> Result<Vec<LanguageSnapshot>, String> {
    let log_args = [
        "log",
        "--format=%H%x09%cd",
        "--date=format:%Y-%m-%d %H:%M:%S",
        "--reverse",
        "--first-parent",
        revision,
    ];
    let log = git(repo, &log_args)?;
    if !log.status.success() {
        return Err(git_failure(&log_args, &log));
    }
    let commit_output = std::str::from_utf8(&log.stdout)
        .map_err(|error| format!("git log returned non-UTF-8 output: {error}"))?;
    let commits: Vec<(String, String)> = commit_output
        .lines()
        .map(|line| {
            line.split_once('\t')
                .map(|(commit, datetime)| (commit.to_owned(), datetime.to_owned()))
                .ok_or_else(|| format!("malformed git log output: {line}"))
        })
        .collect::<Result<_, _>>()?;
    let commit_ids: Vec<&str> = commits.iter().map(|(commit, _)| commit.as_str()).collect();
    let changes = diff_history(repo, &commit_ids)?;
    if uses_versioned_attributes(&changes) || uses_external_attributes(repo, &changes)? {
        return collect_with_grep(repo, &commits);
    }

    let mut blobs = BlobReader::open(repo, false)?;
    let mut totals = BTreeMap::new();
    let mut history = Vec::with_capacity(commits.len());
    for (index, ((commit, datetime), changes)) in commits.iter().zip(changes).enumerate() {
        for change in changes {
            let language = classify_path(&change.path);
            if mode_has_blob(&change.old_mode) {
                let removed = blobs.line_count(&change.old_oid)?.all;
                let total = totals.entry(language).or_insert(0_u64);
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
                let total = totals.entry(language).or_insert(0_u64);
                *total = total.checked_add(added).ok_or_else(|| {
                    format!(
                        "{} line count overflow while adding {}",
                        language.name(),
                        change.new_oid
                    )
                })?;
            }
        }
        history.push(LanguageSnapshot::new(
            index + 1,
            commit,
            datetime,
            totals.clone(),
        ));
    }
    blobs.finish()?;
    Ok(history)
}
