//! Failure report types and secret redaction.
//!
//! A [`FailureReport`] is what gets sent to the operator when a run fails.
//! Everything here is pure: no I/O.

const REDACTED: &str = "[REDACTED]";

/// Describes a failed (or empty) run for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureReport {
    /// Human-readable reason. Must already be redacted by the caller.
    pub message: String,
    /// Language filter of the run; `None` means all languages.
    pub language: Option<String>,
    /// Cloud Run execution name, used to find the logs.
    pub execution: Option<String>,
}

/// Replaces every occurrence of each non-blank secret in `text` with `[REDACTED]`.
///
/// Blank secrets (empty or whitespace only, e.g. an unset token) are ignored.
/// Longer secrets are replaced first so that a secret containing another
/// secret is fully hidden.
pub fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut targets: Vec<&str> = secrets
        .iter()
        .copied()
        .filter(|s| !s.trim().is_empty())
        .collect();
    targets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    targets.into_iter().fold(text.to_string(), |acc, secret| {
        acc.replace(secret, REDACTED)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn t43_redact_secrets_hides_secret() {
        // Given: reqwest-style error text containing the webhook URL
        let text = "error sending request for url (https://discord.com/api/webhooks/1/tok)";
        let secrets = ["https://discord.com/api/webhooks/1/tok"];

        // When
        let result = redact_secrets(text, &secrets);

        // Then
        assert_eq!(result, "error sending request for url ([REDACTED])");
    }

    #[test]
    fn t44_redact_secrets_hides_multiple_secrets_and_occurrences() {
        // Given
        let text = "k=AIza1 t=ghp_2 k=AIza1";
        let secrets = ["AIza1", "ghp_2"];

        // When
        let result = redact_secrets(text, &secrets);

        // Then
        assert_eq!(result, "k=[REDACTED] t=[REDACTED] k=[REDACTED]");
    }

    #[test]
    fn t45_redact_secrets_ignores_blank_secrets() {
        // Given: GITHUB_TOKEN unset -> blank secrets
        let secrets = ["", "   "];

        // When
        let result = redact_secrets("abc", &secrets);

        // Then
        assert_eq!(result, "abc");
    }

    #[rstest]
    #[case::absent("nothing here", "SECRET")]
    #[case::once("a SECRET b", "SECRET")]
    #[case::many("SECRET SECRET SECRET", "SECRET")]
    #[case::prefix("SECRETtail", "SECRET")]
    #[case::suffix("headSECRET", "SECRET")]
    #[case::whole("SECRET", "SECRET")]
    #[case::multibyte("鍵=秘密の値!", "秘密の値")]
    fn p8_redact_secrets_never_leaves_the_secret(#[case] text: &str, #[case] secret: &str) {
        // Given/When
        let result = redact_secrets(text, &[secret]);

        // Then
        assert!(!result.contains(secret), "leaked: {result}");
    }

    #[test]
    fn p8_redact_secrets_hides_secret_that_contains_another_secret() {
        // Given: one secret is a substring of the other
        let result = redact_secrets("x=abcdef", &["abc", "abcdef"]);

        // Then
        assert!(!result.contains("abc"), "leaked: {result}");
    }
}
