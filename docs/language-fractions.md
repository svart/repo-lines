# Language-fraction chart

## Objective

Provide a `repo-lines --languages` view that shows how the fraction of tracked
text lines belonging to each detected language changes along the selected
revision's first-parent history.

## Semantics

- Each row is normalized to 100%; absolute repository size does not affect its
  width.
- Language detection uses case-insensitive filenames and extensions.
- Physical lines are counted, including blank and comment lines, consistently
  with the default chart.
- Unknown text files are grouped under `Other`, while binary files remain
  excluded.
- Language order and visual identity remain fixed for the complete chart.
- Terminal output uses ANSI colors. Redirected output uses letters and a legend.

## Option compatibility

`--languages` composes with `--path`, `--rev`, and `--date`. It cannot be
combined with `--commits` or `--non-blank`.
