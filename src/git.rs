//! Everything that shells out to `git`.
//!
//! This module is the only place that knows how Git is invoked and how its
//! output is framed. The history collectors above it work with `CommitRef`,
//! `Delta` and `BlobReader` and never build an argument list themselves.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, BufWriter, Read, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::thread;

use crate::commit::CommitRef;
use crate::line_count::{LineCount, text_line_counts};

/// One entry of a `git diff-tree --raw` record.
#[derive(Debug)]
pub(crate) struct Delta {
    pub(crate) old_mode: String,
    pub(crate) new_mode: String,
    pub(crate) old_oid: String,
    pub(crate) new_oid: String,
    pub(crate) path: Vec<u8>,
}

pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("could not run git: {error}"))
}

pub(crate) fn git_failure(args: &[&str], output: &Output) -> String {
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        format!("git {} failed with {}", args.join(" "), output.status)
    } else {
        format!("git {} failed: {detail}", args.join(" "))
    }
}

/// Runs `git` with `args`, succeeding or turning a non-zero exit into an error.
pub(crate) fn git_checked(repo: &Path, args: &[&str]) -> Result<Output, String> {
    let output = git(repo, args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(git_failure(args, &output))
    }
}

fn git_with_input(repo: &Path, args: &[&str], input: Vec<u8>) -> Result<Output, String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run git: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "could not open git stdin".to_owned())?;
    let writer = thread::spawn(move || stdin.write_all(&input));
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not wait for git: {error}"))?;
    writer
        .join()
        .map_err(|_| "git input writer panicked".to_owned())?
        .map_err(|error| format!("could not write to git: {error}"))?;
    if !output.status.success() {
        return Err(git_failure(args, &output));
    }
    Ok(output)
}

/// Lists the first-parent history of `revision`, oldest commit first.
pub(crate) fn first_parent_log(repo: &Path, revision: &str) -> Result<Vec<CommitRef>, String> {
    let args = [
        "log",
        "--format=%H%x09%cd",
        "--date=format:%Y-%m-%d %H:%M:%S",
        "--reverse",
        "--first-parent",
        revision,
    ];
    let output = git_checked(repo, &args)?;
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("git log returned non-UTF-8 output: {error}"))?;
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            line.split_once('\t')
                .map(|(id, datetime)| CommitRef::new(index + 1, id, datetime))
                .ok_or_else(|| format!("malformed git log output: {line}"))
        })
        .collect()
}

/// Diffs each commit against its predecessor in one `diff-tree` invocation.
/// The returned vector is parallel to `commits`; the first entry diffs against
/// the empty tree.
pub(crate) fn diff_history(repo: &Path, commits: &[&str]) -> Result<Vec<Vec<Delta>>, String> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }

    let mut input = Vec::new();
    writeln!(input, "{}", commits[0]).map_err(|error| error.to_string())?;
    for pair in commits.windows(2) {
        writeln!(input, "{} {}", pair[1], pair[0]).map_err(|error| error.to_string())?;
    }

    let args = [
        "diff-tree",
        "--stdin",
        "--root",
        "-r",
        "--raw",
        "-z",
        "--no-renames",
        "--no-abbrev",
    ];
    let output = git_with_input(repo, &args, input)?;
    parse_diff_history(&output.stdout, commits)
}

fn parse_diff_history(output: &[u8], commits: &[&str]) -> Result<Vec<Vec<Delta>>, String> {
    let mut tokens: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    if tokens.last() == Some(&&[][..]) {
        tokens.pop();
    }
    let mut cursor = 0;
    let mut history = Vec::with_capacity(commits.len());

    for (index, commit) in commits.iter().enumerate() {
        let expected_header = *commit;
        let Some(header) = tokens.get(cursor) else {
            history.push(Vec::new());
            continue;
        };
        if *header != expected_header.as_bytes() {
            // diff-tree emits no header or records for an unchanged comparison.
            if commits[index + 1..]
                .iter()
                .any(|future| future.as_bytes() == *header)
            {
                history.push(Vec::new());
                continue;
            }
            return Err(format!(
                "unexpected diff-tree header: {}",
                String::from_utf8_lossy(header)
            ));
        }
        cursor += 1;

        let mut changes = Vec::new();
        while tokens
            .get(cursor)
            .is_some_and(|token| token.starts_with(b":"))
        {
            let metadata = std::str::from_utf8(&tokens[cursor][1..])
                .map_err(|error| format!("non-UTF-8 diff-tree metadata: {error}"))?;
            let mut fields = metadata.split_ascii_whitespace();
            let old_mode = fields.next();
            let new_mode = fields.next();
            let old_oid = fields.next();
            let new_oid = fields.next();
            let status = fields.next();
            if fields.next().is_some()
                || old_mode.is_none()
                || new_mode.is_none()
                || old_oid.is_none()
                || new_oid.is_none()
                || status.is_none()
            {
                return Err(format!("malformed diff-tree metadata: {metadata}"));
            }
            if status.is_some_and(|value| value.len() != 1) {
                return Err(format!("unexpected diff-tree status: {metadata}"));
            }
            if tokens.get(cursor + 1).is_none() {
                return Err("missing path after diff-tree metadata".to_owned());
            }
            changes.push(Delta {
                old_mode: old_mode.unwrap().to_owned(),
                new_mode: new_mode.unwrap().to_owned(),
                old_oid: old_oid.unwrap().to_owned(),
                new_oid: new_oid.unwrap().to_owned(),
                path: tokens[cursor + 1].to_owned(),
            });
            cursor += 2;
        }
        history.push(changes);
    }

    if cursor != tokens.len() {
        return Err("unexpected trailing diff-tree output".to_owned());
    }
    Ok(history)
}

