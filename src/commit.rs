/// A commit on the first-parent path, together with its 1-based position in
/// that history. Every snapshot type embeds one of these so the row label is
/// defined in exactly one place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitRef {
    pub(crate) sequence: usize,
    pub(crate) id: String,
    pub(crate) datetime: String,
}

impl CommitRef {
    pub(crate) fn new(sequence: usize, id: &str, datetime: &str) -> Self {
        Self {
            sequence,
            id: id.to_owned(),
            datetime: datetime.to_owned(),
        }
    }

    pub(crate) fn label(&self, date: bool, sequence_width: usize) -> String {
        let short_hash = self.id.get(..8).unwrap_or(&self.id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_pad_the_sequence_and_shorten_the_hash() {
        let commit = CommitRef::new(7, "0123456789abcdef", "2026-07-15 10:00:00");

        assert_eq!(commit.label(false, 3), "007:01234567");
        assert_eq!(commit.label(true, 1), "2026-07-15 10:00:00:7:01234567");
    }

    #[test]
    fn labels_tolerate_hashes_shorter_than_the_abbreviation() {
        assert_eq!(CommitRef::new(1, "abc", "").label(false, 1), "1:abc");
    }
}
