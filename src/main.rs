use std::env;
use std::io::{BufRead, BufReader, BufWriter, IsTerminal, Read, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitCode, Output, Stdio};
use std::{
    collections::{HashMap, HashSet},
    thread,
};
use terminal_size::{Width, terminal_size};

mod chart;
mod cli;
mod commit_frequency;
mod language;
mod language_history;

#[cfg(test)]
use chart::render_layered_bar;
use chart::{
    commit_chart_reserved_width, language_chart_reserved_width, line_chart_reserved_width,
};
use chart::{render_chart, render_commit_chart, render_language_chart};
#[cfg(test)]
use cli::CommitInterval;
#[cfg(test)]
use cli::Options;
use cli::{parse_options, usage};
use commit_frequency::collect_commit_counts;
#[cfg(test)]
use language::Language;
#[cfg(test)]
use language_history::LanguageSnapshot;
use language_history::collect_language_history;

const BAR_WIDTH: usize = 50;

fn choose_bar_width(
    full_width: bool,
    terminal_columns: Option<usize>,
    reserved_width: usize,
) -> usize {
    if full_width {
        terminal_columns
            .map(|columns| columns.saturating_sub(reserved_width))
            .unwrap_or(BAR_WIDTH)
    } else {
        BAR_WIDTH
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    sequence: usize,
    commit: String,
    datetime: String,
    lines: u64,
    non_blank_lines: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LineCount {
    all: u64,
    non_blank: u64,
}

#[derive(Debug)]
struct Delta {
    old_mode: String,
    new_mode: String,
    old_oid: String,
    new_oid: String,
    path: Vec<u8>,
}

impl Snapshot {
    pub(crate) fn new(
        sequence: usize,
        commit: &str,
        datetime: &str,
        lines: u64,
        non_blank_lines: u64,
    ) -> Self {
        Self {
            sequence,
            commit: commit.to_owned(),
            datetime: datetime.to_owned(),
            lines,
            non_blank_lines,
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
    Ok(output)
}

fn diff_history(repo: &Path, commits: &[&str]) -> Result<Vec<Vec<Delta>>, String> {
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
    if !output.status.success() {
        return Err(git_failure(&args, &output));
    }
    parse_diff_history(&output.stdout, commits)
}

fn parse_diff_history(output: &[u8], commits: &[&str]) -> Result<Vec<Vec<Delta>>, String> {
    let mut tokens: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    if tokens.last() == Some(&&[][..]) {
        tokens.pop();
    }
    let mut cursor = 0;
    let mut history = Vec::with_capacity(commits.len());

    for commit in commits {
        let expected_header = *commit;
        let header = tokens
            .get(cursor)
            .ok_or_else(|| format!("missing diff-tree header for {commit}"))?;
        if *header != expected_header.as_bytes() {
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

fn mode_has_blob(mode: &str) -> bool {
    mode.starts_with("100") || mode == "120000"
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
    if !output.status.success() {
        return Err(git_failure(&args, &output));
    }
    let fields: Vec<&[u8]> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect();
    if !fields.len().is_multiple_of(3) {
        return Err("malformed git check-attr output".to_owned());
    }
    Ok(fields
        .chunks_exact(3)
        .any(|record| record[1] != b"diff" || record[2] != b"unspecified"))
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

fn collect_history_with_grep(
    repo: &Path,
    commits: &[(String, String)],
    count_non_blank: bool,
) -> Result<Vec<Snapshot>, String> {
    commits
        .iter()
        .enumerate()
        .map(|(index, (commit, datetime))| {
            let all = grep_line_count(repo, commit, "^")?;
            let non_blank = if count_non_blank {
                grep_line_count(repo, commit, "[^[:space:]]")?
            } else {
                0
            };
            Ok(Snapshot::new(index + 1, commit, datetime, all, non_blank))
        })
        .collect()
}

fn grep_line_count(repo: &Path, commit: &str, pattern: &str) -> Result<u64, String> {
    let args = ["grep", "-I", "-c", pattern, commit, "--"];
    let output = git(repo, &args)?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(git_failure(&args, &output));
    }
    sum_grep_counts(&output.stdout)
}

fn text_line_counts(contents: &[u8], count_non_blank: bool) -> LineCount {
    const BINARY_SNIFF_BYTES: usize = 8_000;
    if contents[..contents.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return LineCount::default();
    }
    if contents.is_empty() {
        return LineCount::default();
    }

    let mut count = LineCount::default();
    let mut line_has_content = false;
    for byte in contents {
        if *byte == b'\n' {
            count.all += 1;
            count.non_blank += u64::from(line_has_content);
            line_has_content = false;
        } else if count_non_blank && !byte.is_ascii_whitespace() {
            line_has_content = true;
        }
    }
    if contents.last() != Some(&b'\n') {
        count.all += 1;
        count.non_blank += u64::from(line_has_content);
    }
    count
}

struct BlobReader {
    child: Option<Child>,
    input: Option<BufWriter<ChildStdin>>,
    output: BufReader<ChildStdout>,
    cache: HashMap<String, LineCount>,
    count_non_blank: bool,
}

impl BlobReader {
    fn open(repo: &Path, count_non_blank: bool) -> Result<Self, String> {
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

    fn line_count(&mut self, oid: &str) -> Result<LineCount, String> {
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

    fn finish(mut self) -> Result<(), String> {
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

fn collect_history(
    repo: &Path,
    revision: &str,
    count_non_blank: bool,
) -> Result<Vec<Snapshot>, String> {
    let rev_list_args = [
        "log",
        "--format=%H%x09%cd",
        "--date=format:%Y-%m-%d %H:%M:%S",
        "--reverse",
        "--first-parent",
        revision,
    ];
    let rev_list = git(repo, &rev_list_args)?;
    if !rev_list.status.success() {
        return Err(git_failure(&rev_list_args, &rev_list));
    }
    let commit_output = std::str::from_utf8(&rev_list.stdout)
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
        return collect_history_with_grep(repo, &commits, count_non_blank);
    }
    let mut blobs = BlobReader::open(repo, count_non_blank)?;
    let mut total = LineCount::default();
    let mut history = Vec::with_capacity(commits.len());

    for (index, ((commit, datetime), changes)) in commits.iter().zip(changes).enumerate() {
        for change in changes {
            if mode_has_blob(&change.old_mode) {
                let removed = blobs.line_count(&change.old_oid)?;
                total.all = total.all.checked_sub(removed.all).ok_or_else(|| {
                    format!("line count underflow while removing {}", change.old_oid)
                })?;
                total.non_blank =
                    total
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
                total.all = total.all.checked_add(added.all).ok_or_else(|| {
                    format!("line count overflow while adding {}", change.new_oid)
                })?;
                total.non_blank =
                    total
                        .non_blank
                        .checked_add(added.non_blank)
                        .ok_or_else(|| {
                            format!(
                                "non-blank line count overflow while adding {}",
                                change.new_oid
                            )
                        })?;
            }
        }
        history.push(Snapshot::new(
            index + 1,
            commit,
            datetime,
            total.all,
            total.non_blank,
        ));
    }
    blobs.finish()?;
    Ok(history)
}

fn run() -> Result<(), String> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        print!("{}", usage());
        return Ok(());
    }
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "-V" | "--version"))
    {
        println!("repo-lines {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let options = parse_options(arguments).map_err(|error| format!("{error}\n\n{}", usage()))?;

    let repo = Path::new(&options.path);
    let terminal_columns = options
        .full_width
        .then(terminal_size)
        .flatten()
        .map(|(Width(columns), _)| usize::from(columns));
    if let Some(interval) = options.commits {
        let counts = collect_commit_counts(repo, &options.revision, interval)?;
        let width = choose_bar_width(
            options.full_width,
            terminal_columns,
            commit_chart_reserved_width(&counts),
        );
        print!("{}", render_commit_chart(&counts, interval, width));
    } else if options.languages {
        let history = collect_language_history(repo, &options.revision)?;
        let width = choose_bar_width(
            options.full_width,
            terminal_columns,
            language_chart_reserved_width(&history, options.date),
        );
        print!(
            "{}",
            render_language_chart(
                &history,
                width,
                options.date,
                std::io::stdout().is_terminal()
            )
        );
    } else {
        let history = collect_history(repo, &options.revision, options.non_blank)?;
        let width = choose_bar_width(
            options.full_width,
            terminal_columns,
            line_chart_reserved_width(&history, options.date),
        );
        print!(
            "{}",
            render_chart(
                &history,
                width,
                options.date,
                options.non_blank,
                std::io::stdout().is_terminal()
            )
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("repo-lines: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn parses_default_options() {
        assert_eq!(
            parse_options(Vec::<String>::new()).unwrap(),
            Options {
                date: false,
                non_blank: false,
                languages: false,
                full_width: false,
                commits: None,
                revision: "HEAD".to_owned(),
                path: ".".to_owned(),
            }
        );
    }

    #[test]
    fn parses_revision_path_and_date_options() {
        let arguments = [
            "--path",
            "/tmp/project",
            "--date",
            "--non-blank",
            "--rev",
            "main",
        ]
        .map(str::to_owned);

        assert_eq!(
            parse_options(arguments).unwrap(),
            Options {
                date: true,
                non_blank: true,
                languages: false,
                full_width: false,
                commits: None,
                revision: "main".to_owned(),
                path: "/tmp/project".to_owned(),
            }
        );
    }

    #[test]
    fn parses_commit_interval_and_rejects_incompatible_chart_options() {
        assert_eq!(
            parse_options(["--commits", "monthly"].map(str::to_owned)).unwrap(),
            Options {
                date: false,
                non_blank: false,
                languages: false,
                full_width: false,
                commits: Some(CommitInterval::Monthly),
                revision: "HEAD".to_owned(),
                path: ".".to_owned(),
            }
        );
        assert_eq!(
            parse_options(["--commits", "hourly"].map(str::to_owned)).unwrap_err(),
            "invalid value for --commits: hourly (expected daily, weekly, monthly, or yearly)"
        );
        assert_eq!(
            parse_options(["--commits", "daily", "--date"].map(str::to_owned)).unwrap_err(),
            "--commits cannot be combined with --date or --non-blank"
        );
    }

    #[test]
    fn parses_language_chart_and_rejects_incompatible_modes() {
        assert_eq!(
            parse_options(["--languages", "--date"].map(str::to_owned)).unwrap(),
            Options {
                date: true,
                non_blank: false,
                languages: true,
                full_width: false,
                commits: None,
                revision: "HEAD".to_owned(),
                path: ".".to_owned(),
            }
        );
        assert_eq!(
            parse_options(["--languages", "--non-blank"].map(str::to_owned)).unwrap_err(),
            "--languages cannot be combined with --commits or --non-blank"
        );
        assert_eq!(
            parse_options(["--languages", "--commits", "daily"].map(str::to_owned)).unwrap_err(),
            "--languages cannot be combined with --commits or --non-blank"
        );
    }

    #[test]
    fn parses_full_width_independently_of_chart_mode() {
        let options =
            parse_options(["--full-width", "--languages", "--date"].map(str::to_owned)).unwrap();

        assert!(options.full_width);
        assert!(options.languages);
        assert!(options.date);
    }

    #[test]
    fn chooses_full_terminal_width_only_when_requested_and_available() {
        assert_eq!(choose_bar_width(false, Some(120), 30), BAR_WIDTH);
        assert_eq!(choose_bar_width(true, Some(120), 30), 90);
        assert_eq!(choose_bar_width(true, Some(20), 30), 0);
        assert_eq!(choose_bar_width(true, None, 30), BAR_WIDTH);
    }

    #[test]
    fn reserves_non_bar_columns_for_each_chart_mode() {
        let lines = vec![Snapshot::new(
            1,
            "0123456789abcdef",
            "2026-07-15 10:00:00",
            10,
            5,
        )];
        let languages = vec![LanguageSnapshot::new(
            1,
            "0123456789abcdef",
            "2026-07-15 10:00:00",
            BTreeMap::from([(Language::Rust, 10)]),
        )];

        assert_eq!(line_chart_reserved_width(&lines, false), 15);
        assert_eq!(
            commit_chart_reserved_width(&[("2026-07-15".to_owned(), 10)]),
            15
        );
        assert_eq!(language_chart_reserved_width(&languages, false), 12);
    }

    #[test]
    fn rejects_missing_option_values_and_positional_revision() {
        assert_eq!(
            parse_options(["--rev".to_owned()]).unwrap_err(),
            "missing value for --rev"
        );
        assert_eq!(
            parse_options(["--path".to_owned()]).unwrap_err(),
            "missing value for --path"
        );
        assert_eq!(
            parse_options(["main".to_owned()]).unwrap_err(),
            "unexpected argument: main"
        );
    }

    #[test]
    fn renders_daily_commit_counts_with_empty_days() {
        let commits = vec![("2026-07-14".to_owned(), 2), ("2026-07-16".to_owned(), 1)];

        assert_eq!(
            render_commit_chart(&commits, CommitInterval::Daily, 10),
            "    0 commits\n\
             2026-07-14  ██████████ 2\n\
             2026-07-15  0\n\
             2026-07-16  █████ 1\n"
        );
    }

    #[test]
    fn renders_weekly_monthly_and_yearly_commit_counts() {
        assert_eq!(
            render_commit_chart(
                &[("2026-W01".to_owned(), 1), ("2026-W03".to_owned(), 2)],
                CommitInterval::Weekly,
                10,
            ),
            "    0 commits\n\
             2026-W01  █████ 1\n\
             2026-W02  0\n\
             2026-W03  ██████████ 2\n"
        );
        assert_eq!(
            render_commit_chart(
                &[("2025-11".to_owned(), 1), ("2026-01".to_owned(), 2)],
                CommitInterval::Monthly,
                10,
            ),
            "    0 commits\n\
             2025-11  █████ 1\n\
             2025-12  0\n\
             2026-01  ██████████ 2\n"
        );
        assert_eq!(
            render_commit_chart(
                &[("2024".to_owned(), 1), ("2026".to_owned(), 2)],
                CommitInterval::Yearly,
                10,
            ),
            "    0 commits\n\
             2024  █████ 1\n\
             2025  0\n\
             2026  ██████████ 2\n"
        );
    }

    #[test]
    fn counts_all_text_lines_and_ignores_binary_blobs() {
        assert_eq!(text_line_counts(b"", false).all, 0);
        assert_eq!(text_line_counts(b"one\ntwo\n", false).all, 2);
        assert_eq!(text_line_counts(b"one\ntwo", false).all, 2);
        assert_eq!(text_line_counts(b"binary\0data\n", false).all, 0);
    }

    #[test]
    fn counts_all_and_non_blank_text_lines_in_one_pass() {
        assert_eq!(
            text_line_counts(b"one\n\n  \n\ttwo\nthree", true),
            LineCount {
                all: 5,
                non_blank: 3,
            }
        );
        assert_eq!(
            text_line_counts(b"binary\0data\n", true),
            LineCount::default()
        );
    }

    #[test]
    fn skips_non_blank_counting_when_not_requested() {
        assert_eq!(
            text_line_counts(b"one\n\nthree\n", false),
            LineCount {
                all: 3,
                non_blank: 0,
            }
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

        assert_eq!(snapshots[0].lines, 5);
        assert_eq!(snapshots[0].non_blank_lines, 3);
    }

    #[test]
    fn honors_repository_local_attribute_overrides() {
        let repo = TempRepo::new();
        repo.write_git_info_attributes(b"*.forced-text diff\n*.forced-binary -diff\n");
        repo.write("data.forced-text", b"one\0two\n");
        repo.write("data.forced-binary", b"one\ntwo\n");
        repo.commit("add files with local attribute overrides");

        let snapshots = collect_history(repo.path(), "HEAD", true).unwrap();

        assert_eq!(snapshots[0].lines, 1);
        assert_eq!(snapshots[0].non_blank_lines, 1);
    }

    #[test]
    fn renders_scaled_bars_in_history_order() {
        let history = vec![
            Snapshot::new(1, "0123456789abcdef", "2026-07-15 10:00:00", 5, 4),
            Snapshot::new(2, "fedcba9876543210", "2026-07-15 11:00:00", 10, 8),
            Snapshot::new(3, "aabbccddeeff0011", "2026-07-15 12:00:00", 0, 0),
        ];

        assert_eq!(
            render_chart(&history, 10, false, false, false),
            "        0 LoC\n\
             1:01234567  █████ 5\n\
             2:fedcba98  ██████████ 10\n\
             3:aabbccdd  0\n"
        );

        assert_eq!(
            render_chart(&history, 10, true, false, false),
            "        0 LoC\n\
             2026-07-15 10:00:00:1:01234567  █████ 5\n\
             2026-07-15 11:00:00:2:fedcba98  ██████████ 10\n\
             2026-07-15 12:00:00:3:aabbccdd  0\n"
        );
    }

    #[test]
    fn renders_language_fractions_with_stable_plain_text_symbols() {
        let history = vec![
            LanguageSnapshot::new(
                1,
                "0123456789abcdef",
                "2026-07-15 10:00:00",
                BTreeMap::from([(Language::Rust, 3), (Language::Markdown, 1)]),
            ),
            LanguageSnapshot::new(
                2,
                "fedcba9876543210",
                "2026-07-15 11:00:00",
                BTreeMap::from([(Language::Rust, 2), (Language::Python, 2)]),
            ),
        ];

        let chart = render_language_chart(&history, 10, false, false);

        assert!(chart.contains("1:01234567  AAACCCCCCC\n"));
        assert!(chart.contains("2:fedcba98  BBBBBCCCCC\n"));
        assert!(chart.ends_with("Legend: A Markdown, B Python, C Rust\n"));
    }

    #[test]
    fn language_fraction_rounding_always_fills_the_bar() {
        let history = vec![LanguageSnapshot::new(
            1,
            "0123456789abcdef",
            "2026-07-15 10:00:00",
            BTreeMap::from([
                (Language::Rust, 1),
                (Language::Python, 1),
                (Language::Markdown, 1),
            ]),
        )];

        let chart = render_language_chart(&history, 10, false, false);
        let bar = chart
            .lines()
            .find(|line| line.contains("1:01234567"))
            .and_then(|line| line.rsplit_once("  ").map(|(_, bar)| bar))
            .unwrap();

        assert_eq!(bar.chars().count(), 10);
    }

    #[test]
    fn leaves_the_default_chart_uncolored() {
        let history = vec![Snapshot::new(
            1,
            "0123456789abcdef",
            "2026-07-15 10:00:00",
            10,
            5,
        )];

        assert_eq!(
            render_chart(&history, 10, false, false, true),
            "        0 LoC\n1:01234567  ██████████ 10\n"
        );
    }

    #[test]
    fn collects_non_blank_line_history() {
        let repo = TempRepo::new();
        repo.write("tracked.txt", b"one\n\n  \ntwo\n");
        repo.commit("add blank and non-blank lines");

        let snapshots = collect_history(repo.path(), "HEAD", true).unwrap();

        assert_eq!(snapshots[0].lines, 4);
        assert_eq!(snapshots[0].non_blank_lines, 2);
    }

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
    fn language_history_honors_historical_attribute_overrides() {
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
        assert_eq!(snapshots[0].sequence, 1);
        assert_eq!(snapshots[0].commit, first);
        assert_eq!(snapshots[1].lines, 3);
        assert_eq!(snapshots[2].lines, 5);
        assert_eq!(snapshots[3].lines, 2);
    }

    #[test]
    fn counts_first_parent_commits_in_calendar_intervals() {
        let repo = TempRepo::new();
        repo.commit_at("first", "2026-01-01T12:00:00+0000");
        repo.commit_at("second", "2026-01-01T13:00:00+0000");
        repo.commit_at("third", "2026-01-03T12:00:00+0000");

        assert_eq!(
            collect_commit_counts(repo.path(), "HEAD", CommitInterval::Daily).unwrap(),
            vec![("2026-01-01".to_owned(), 2), ("2026-01-03".to_owned(), 1)]
        );
    }

    struct TempRepo {
        path: std::path::PathBuf,
    }

    impl TempRepo {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("repo-lines-test-{}-{id}", std::process::id()));
            fs::create_dir(&path).unwrap();
            let repo = Self { path };
            repo.run(&["init", "-b", "master"]);
            repo.run(&["config", "user.name", "Repo Lines Test"]);
            repo.run(&["config", "user.email", "repo-lines@example.invalid"]);
            repo
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write(&self, name: &str, contents: &[u8]) {
            fs::write(self.path.join(name), contents).unwrap();
        }

        fn write_git_info_attributes(&self, contents: &[u8]) {
            fs::write(self.path.join(".git/info/attributes"), contents).unwrap();
        }

        fn commit(&self, message: &str) {
            self.run(&["add", "."]);
            self.run(&["commit", "-m", message]);
        }

        fn commit_at(&self, message: &str, datetime: &str) {
            let output = Command::new("git")
                .args(["commit", "--allow-empty", "-m", message])
                .current_dir(&self.path)
                .env("GIT_AUTHOR_DATE", datetime)
                .env("GIT_COMMITTER_DATE", datetime)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn head(&self) -> String {
            String::from_utf8(self.output(&["rev-parse", "HEAD"]).stdout)
                .unwrap()
                .trim()
                .to_owned()
        }

        fn run(&self, args: &[&str]) {
            let output = self.output(args);
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn output(&self, args: &[&str]) -> std::process::Output {
            Command::new("git")
                .args(args)
                .current_dir(&self.path)
                .output()
                .unwrap()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
