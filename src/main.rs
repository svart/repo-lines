use std::env;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, BufWriter, Read, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitCode, Output, Stdio};
use std::{
    collections::{HashMap, HashSet},
    thread,
};

const BAR_WIDTH: usize = 50;
const FRACTIONAL_BLOCKS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    sequence: usize,
    commit: String,
    datetime: String,
    lines: u64,
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
    fn new(sequence: usize, commit: &str, datetime: &str, lines: u64) -> Self {
        Self {
            sequence,
            commit: commit.to_owned(),
            datetime: datetime.to_owned(),
            lines,
        }
    }

    fn label(&self, date: bool, sequence_width: usize) -> String {
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

fn git(repo: &Path, args: &[&str]) -> Result<Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|error| format!("could not run git: {error}"))
}

fn git_failure(args: &[&str], output: &Output) -> String {
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
) -> Result<Vec<Snapshot>, String> {
    commits
        .iter()
        .enumerate()
        .map(|(index, (commit, datetime))| {
            let args = ["grep", "-I", "-c", "^", commit.as_str(), "--"];
            let output = git(repo, &args)?;
            if !output.status.success() && output.status.code() != Some(1) {
                return Err(git_failure(&args, &output));
            }
            Ok(Snapshot::new(
                index + 1,
                commit,
                datetime,
                sum_grep_counts(&output.stdout)?,
            ))
        })
        .collect()
}

fn text_line_count(contents: &[u8]) -> u64 {
    const BINARY_SNIFF_BYTES: usize = 8_000;
    if contents[..contents.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
        return 0;
    }
    if contents.is_empty() {
        return 0;
    }
    contents.iter().filter(|byte| **byte == b'\n').count() as u64
        + u64::from(contents.last() != Some(&b'\n'))
}

struct BlobReader {
    child: Option<Child>,
    input: Option<BufWriter<ChildStdin>>,
    output: BufReader<ChildStdout>,
    cache: HashMap<String, u64>,
}

impl BlobReader {
    fn open(repo: &Path) -> Result<Self, String> {
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
        })
    }

    fn line_count(&mut self, oid: &str) -> Result<u64, String> {
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

        let lines = text_line_count(&contents);
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

fn collect_history(repo: &Path, revision: &str) -> Result<Vec<Snapshot>, String> {
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
        return collect_history_with_grep(repo, &commits);
    }
    let mut blobs = BlobReader::open(repo)?;
    let mut total = 0_u64;
    let mut history = Vec::with_capacity(commits.len());

    for (index, ((commit, datetime), changes)) in commits.iter().zip(changes).enumerate() {
        for change in changes {
            if mode_has_blob(&change.old_mode) {
                let removed = blobs.line_count(&change.old_oid)?;
                total = total.checked_sub(removed).ok_or_else(|| {
                    format!("line count underflow while removing {}", change.old_oid)
                })?;
            }
            if mode_has_blob(&change.new_mode) {
                let added = blobs.line_count(&change.new_oid)?;
                total = total.checked_add(added).ok_or_else(|| {
                    format!("line count overflow while adding {}", change.new_oid)
                })?;
            }
        }
        history.push(Snapshot::new(index + 1, commit, datetime, total));
    }
    blobs.finish()?;
    Ok(history)
}

fn render_bar(value: u64, maximum: u64, width: usize) -> String {
    if value == 0 || maximum == 0 || width == 0 {
        return String::new();
    }

    let eighths = (u128::from(value) * width as u128 * 8) / u128::from(maximum);
    let full_blocks = (eighths / 8) as usize;
    let fraction = (eighths % 8) as usize;
    let mut bar = "█".repeat(full_blocks);
    if fraction != 0 {
        bar.push(FRACTIONAL_BLOCKS[fraction]);
    }
    bar
}

fn render_chart(history: &[Snapshot], width: usize, date: bool) -> String {
    let maximum = history
        .iter()
        .map(|snapshot| snapshot.lines)
        .max()
        .unwrap_or(0);
    let sequence_width = history.len().max(1).to_string().len();
    let label_width = history
        .iter()
        .map(|snapshot| snapshot.label(date, sequence_width).len())
        .max()
        .unwrap_or(15);
    let mut chart = String::from("        0 LoC\n");

    for snapshot in history {
        let label = snapshot.label(date, sequence_width);
        let bar = render_bar(snapshot.lines, maximum, width);
        let _ = writeln!(
            chart,
            "{label:>label_width$}  {bar}{}{}",
            if bar.is_empty() { "" } else { " " },
            snapshot.lines
        );
    }
    chart
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    date: bool,
    revision: String,
    path: String,
}

