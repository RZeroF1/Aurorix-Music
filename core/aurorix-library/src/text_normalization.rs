//! Deterministic text preparation for local catalog search.
//!
//! The search index stores normalized text and separately stores the tokens
//! emitted by [`search_tokens`]. This module intentionally has no database,
//! filesystem, Provider, or platform dependency so every host uses the same
//! query representation.

/// A normalized search string.
///
/// Normalization applies Unicode lowercase mapping, compatibility folds for
/// the full-width ASCII range, common decomposed-accent composition, and
/// Unicode-aware whitespace collapsing. It preserves non-ASCII letters and
/// does not remove diacritics or punctuation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedSearchText(String);

impl NormalizedSearchText {
    /// Normalizes a search field or query.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self(normalize_search_text(value))
    }

    /// Returns the normalized search text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this value and returns its normalized string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl AsRef<str> for NormalizedSearchText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Normalizes search text deterministically across platforms.
///
/// Unicode lowercase expansion is delegated to [`char::to_lowercase`]. ASCII
/// full-width characters (`U+FF01..=U+FF5E`) are folded to their ASCII forms,
/// common Latin base-plus-mark pairs are composed, and every Unicode
/// whitespace run becomes one ASCII space. Leading and trailing whitespace is
/// removed. No locale-specific or lossy transliteration is performed.
#[must_use]
pub fn normalize_search_text(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut pending_space = false;

    for character in value.chars() {
        if character.is_whitespace() {
            if !normalized.is_empty() {
                pending_space = true;
            }
            continue;
        }

        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }

        let folded = fold_full_width(character);
        for lower in folded.to_lowercase() {
            if let Some(composed) = compose_common_mark(normalized.chars().last(), lower) {
                normalized.pop();
                normalized.push(composed);
            } else {
                normalized.push(lower);
            }
        }
    }

    normalized
}

fn compose_common_mark(previous: Option<char>, mark: char) -> Option<char> {
    let previous = previous?;
    let composed = match (previous, mark) {
        ('a', '\u{0300}') => '\u{00E0}',
        ('a', '\u{0301}') => '\u{00E1}',
        ('a', '\u{0302}') => '\u{00E2}',
        ('a', '\u{0303}') => '\u{00E3}',
        ('a', '\u{0308}') => '\u{00E4}',
        ('a', '\u{030A}') => '\u{00E5}',
        ('a', '\u{0304}') => '\u{0101}',
        ('a', '\u{0306}') => '\u{0103}',
        ('a', '\u{030C}') => '\u{01CE}',
        ('a', '\u{0328}') => '\u{0105}',
        ('c', '\u{0301}') => '\u{0107}',
        ('c', '\u{0302}') => '\u{0109}',
        ('c', '\u{030C}') => '\u{010D}',
        ('c', '\u{0327}') => '\u{00E7}',
        ('d', '\u{030C}') => '\u{010F}',
        ('e', '\u{0300}') => '\u{00E8}',
        ('e', '\u{0301}') => '\u{00E9}',
        ('e', '\u{0302}') => '\u{00EA}',
        ('e', '\u{0308}') => '\u{00EB}',
        ('e', '\u{0304}') => '\u{0113}',
        ('e', '\u{0306}') => '\u{0115}',
        ('e', '\u{030C}') => '\u{011B}',
        ('e', '\u{0328}') => '\u{0119}',
        ('g', '\u{0302}') => '\u{011D}',
        ('g', '\u{030C}') => '\u{01F5}',
        ('h', '\u{0302}') => '\u{0125}',
        ('i', '\u{0300}') => '\u{00EC}',
        ('i', '\u{0301}') => '\u{00ED}',
        ('i', '\u{0302}') => '\u{00EE}',
        ('i', '\u{0308}') => '\u{00EF}',
        ('i', '\u{0304}') => '\u{012B}',
        ('i', '\u{0306}') => '\u{012D}',
        ('i', '\u{0328}') => '\u{012F}',
        ('j', '\u{0302}') => '\u{0135}',
        ('k', '\u{030C}') => '\u{01E9}',
        ('l', '\u{0301}') => '\u{013A}',
        ('l', '\u{030C}') => '\u{013E}',
        ('n', '\u{0301}') => '\u{0144}',
        ('n', '\u{0303}') => '\u{00F1}',
        ('n', '\u{030C}') => '\u{0148}',
        ('o', '\u{0300}') => '\u{00F2}',
        ('o', '\u{0301}') => '\u{00F3}',
        ('o', '\u{0302}') => '\u{00F4}',
        ('o', '\u{0303}') => '\u{00F5}',
        ('o', '\u{0308}') => '\u{00F6}',
        ('o', '\u{0304}') => '\u{014D}',
        ('o', '\u{0306}') => '\u{014F}',
        ('o', '\u{030C}') => '\u{01D2}',
        ('o', '\u{0328}') => '\u{01EB}',
        ('r', '\u{0301}') => '\u{0155}',
        ('r', '\u{030C}') => '\u{0159}',
        ('s', '\u{0301}') => '\u{015B}',
        ('s', '\u{030C}') => '\u{0161}',
        ('t', '\u{030C}') => '\u{0165}',
        ('u', '\u{0300}') => '\u{00F9}',
        ('u', '\u{0301}') => '\u{00FA}',
        ('u', '\u{0302}') => '\u{00FB}',
        ('u', '\u{0308}') => '\u{00FC}',
        ('u', '\u{0304}') => '\u{016B}',
        ('u', '\u{0306}') => '\u{016D}',
        ('u', '\u{030A}') => '\u{016F}',
        ('u', '\u{030C}') => '\u{01D4}',
        ('u', '\u{0328}') => '\u{0173}',
        ('w', '\u{0302}') => '\u{0175}',
        ('y', '\u{0301}') => '\u{00FD}',
        ('y', '\u{0302}') => '\u{0177}',
        ('y', '\u{0308}') => '\u{00FF}',
        ('z', '\u{0301}') => '\u{017A}',
        ('z', '\u{030C}') => '\u{017E}',
        _ => return None,
    };
    Some(composed)
}

