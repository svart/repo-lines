use std::env;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Output};

const BAR_WIDTH: usize = 50;
const FRACTIONAL_BLOCKS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    sequence: usize,
    commit: String,
    lines: u64,
}

impl Snapshot {
    fn new(sequence: usize, commit: &str, lines: u64) -> Self {
        Self {
            sequence,
            commit: commit.to_owned(),
            lines,
        }
    }

    fn label(&self) -> String {
        let short_hash = self.commit.get(..8).unwrap_or(&self.commit);
        format!("{:06}:{short_hash}", self.sequence)
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

fn collect_history(repo: &Path, revision: &str) -> Result<Vec<Snapshot>, String> {
    let rev_list_args = ["rev-list", "--reverse", "--first-parent", revision];
    let rev_list = git(repo, &rev_list_args)?;
    if !rev_list.status.success() {
        return Err(git_failure(&rev_list_args, &rev_list));
    }
    let commits = std::str::from_utf8(&rev_list.stdout)
        .map_err(|error| format!("git rev-list returned non-UTF-8 output: {error}"))?;

    commits
        .lines()
        .enumerate()
        .map(|(index, commit)| {
            let grep_args = ["grep", "-I", "-c", "^", commit, "--"];
            let grep = git(repo, &grep_args)?;
            // Like grep(1), git grep exits 1 when there are no matches.
            if !grep.status.success() && grep.status.code() != Some(1) {
                return Err(git_failure(&grep_args, &grep));
            }
            let lines = sum_grep_counts(&grep.stdout)?;
            Ok(Snapshot::new(index + 1, commit, lines))
        })
        .collect()
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

fn render_chart(history: &[Snapshot], width: usize) -> String {
    let maximum = history
        .iter()
        .map(|snapshot| snapshot.lines)
        .max()
        .unwrap_or(0);
    let label_width = history
        .iter()
        .map(|snapshot| snapshot.label().len())
        .max()
        .unwrap_or(15);
    let mut chart = String::from("        0 LoC\n");

    for snapshot in history {
        let label = snapshot.label();
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

fn usage() -> &'static str {
    "Usage: repo-lines [REVISION]\n\nPlot the line count along a Git revision's first-parent history.\n\nArguments:\n  REVISION  revision to inspect [default: HEAD]\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n"
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let revision = match args.next().as_deref() {
        None => "HEAD".to_owned(),
        Some("-h" | "--help") => {
            print!("{}", usage());
            return Ok(());
        }
        Some("-V" | "--version") => {
            println!("repo-lines {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(revision) => revision.to_owned(),
    };
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument: {extra}\n\n{}", usage()));
    }

    let history = collect_history(Path::new("."), &revision)?;
    print!("{}", render_chart(&history, BAR_WIDTH));
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
    fn parses_git_grep_counts_from_the_last_colon() {
        let output = b"src/main.rs:12\nnotes:with:colons.txt:7\n";

        assert_eq!(sum_grep_counts(output).unwrap(), 19);
    }

    #[test]
    fn rejects_malformed_git_grep_output() {
        let error = sum_grep_counts(b"src/main.rs:not-a-number\n").unwrap_err();

        assert!(error.contains("not-a-number"));
    }

    #[test]
    fn renders_scaled_bars_in_history_order() {
        let history = vec![
            Snapshot::new(1, "0123456789abcdef", 5),
            Snapshot::new(2, "fedcba9876543210", 10),
            Snapshot::new(3, "aabbccddeeff0011", 0),
        ];

        assert_eq!(
            render_chart(&history, 10),
            "        0 LoC\n\
             000001:01234567  █████ 5\n\
             000002:fedcba98  ██████████ 10\n\
             000003:aabbccdd  0\n"
        );
    }

    #[test]
    fn collects_oldest_first_snapshots_from_first_parent_only() {
        let repo = TempRepo::new();
        repo.write("tracked.txt", b"one\n");
        repo.commit("initial");
        let first = repo.head();

        repo.run(&["checkout", "-b", "side"]);
        repo.write("side.txt", b"side\nbranch\n");
        repo.commit("side");
        repo.run(&["checkout", "master"]);
        repo.write("tracked.txt", b"one\ntwo\nthree\n");
        repo.commit("main");
        repo.run(&["merge", "--no-ff", "side", "-m", "merge side"]);

        let snapshots = collect_history(repo.path(), "HEAD").unwrap();

        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0], Snapshot::new(1, &first, 1));
        assert_eq!(snapshots[1].lines, 3);
        assert_eq!(snapshots[2].lines, 5);
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
