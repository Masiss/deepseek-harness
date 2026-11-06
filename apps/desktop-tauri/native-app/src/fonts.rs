//! Whether displayed text needs a CJK fallback face.
//!
//! The gpui shell resolves fonts through its theme; this module only answers the
//! script question the shell asks before it lets the renderer pick fallbacks.

/// Whether text needs a CJK face to render.
///
/// Covers CJK ideographs and compatibility blocks, kana, hangul, and fullwidth
/// forms; Vietnamese and other Latin diacritics are already covered.
///
/// @param text - Text about to be displayed.
/// @returns True when a CJK fallback is required.
pub fn contains_cjk(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character as u32,
            0x2E80..=0x2EFF
                | 0x3040..=0x30FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xAC00..=0xD7AF
                | 0xF900..=0xFAFF
                | 0xFF00..=0xFFEF
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cjk_but_not_latin_diacritics() {
        assert!(contains_cjk("中文測試 日本語 한국어"));
        assert!(!contains_cjk("việt nam · tiếng Việt"));
        assert!(!contains_cjk("plain ascii"));
    }
}