/// Produces the deterministic search tokens for normalized text.
///
/// Runs of Latin, Cyrillic, Greek, numeric, or punctuation characters are
/// retained as whitespace-delimited tokens. Runs of CJK ideographs are
/// additionally represented by adjacent two-character tokens. A one-character
/// CJK run remains a one-character token. Token order and duplicates are
/// preserved; de-duplication and ranking belong to the query implementation.
#[must_use]
pub fn search_tokens(value: &str) -> Vec<String> {
    let normalized = normalize_search_text(value);
    tokenize_normalized(&normalized)
}

/// Alias for [`search_tokens`] using the operation-oriented name used by
/// indexing adapters.
#[must_use]
pub fn tokenize_search_text(value: &str) -> Vec<String> {
    search_tokens(value)
}

fn tokenize_normalized(value: &str) -> Vec<String> {
    let characters: Vec<char> = value.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < characters.len() {
        if characters[index].is_whitespace() {
            index += 1;
            continue;
        }

        let is_cjk = is_cjk_character(characters[index]);
        let start = index;
        index += 1;
        while index < characters.len()
            && !characters[index].is_whitespace()
            && is_cjk_character(characters[index]) == is_cjk
        {
            index += 1;
        }

        let run: String = characters[start..index].iter().collect();
        if is_cjk {
            let run_chars: Vec<char> = run.chars().collect();
            if run_chars.len() < 2 {
                tokens.push(run);
            } else {
                for pair in run_chars.windows(2) {
                    tokens.push(pair.iter().collect());
                }
            }
        } else {
            tokens.push(run);
        }
    }

    tokens
}

fn fold_full_width(character: char) -> char {
    match character {
        '\u{FF01}'..='\u{FF5E}' => {
            char::from_u32(character as u32 - 0xFEE0).expect("ASCII fold is valid")
        }
        '\u{3000}' => ' ',
        _ => character,
    }
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2FA1F
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

#[cfg(test)]
mod tests {
    use super::{NormalizedSearchText, normalize_search_text, search_tokens};

    #[test]
    fn normalization_folds_case_width_and_unicode_whitespace() {
        assert_eq!(
            normalize_search_text("  ＢＯＡＲＤＳ　of\u{00A0}CANADA  "),
            "boards of canada"
        );
    }

    #[test]
    fn normalization_preserves_diacritics_and_non_latin_text() {
        assert_eq!(
            normalize_search_text("Beyoncé — Déjà Vu / КИНО"),
            "beyoncé — déjà vu / кино"
        );
    }

    #[test]
    fn normalization_composes_common_decomposed_latin_accents() {
        assert_eq!(
            normalize_search_text("Cafe\u{0301} NOE\u{0308}L"),
            "café noël"
        );
    }

    #[test]
    fn normalized_value_has_stable_value_semantics() {
        let left = NormalizedSearchText::new("  Ａ  ");
        let right = NormalizedSearchText::new("a");

        assert_eq!(left, right);
        assert_eq!(left.as_str(), "a");
        assert_eq!(left.into_string(), "a");
    }

    #[test]
    fn tokens_keep_words_and_emit_cjk_bigrams() {
        assert_eq!(
            search_tokens("  Radiohead 東京事変 2026  "),
            [
                "radiohead".to_owned(),
                "東京".to_owned(),
                "京事".to_owned(),
                "事変".to_owned(),
                "2026".to_owned(),
            ]
        );
    }

    #[test]
    fn cjk_runs_are_split_at_whitespace_and_mixed_scripts() {
        assert_eq!(
            search_tokens("東京 rock 京都"),
            ["東京".to_owned(), "rock".to_owned(), "京都".to_owned()]
        );
        assert_eq!(search_tokens("京"), ["京"]);
    }

    #[test]
    fn punctuation_stays_in_non_cjk_tokens_and_empty_input_is_empty() {
        assert_eq!(search_tokens("Boards-of-Canada"), ["boards-of-canada"]);
        assert!(search_tokens(" \t\n ").is_empty());
    }
}