pub(crate) fn mode_has_blob(mode: &str) -> bool {
    mode.starts_with("100") || mode == "120000"
}

/// Reports whether any Git attribute rule can change what counts as text.
///
/// Incremental counting reuses a blob's line count across commits, which is
/// only sound while the text/binary decision is a property of the blob itself.
/// A `.gitattributes` file anywhere in the history, or a repository-local rule
/// covering a touched path, breaks that assumption and forces the slower
/// commit-by-commit `git grep` path.
pub(crate) fn needs_attribute_aware_count(
    repo: &Path,
    history: &[Vec<Delta>],
) -> Result<bool, String> {
    if uses_versioned_attributes(history) {
        return Ok(true);
    }
    uses_external_attributes(repo, history)
}

fn uses_versioned_attributes(history: &[Vec<Delta>]) -> bool {
    history.iter().flatten().any(|change| {
        change.path.rsplit(|byte| *byte == b'/').next() == Some(b".gitattributes".as_slice())
    })
}

fn uses_external_attributes(repo: &Path, history: &[Vec<Delta>]) -> Result<bool, String> {
    let mut paths = HashSet::new();
    let mut input = Vec::new();
    for change in history.iter().flatten() {
        if paths.insert(change.path.as_slice()) {
            input.extend_from_slice(&change.path);
            input.push(0);
        }
    }
    let args = ["check-attr", "-z", "--stdin", "diff"];
    let output = git_with_input(repo, &args, input)?;
    let fields: Vec<&[u8]> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect();
    let (records, remainder) = fields.as_chunks::<3>();
    if !remainder.is_empty() {
        return Err("malformed git check-attr output".to_owned());
    }
    Ok(records
        .iter()
        .any(|record| record[1] != b"diff" || record[2] != b"unspecified"))
}

/// Counts lines with `git grep`, which applies the attribute rules in effect at
/// `commit`. `pattern` selects which lines are counted.
pub(crate) fn grep_line_count(repo: &Path, commit: &str, pattern: &str) -> Result<u64, String> {
    let args = ["grep", "-I", "-c", pattern, commit, "--"];
    let output = grep(repo, &args)?;
    sum_grep_counts(&output.stdout)
}

/// Runs `git grep`, treating exit status 1 ("no match") as success.
pub(crate) fn grep(repo: &Path, args: &[&str]) -> Result<Output, String> {
    let output = git(repo, args)?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(git_failure(args, &output));
    }
    Ok(output)
}

fn sum_grep_counts(output: &[u8]) -> Result<u64, String> {
    let output = std::str::from_utf8(output)
        .map_err(|error| format!("git grep returned non-UTF-8 output: {error}"))?;
    output.lines().try_fold(0_u64, |sum, line| {
        let count = line
            .rsplit_once(':')
            .map(|(_, count)| count)
            .ok_or_else(|| format!("malformed git grep output: {line}"))?;
        let count = count
            .parse::<u64>()
            .map_err(|_| format!("invalid line count in git grep output: {count}"))?;
        sum.checked_add(count)
            .ok_or_else(|| "total line count overflowed u64".to_owned())
    })
}

/// A long-lived `git cat-file --batch` process with a per-blob line-count cache.
pub(crate) struct BlobReader {
    child: Option<Child>,
    input: Option<BufWriter<ChildStdin>>,
    output: BufReader<ChildStdout>,
    cache: HashMap<String, LineCount>,
    count_non_blank: bool,
}