fn parse_options(arguments: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut arguments = arguments.into_iter();
    let mut options = Options {
        date: false,
        revision: "HEAD".to_owned(),
        path: ".".to_owned(),
    };

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--date" => options.date = true,
            "--rev" => {
                options.revision = arguments
                    .next()
                    .ok_or_else(|| "missing value for --rev".to_owned())?;
            }
            "--path" => {
                options.path = arguments
                    .next()
                    .ok_or_else(|| "missing value for --path".to_owned())?;
            }
            _ => return Err(format!("unexpected argument: {argument}")),
        }
    }

    Ok(options)
}

fn usage() -> &'static str {
    "Usage: repo-lines [OPTIONS]\n\nPlot the line count along a Git revision's first-parent history.\n\nOptions:\n  --rev <REVISION>  Revision to inspect [default: HEAD]\n  --path <PATH>     Repository path [default: .]\n  --date            Print commit date and time\n  -h, --help        Print help\n  -V, --version     Print version\n"
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

    let history = collect_history(Path::new(&options.path), &options.revision)?;
    print!("{}", render_chart(&history, BAR_WIDTH, options.date));
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
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn parses_default_options() {
        assert_eq!(
            parse_options(Vec::<String>::new()).unwrap(),
            Options {
                date: false,
                revision: "HEAD".to_owned(),
                path: ".".to_owned(),
            }
        );
    }

    #[test]
    fn parses_revision_path_and_date_options() {
        let arguments = ["--path", "/tmp/project", "--date", "--rev", "main"].map(str::to_owned);

        assert_eq!(
            parse_options(arguments).unwrap(),
            Options {
                date: true,
                revision: "main".to_owned(),
                path: "/tmp/project".to_owned(),
            }
        );
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
    fn counts_text_lines_and_ignores_binary_blobs() {
        assert_eq!(text_line_count(b""), 0);
        assert_eq!(text_line_count(b"one\ntwo\n"), 2);
        assert_eq!(text_line_count(b"one\ntwo"), 2);
        assert_eq!(text_line_count(b"binary\0data\n"), 0);
    }

    #[test]
    fn honors_historical_gitattributes_binary_overrides() {
        let repo = TempRepo::new();
        repo.write(
            ".gitattributes",
            b"*.forced-text diff\n*.forced-binary -diff\n",
        );
        repo.write("data.forced-text", b"one\0two\n");
        repo.write("data.forced-binary", b"one\ntwo\n");
        repo.commit("add attribute overrides");

        let snapshots = collect_history(repo.path(), "HEAD").unwrap();

        assert_eq!(snapshots[0].lines, 3);
    }

    #[test]
    fn honors_repository_local_attribute_overrides() {
        let repo = TempRepo::new();
        repo.write_git_info_attributes(b"*.forced-text diff\n*.forced-binary -diff\n");
        repo.write("data.forced-text", b"one\0two\n");
        repo.write("data.forced-binary", b"one\ntwo\n");
        repo.commit("add files with local attribute overrides");

        let snapshots = collect_history(repo.path(), "HEAD").unwrap();

        assert_eq!(snapshots[0].lines, 1);
    }

    #[test]
    fn renders_scaled_bars_in_history_order() {
        let history = vec![
            Snapshot::new(1, "0123456789abcdef", "2026-07-15 10:00:00", 5),
            Snapshot::new(2, "fedcba9876543210", "2026-07-15 11:00:00", 10),
            Snapshot::new(3, "aabbccddeeff0011", "2026-07-15 12:00:00", 0),
        ];

        assert_eq!(
            render_chart(&history, 10, false),
            "        0 LoC\n\
             1:01234567  █████ 5\n\
             2:fedcba98  ██████████ 10\n\
             3:aabbccdd  0\n"
        );

        assert_eq!(
            render_chart(&history, 10, true),
            "        0 LoC\n\
             2026-07-15 10:00:00:1:01234567  █████ 5\n\
             2026-07-15 11:00:00:2:fedcba98  ██████████ 10\n\
             2026-07-15 12:00:00:3:aabbccdd  0\n"
        );
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

        let snapshots = collect_history(repo.path(), "HEAD").unwrap();

        assert_eq!(snapshots.len(), 4);
        assert_eq!(snapshots[0].sequence, 1);
        assert_eq!(snapshots[0].commit, first);
        assert_eq!(snapshots[1].lines, 3);
        assert_eq!(snapshots[2].lines, 5);
        assert_eq!(snapshots[3].lines, 2);
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
