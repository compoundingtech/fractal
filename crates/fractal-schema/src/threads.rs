//! Metadata reserved for participant-authorized thread reads.

pub fn is_reserved_subject(subject: &str) -> bool {
    subject.starts_with("thread/")
}

pub fn is_reserved_kind(kind: &str) -> bool {
    kind.starts_with("thread.")
}

pub fn is_reserved_record(subject: &str, kind: &str) -> bool {
    is_reserved_subject(subject) || is_reserved_kind(kind)
}
