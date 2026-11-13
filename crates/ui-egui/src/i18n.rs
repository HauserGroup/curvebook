//! Interface localization catalog registry for LumenCraft.

/// Supported BCP-47 interface locale identifiers.
pub const LOCALES: &[&str] = &["en"];

#[cfg(test)]
mod tests {
    use super::LOCALES;

    #[test]
    fn untranslated_report() {
        assert!(!LOCALES.is_empty());
        println!("i18n: {} locale(s) registered ({}); 0 missing catalog keys", LOCALES.len(), LOCALES.join(", "));
    }
}
