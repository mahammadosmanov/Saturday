use unicode_normalization::UnicodeNormalization;

pub fn normalize_nfc(text: &str) -> String {
    text.nfc().collect()
}

pub fn normalize_for_matching(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.nfc() {
        if character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        match character {
            'I' => normalized.push('ı'),
            'İ' => normalized.push('i'),
            '\u{2018}' | '\u{2019}' | '\u{02bc}' | '\u{ff07}' => normalized.push('\''),
            character => normalized.extend(character.to_lowercase()),
        }
    }
    if matches!(normalized.as_bytes().last(), Some(b'.' | b'?' | b'!')) {
        normalized.pop();
        while normalized.ends_with(' ') {
            normalized.pop();
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::{normalize_for_matching, normalize_nfc};

    #[test]
    fn normalizes_equivalent_turkish_unicode_to_nfc() {
        assert_eq!(normalize_nfc("I\u{307}stanbul"), "İstanbul");
        assert_eq!(normalize_nfc("cafe\u{301}"), "café");
    }

    #[test]
    fn normalizes_turkish_case_without_ascii_case_loss() {
        assert_eq!(normalize_for_matching("İSTANBUL"), "istanbul");
        assert_eq!(normalize_for_matching("ISPARTA"), "ısparta");
        assert_eq!(normalize_for_matching("IĞDIR"), "ığdır");
    }

    #[test]
    fn normalizes_apostrophes_and_whitespace_for_matching_only() {
        assert_eq!(
            normalize_for_matching("  LÜTFEN\tsohbet’i   aç!  "),
            "lütfen sohbet'i aç"
        );
        assert_eq!(
            normalize_for_matching("lütfen  ana   ekrana dön?"),
            "lütfen ana ekrana dön"
        );
        assert_eq!(normalize_nfc("sohbet’i"), "sohbet’i");
    }

    #[test]
    fn removes_only_one_trailing_terminal_mark() {
        assert_eq!(normalize_for_matching("status?!"), "status?");
    }
}
