# Line-history chart

## Objective

Provide the default `repo-lines` view: how the number of tracked text lines
changes across the selected revision's first-parent history, with an optional
`--non-blank` overlay.

## Counting strategy

The chart is built incrementally. `repo-lines` walks the history oldest to
newest and adjusts a running total using each commit's changed blobs, rather
than recounting the whole tree at every commit. Blob line counts are cached by
object id, so a blob that survives many commits is read once.

This is only sound while a blob's text/binary classification is a property of
the blob itself. Git attributes can override that classification per path, and
the override can change over time, so the incremental path is abandoned when
either holds:

- a `.gitattributes` file appears anywhere in the diffed history, or
- `git check-attr` reports a non-default `diff` attribute for any touched path,
  which covers `.git/info/attributes` and the user's global attributes file.

In those repositories every commit is counted separately with `git grep -I -c`,
which applies the attribute rules in effect at that commit. The result is
slower but historically accurate.

## Semantics

- Physical lines are counted, including blank and comment lines. A file whose
  final line has no newline still counts that line.
- A blob containing a NUL byte in its first 8000 bytes is treated as binary and
  contributes zero lines.
- `--non-blank` counts a line as non-blank when it holds any non-whitespace
  byte. The extra pass runs only when the option is given.
- Only the first-parent path is plotted; commits reachable exclusively through a
  merge's other parents do not appear.

## Option compatibility

The line chart composes with `--path`, `--rev`, `--date`, `--non-blank`, and
`--full-width`. It is the default when neither `--languages` nor `--commits` is
given.
