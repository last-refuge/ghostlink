//! Prefer explicitly UTF-8 text, while accepting standard plain-text offers.
pub(crate) fn preferred(offered: &[String]) -> Option<&'static str> {
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"]
        .into_iter()
        .find(|candidate| offered.iter().any(|mime| mime == *candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_plain_text_only_clipboards() {
        assert_eq!(preferred(&["text/plain".into()]), Some("text/plain"));
    }
    #[test]
    fn prefers_utf8_when_multiple_formats_are_offered() {
        assert_eq!(
            preferred(&[
                "text/plain".into(),
                "text/html".into(),
                "text/plain;charset=utf-8".into()
            ]),
            Some("text/plain;charset=utf-8")
        );
        assert_eq!(preferred(&["UTF8_STRING".into()]), Some("UTF8_STRING"));
    }
    #[test]
    fn ignores_non_text_and_missing_offers() {
        assert_eq!(preferred(&["image/png".into(), "text/html".into()]), None);
        assert_eq!(preferred(&[]), None);
    }
}
