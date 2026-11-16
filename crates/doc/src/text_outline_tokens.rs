//! Bounded interning of canonical short reference *strings*, never resolved file outlines.
//! Meaning stays in the owning file's pool: sharing the immutable "@0" token across two
//! documents must not share either document's geometry or introduce serialization context.
use std::sync::{Arc, OnceLock};

const TOKENS: usize = 4096;
static REFERENCES: [OnceLock<Arc<str>>; TOKENS] = [const { OnceLock::new() }; TOKENS];

pub(crate) fn get(path: &str) -> Arc<str> {
    if let Some(digits) = path.strip_prefix('@')
        && !digits.is_empty()
        && digits.len() <= 4
        && (digits.len() == 1 || !digits.starts_with('0'))
        && digits.bytes().all(|b| b.is_ascii_digit())
        && let Ok(index) = digits.parse::<usize>()
        && let Some(slot) = REFERENCES.get(index)
    {
        return slot.get_or_init(|| Arc::from(path)).clone();
    }
    Arc::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_references_share_storage_without_rewriting_any_string() {
        for path in ["@0", "@17", "@4095"] {
            assert!(Arc::ptr_eq(&get(path), &get(path)));
        }
        for path in ["", "@", "@00", "@+1", "@-1", "@4096", "@12345678901234567890", "M0 0L10 10Z"] {
            assert_eq!(get(path).as_ref(), path);
            assert!(!Arc::ptr_eq(&get(path), &get(path)));
        }
    }
}
