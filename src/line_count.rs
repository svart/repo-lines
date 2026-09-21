/// Physical line counts for a single blob or for a whole snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineCount {
    pub(crate) all: u64,
    pub(crate) non_blank: u64,
}

/// Counts physical lines in `contents`, treating blobs that look binary as
/// empty. `count_non_blank` is opt-in because the extra per-byte work is only
/// needed for the `--non-blank` overlay.
pub(crate) fn text_line_counts(contents: &[u8], count_non_blank: bool) -> LineCount {
    const BINARY_SNIFF_BYTES: usize = 8_000;
    if contents.is_empty() {
        return LineCount::default();
    }
    if contents[..contents.len().min(BINARY_SNIFF_BYTES)].contains(&0) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
