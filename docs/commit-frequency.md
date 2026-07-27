# Commit-frequency chart

## Objective

Provide a `repo-lines --commits <daily|weekly|monthly|yearly>` view that shows
how many commits occurred in each calendar interval of the selected revision's
first-parent history.

## Success criteria

- The selected interval controls calendar grouping: day, ISO week, month, or year.
- Intervals without commits between the first and final commits are shown with a zero count.
- The output uses the existing scaled terminal bars and labels every interval.
- `--commits` composes with `--path`, `--rev`, and `--full-width`; `--date` and
  `--non-blank` are rejected in this mode.

## Boundaries

- Always: use Git's first-parent history and run the Rust test suite.
- Ask first: add external dependencies or change the default line-history chart.
- Never: count commits reachable only through a merged side branch.
