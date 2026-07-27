# repo-lines

`repo-lines` draws a terminal chart showing how the number of lines in a Git
repository changes over its first-parent commit history.

It counts lines in tracked text files, ignores binary files, and honors Git
attributes that mark files as text or binary.

```text
        0 LoC
1:3e1427b5  █████████████████████▊ 305
2:31676b92  ███████████████████████████████████████████████▏ 659
3:c215ee74  ██████████████████████████████████████████████████ 698
```

## Installation

Install from GitHub with Cargo:

```sh
cargo install --git https://github.com/svart/repo-lines.git
```

Or build from a local checkout:

```sh
git clone https://github.com/svart/repo-lines.git
cd repo-lines
cargo install --path .
```

## Usage

Run `repo-lines` inside a Git repository, or select one with `--path`:

```sh
repo-lines [OPTIONS]
```

The repository path defaults to the current directory and the revision defaults
to `HEAD`. For example:

```sh
repo-lines
repo-lines --rev main
repo-lines --path ../another-repository
repo-lines --path ../another-repository --rev v1.0.0 --date
repo-lines --non-blank
repo-lines --languages
repo-lines --commits weekly
```

Options:

- `--rev <REVISION>` selects the revision to inspect
- `--path <PATH>` selects the Git repository to analyze
- `--date` prints the commit date and time before the commit number
- `--non-blank` overlays non-blank lines in grey on the white total-lines bar
- `--languages` shows the fraction of text lines belonging to each detected
  language as a normalized stacked bar. It can be combined with `--date`, but
  not with `--non-blank` or `--commits`.
- `--commits <daily|weekly|monthly|yearly>` shows first-parent commit counts per
  calendar interval instead of line history. Empty intervals between activity
  are included as zero-height bars; this mode cannot be combined with `--date`
  or `--non-blank`.
- `-h`, `--help` prints help
- `-V`, `--version` prints the version

## How it works

`repo-lines` walks the selected revision's first-parent history from oldest to
newest. It updates the line total from each commit's changed blobs, avoiding a
full recount when possible. Repositories with relevant Git attribute rules are
counted commit by commit so historical text and binary overrides remain
accurate.

Non-blank lines are counted only when `--non-blank` is selected. When output is
a terminal, the chart draws the total in white and overlays the non-blank
portion in grey; redirected output remains uncolored.

Language charts classify tracked text files by filename or extension. They use
the same physical line counts as the default chart, including blank and comment
lines. Unrecognized files are grouped under `Other`, so every non-empty bar
represents 100% of the repository's text lines. Terminal output uses colors;
redirected output uses letters identified by the legend.

Only the first-parent path is plotted, so commits reachable exclusively through
a merge's other parents do not appear as separate points.

## License

Licensed under the [MIT License](LICENSE).