impl BlobReader {
    pub(crate) fn open(repo: &Path, count_non_blank: bool) -> Result<Self, String> {
        let mut child = Command::new("git")
            .args(["cat-file", "--batch"])
            .current_dir(repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run git cat-file: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| "could not open git cat-file stdin".to_owned())?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| "could not open git cat-file stdout".to_owned())?;
        Ok(Self {
            child: Some(child),
            input: Some(BufWriter::new(input)),
            output: BufReader::new(output),
            cache: HashMap::new(),
            count_non_blank,
        })
    }

    pub(crate) fn line_count(&mut self, oid: &str) -> Result<LineCount, String> {
        if let Some(lines) = self.cache.get(oid) {
            return Ok(*lines);
        }

        let input = self
            .input
            .as_mut()
            .ok_or_else(|| "git cat-file input is closed".to_owned())?;
        writeln!(input, "{oid}").map_err(|error| format!("could not query blob: {error}"))?;
        input
            .flush()
            .map_err(|error| format!("could not query blob: {error}"))?;

        let mut header = String::new();
        self.output
            .read_line(&mut header)
            .map_err(|error| format!("could not read blob header: {error}"))?;
        let mut fields = header.split_ascii_whitespace();
        let returned_oid = fields.next().unwrap_or_default();
        let object_type = fields.next().unwrap_or_default();
        let size = fields
            .next()
            .ok_or_else(|| format!("malformed cat-file header: {header:?}"))?
            .parse::<usize>()
            .map_err(|_| format!("invalid blob size in cat-file header: {header:?}"))?;
        if returned_oid != oid || object_type != "blob" || fields.next().is_some() {
            return Err(format!("unexpected cat-file header: {header:?}"));
        }

        let mut contents = vec![0; size];
        self.output
            .read_exact(&mut contents)
            .map_err(|error| format!("could not read blob {oid}: {error}"))?;
        let mut terminator = [0];
        self.output
            .read_exact(&mut terminator)
            .map_err(|error| format!("could not read blob terminator: {error}"))?;
        if terminator[0] != b'\n' {
            return Err(format!("invalid blob terminator for {oid}"));
        }

        let lines = text_line_counts(&contents, self.count_non_blank);
        self.cache.insert(oid.to_owned(), lines);
        Ok(lines)
    }

    pub(crate) fn finish(mut self) -> Result<(), String> {
        self.input.take();
        let mut child = self.child.take().expect("child is present until finish");
        let status = child
            .wait()
            .map_err(|error| format!("could not wait for git cat-file: {error}"))?;
        if status.success() {
            return Ok(());
        }
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut detail);
        }
        Err(if detail.trim().is_empty() {
            format!("git cat-file failed with {status}")
        } else {
            format!("git cat-file failed: {}", detail.trim())
        })
    }
}

impl Drop for BlobReader {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_diff_tree_comparisons_without_headers() {
        let commits = ["first", "empty", "third"];
        let output = b"first\0\
            :000000 100644 0000000000000000000000000000000000000000 \
            1111111111111111111111111111111111111111 A\0first.txt\0\
            third\0\
            :100644 100644 1111111111111111111111111111111111111111 \
            2222222222222222222222222222222222222222 M\0third.txt\0";

        let history = parse_diff_history(output, &commits).unwrap();

        assert_eq!(history.len(), 3);
        assert_eq!(history[0].len(), 1);
        assert!(history[1].is_empty());
        assert_eq!(history[2].len(), 1);
        assert_eq!(history[2][0].path, b"third.txt");
    }

    #[test]
    fn rejects_truncated_and_trailing_diff_tree_output() {
        assert!(parse_diff_history(b"other\0", &["first"]).is_err());
        assert!(
            parse_diff_history(
                b"first\0:100644 100644 1111 2222 M\0only.txt\0extra\0",
                &["first"],
            )
            .is_err()
        );
    }

    #[test]
    fn sums_grep_counts_and_rejects_malformed_lines() {
        assert_eq!(sum_grep_counts(b"HEAD:a.rs:3\nHEAD:b.rs:4\n").unwrap(), 7);
        assert!(sum_grep_counts(b"no-colon\n").is_err());
    }

    #[test]
    fn recognizes_blob_modes() {
        assert!(mode_has_blob("100644"));
        assert!(mode_has_blob("100755"));
        assert!(mode_has_blob("120000"));
        assert!(!mode_has_blob("040000"));
        assert!(!mode_has_blob("000000"));
    }

    #[test]
    fn reads_the_first_parent_log_oldest_first() {
        let repo = crate::test_repo::TempRepo::new();
        repo.write("a.txt", b"one\n");
        repo.commit("first");
        let first = repo.head();
        repo.write("a.txt", b"one\ntwo\n");
        repo.commit("second");

        let log = first_parent_log(repo.path(), "HEAD").unwrap();

        assert_eq!(log.len(), 2);
        assert_eq!(log[0].sequence, 1);
        assert_eq!(log[0].id, first);
        assert_eq!(log[1].sequence, 2);
    }
}
