//! Discord embed types and pure conversion helpers.
//!
//! Everything here is free of I/O: truncation, summary-to-embed conversion,
//! byte-size limiting and `retry_after` parsing.
use std::time::Duration;

use serde::Serialize;

use crate::domain::{FailureReport, TrendingSummary};

pub(crate) const TITLE_LIMIT: usize = 256;
pub(crate) const DESCRIPTION_LIMIT: usize = 4096;
pub(crate) const FIELD_NAME_LIMIT: usize = 256;
pub(crate) const FIELD_VALUE_LIMIT: usize = 1024;
pub(crate) const FOOTER_LIMIT: usize = 2048;
pub(crate) const MAX_EMBED_BYTES: usize = 8000;
pub(crate) const TOTAL_CHARS_PER_MESSAGE: usize = 6000;
pub(crate) const EMBED_COLOR: u32 = 0x24292F;
pub(crate) const FOOTER_TEXT: &str = "GitHub Trending";
pub(crate) const FAILURE_EMBED_COLOR: u32 = 0xD73A49;
pub(crate) const FAILURE_EMBED_TITLE: &str = "GitHub Trending digest failed";
const FAILURE_EMBED_URL: &str = "https://github.com/trending";
const EMPTY_FAILURE_MESSAGE: &str = "(no error message)";
pub(crate) const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(1);
pub(crate) const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

const ELLIPSIS: char = '…';

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DiscordEmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DiscordEmbedFooter {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DiscordEmbed {
    pub title: String,
    pub url: String,
    pub description: String,
    pub color: u32,
    pub fields: Vec<DiscordEmbedField>,
    pub footer: DiscordEmbedFooter,
}

#[derive(Debug, Serialize)]
pub(crate) struct DiscordWebhookPayload {
    pub embeds: Vec<DiscordEmbed>,
}

impl DiscordEmbed {
    /// UTF-8 byte count of the same text parts that `char_count` measures.
    /// Webhooks reject oversized messages (413) by bytes, not characters.
    pub(crate) fn byte_count(&self) -> usize {
        let fields: usize = self
            .fields
            .iter()
            .map(|f| f.name.len() + f.value.len())
            .sum();
        self.title.len() + self.description.len() + fields + self.footer.text.len()
    }

    /// Character count as Discord counts it toward the 6000 limit
    /// (title + description + field names/values + footer text).
    pub(crate) fn char_count(&self) -> usize {
        let fields: usize = self
            .fields
            .iter()
            .map(|f| f.name.chars().count() + f.value.chars().count())
            .sum();
        self.title.chars().count()
            + self.description.chars().count()
            + fields
            + self.footer.text.chars().count()
    }
}

/// Truncates `text` to at most `max_bytes` UTF-8 bytes, ending with `…` when cut.
/// Requires `max_bytes >= 3` (the byte length of `…`).
pub(crate) fn truncate_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let budget = max_bytes.saturating_sub(ELLIPSIS.len_utf8());
    let mut result = String::new();
    for c in text.chars() {
        if result.len() + c.len_utf8() > budget {
            break;
        }
        result.push(c);
    }
    result.push(ELLIPSIS);
    result
}

/// Truncates `text` to at most `max` characters, ending with `…` when cut.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars()
        .take(max.saturating_sub(1))
        .chain(std::iter::once(ELLIPSIS))
        .collect()
}

/// Converts a summary into an embed that respects every Discord limit.
pub(crate) fn build_embed(summary: &TrendingSummary) -> DiscordEmbed {
    let repo = summary.repository();

    let mut fields = Vec::new();
    if let Some(language) = repo.language().filter(|l| !l.trim().is_empty()) {
        fields.push(build_field("Language", language, true));
    }
    fields.push(build_field("Stars", &format!("★{}", repo.stars()), true));
    if let Some(value) = bullet_list(summary.use_cases()) {
        fields.push(build_field("Key Use Cases", &value, false));
    }
    if let Some(value) = bullet_list(summary.target_languages()) {
        fields.push(build_field("Target Languages", &value, false));
    }

    let mut embed = DiscordEmbed {
        title: truncate_chars(&repo.full_name(), TITLE_LIMIT),
        url: repo.url().to_string(),
        description: String::new(),
        color: EMBED_COLOR,
        fields,
        footer: DiscordEmbedFooter {
            text: truncate_chars(FOOTER_TEXT, FOOTER_LIMIT),
        },
    };

    // The description absorbs whatever room is left under the 4096 / 6000-char
    // and 8000-byte budgets.
    let description_max = DESCRIPTION_LIMIT.min(TOTAL_CHARS_PER_MESSAGE - embed.char_count());
    let description_max_bytes = MAX_EMBED_BYTES - embed.byte_count();
    let mut description = summary.ai_summary().to_string();
    if let Some(original) = repo.description().filter(|d| !d.trim().is_empty()) {
        description.push_str(&format!("\n\n> {original}"));
    }
    embed.description = truncate_bytes(
        &truncate_chars(&description, description_max),
        description_max_bytes,
    );
    embed
}

/// Converts a failure report into an embed that respects every Discord limit.
pub(crate) fn build_failure_embed(report: &FailureReport) -> DiscordEmbed {
    let mut fields = vec![build_field(
        "Language",
        report.language.as_deref().unwrap_or("all"),
        true,
    )];
    if let Some(execution) = &report.execution {
        fields.push(build_field("Execution", execution, true));
    }

    let mut embed = DiscordEmbed {
        title: truncate_chars(FAILURE_EMBED_TITLE, TITLE_LIMIT),
        url: FAILURE_EMBED_URL.to_string(),
        description: String::new(),
        color: FAILURE_EMBED_COLOR,
        fields,
        footer: DiscordEmbedFooter {
            text: truncate_chars(FOOTER_TEXT, FOOTER_LIMIT),
        },
    };

    // The description absorbs whatever room is left under the Discord budgets.
    let description_max = DESCRIPTION_LIMIT.min(TOTAL_CHARS_PER_MESSAGE - embed.char_count());
    let description_max_bytes = MAX_EMBED_BYTES - embed.byte_count();
    let message = if report.message.trim().is_empty() {
        EMPTY_FAILURE_MESSAGE
    } else {
        report.message.as_str()
    };
    embed.description = truncate_bytes(
        &truncate_chars(message, description_max),
        description_max_bytes,
    );
    embed
}

fn build_field(name: &str, value: &str, inline: bool) -> DiscordEmbedField {
    DiscordEmbedField {
        name: truncate_chars(name, FIELD_NAME_LIMIT),
        value: truncate_bytes(&truncate_chars(value, FIELD_VALUE_LIMIT), FIELD_VALUE_LIMIT),
        inline,
    }
}

fn bullet_list(items: &[String]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    Some(
        items
            .iter()
            .map(|item| format!("- {item}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Reads `retry_after` (seconds) from a 429 body, clamped to 0..=60 seconds.
pub(crate) fn parse_retry_after(body: &str) -> Duration {
    let seconds = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("retry_after").and_then(serde_json::Value::as_f64))
        .filter(|s| s.is_finite());
    match seconds {
        Some(s) => Duration::from_secs_f64(s.clamp(0.0, MAX_RETRY_AFTER.as_secs_f64())),
        None => DEFAULT_RETRY_AFTER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Repository;
    use rstest::rstest;

    fn make_summary(
        name: &str,
        description: Option<String>,
        language: Option<String>,
        ai_summary: &str,
        use_cases: Vec<String>,
        target_languages: Vec<String>,
    ) -> TrendingSummary {
        let repo = Repository::new(
            name.to_string(),
            "o".to_string(),
            "https://github.com/o/n".to_string(),
            description,
            10,
            language,
            vec![],
        )
        .unwrap();
        TrendingSummary::new(repo, ai_summary.to_string(), use_cases, target_languages).unwrap()
    }

    fn simple_summary() -> TrendingSummary {
        make_summary("n", None, None, "Summary", vec![], vec![])
    }

    fn assert_within_limits(embed: &DiscordEmbed) {
        assert!(embed.title.chars().count() <= TITLE_LIMIT);
        assert!(embed.description.chars().count() <= DESCRIPTION_LIMIT);
        assert!(embed.fields.len() <= 25);
        for f in &embed.fields {
            assert!(f.name.chars().count() <= FIELD_NAME_LIMIT);
            assert!(f.value.chars().count() <= FIELD_VALUE_LIMIT);
        }
        assert!(embed.footer.text.chars().count() <= FOOTER_LIMIT);
        assert!(embed.char_count() <= TOTAL_CHARS_PER_MESSAGE);
        for f in &embed.fields {
            assert!(f.value.len() <= FIELD_VALUE_LIMIT);
        }
        assert!(embed.byte_count() <= MAX_EMBED_BYTES);
    }

    // ---- truncate_chars (T1-T6, P1, P2) ----

    #[test]
    fn t1_truncate_keeps_short_text() {
        // Given / When / Then
        assert_eq!(truncate_chars("abc", 5), "abc");
    }

    #[test]
    fn t2_truncate_keeps_text_at_exact_limit() {
        // Given
        let text = "a".repeat(256);
        // When
        let result = truncate_chars(&text, 256);
        // Then
        assert_eq!(result, text);
    }

    #[test]
    fn t3_truncate_cuts_one_over_limit() {
        // Given
        let text = "a".repeat(257);
        // When
        let result = truncate_chars(&text, 256);
        // Then
        assert_eq!(result, format!("{}…", "a".repeat(255)));
        assert_eq!(result.chars().count(), 256);
    }

    #[test]
    fn t4_truncate_multibyte_over_limit() {
        // Given
        let text = "あ".repeat(4097);
        // When
        let result = truncate_chars(&text, 4096);
        // Then
        assert_eq!(result, format!("{}…", "あ".repeat(4095)));
        assert_eq!(result.chars().count(), 4096);
    }

    #[test]
    fn t5_truncate_keeps_multibyte_text_within_char_limit() {
        // Given: 12288 bytes but only 4096 chars
        let text = "あ".repeat(4096);
        // When
        let result = truncate_chars(&text, 4096);
        // Then
        assert_eq!(result, text);
    }

    #[test]
    fn t6_truncate_four_byte_chars() {
        // Given
        let text = "🦀".repeat(300);
        // When
        let result = truncate_chars(&text, 256);
        // Then
        assert_eq!(result, format!("{}…", "🦀".repeat(255)));
    }

    #[rstest]
    #[case("", 1)]
    #[case("a", 1)]
    #[case("ab", 1)]
    #[case("abc", 3)]
    #[case("abcd", 3)]
    #[case("あいうえお", 4)]
    #[case("a🦀あ🦀a🦀", 5)]
    #[case("a🦀あ🦀a🦀", 6)]
    #[case("a🦀あ🦀a🦀", 2)]
    fn p1_p2_truncate_invariants(#[case] text: &str, #[case] max: usize) {
        // When
        let result = truncate_chars(text, max);
        // Then: P1
        assert!(result.chars().count() <= max);
        // Then: P2
        let len = text.chars().count();
        if len <= max {
            assert_eq!(result, text);
        } else {
            let expected: String = text.chars().take(max - 1).chain(['…']).collect();
            assert_eq!(result, expected);
        }
    }

    // ---- char_count (T7) ----

    #[test]
    fn t7_char_count_sums_text_parts_only() {
        // Given
        let embed = DiscordEmbed {
            title: "abc".to_string(),
            url: "https://github.com/a/b".to_string(),
            description: "あいう".to_string(),
            color: EMBED_COLOR,
            fields: vec![DiscordEmbedField {
                name: "Stars".to_string(),
                value: "★1".to_string(),
                inline: true,
            }],
            footer: DiscordEmbedFooter {
                text: "GitHub Trending".to_string(),
            },
        };
        // When / Then
        assert_eq!(embed.char_count(), 3 + 3 + 5 + 2 + 15);
    }

    // ---- build_embed (T8-T14, P3) ----

    fn field(name: &str, value: &str, inline: bool) -> DiscordEmbedField {
        DiscordEmbedField {
            name: name.to_string(),
            value: value.to_string(),
            inline,
        }
    }

    #[test]
    fn t8_build_embed_full() {
        // Given
        let repo = Repository::new(
            "rust".to_string(),
            "rust-lang".to_string(),
            "https://github.com/rust-lang/rust".to_string(),
            Some("A language empowering everyone".to_string()),
            80000,
            Some("Rust".to_string()),
            vec![],
        )
        .unwrap();
        let summary = TrendingSummary::new(
            repo,
            "A systems programming language.".to_string(),
            vec!["CLI tools".to_string(), "Web servers".to_string()],
            vec!["Rust".to_string()],
        )
        .unwrap();
        // When
        let embed = build_embed(&summary);
        // Then
        assert_eq!(embed.title, "rust-lang/rust");
        assert_eq!(embed.url, "https://github.com/rust-lang/rust");
        assert_eq!(
            embed.description,
            "A systems programming language.\n\n> A language empowering everyone"
        );
        assert_eq!(embed.color, 0x24292F);
        assert_eq!(
            embed.fields,
            vec![
                field("Language", "Rust", true),
                field("Stars", "★80000", true),
                field("Key Use Cases", "- CLI tools\n- Web servers", false),
                field("Target Languages", "- Rust", false),
            ]
        );
        assert_eq!(embed.footer.text, "GitHub Trending");
    }

    #[test]
    fn t9_build_embed_omits_optional_fields() {
        // Given
        let summary = simple_summary();
        // When
        let embed = build_embed(&summary);
        // Then
        assert_eq!(embed.description, "Summary");
        assert_eq!(embed.fields, vec![field("Stars", "★10", true)]);
    }

    #[test]
    fn t10_build_embed_skips_blank_language() {
        // Given
        let summary = make_summary("n", None, Some("  ".to_string()), "Summary", vec![], vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        assert!(embed.fields.iter().all(|f| f.name != "Language"));
    }

    #[test]
    fn t34_build_embed_ignores_blank_repo_description() {
        // Given
        let summary = make_summary("n", Some("  ".to_string()), None, "Summary", vec![], vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        assert_eq!(embed.description, "Summary");
    }

    #[test]
    fn t11_build_embed_truncates_title() {
        // Given
        let summary = make_summary(&"n".repeat(300), None, None, "Summary", vec![], vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        assert_eq!(embed.title.chars().count(), 256);
        assert!(embed.title.ends_with('…'));
        assert!(embed.title.starts_with("o/"));
    }

    #[test]
    fn t12_build_embed_truncates_japanese_description() {
        // Given
        let summary = make_summary("n", None, None, &"あ".repeat(5000), vec![], vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        assert!(embed.byte_count() <= 8000);
        assert!(embed.description.ends_with('…'));
        assert!(embed.description.chars().count() < 4096);
    }

    #[test]
    fn t13_build_embed_truncates_field_value() {
        // Given
        let use_cases = (0..200).map(|i| format!("ユースケース{i}")).collect();
        let summary = make_summary("n", None, None, "Summary", use_cases, vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        let f = embed
            .fields
            .iter()
            .find(|f| f.name == "Key Use Cases")
            .unwrap();
        assert!(f.value.len() <= 1024);
        assert!(f.value.ends_with('…'));
    }

    #[test]
    fn t14_build_embed_total_within_6000_when_everything_is_maxed() {
        // Given
        let summary = make_summary(
            &"n".repeat(300),
            Some("d".repeat(5000)),
            Some("L".repeat(2000)),
            &"あ".repeat(5000),
            vec!["u".repeat(2000)],
            vec!["t".repeat(2000)],
        );
        // When
        let embed = build_embed(&summary);
        // Then
        assert!(embed.char_count() <= 6000);
        assert!(embed.description.ends_with('…'));
        assert_within_limits(&embed);
    }

    #[rstest]
    #[case(simple_summary())]
    #[case(make_summary("n", Some("d".into()), Some("Rust".into()), "s", vec!["a".into()], vec!["b".into()]))]
    #[case(make_summary(&"n".repeat(300), None, None, &"あ".repeat(5000), vec![], vec![]))]
    #[case(make_summary("n", Some("🦀".repeat(5000)), Some("L".repeat(2000)), "s", vec!["u".repeat(2000)], vec!["t".repeat(2000)]))]
    #[case(make_summary(&"n".repeat(300), Some("d".repeat(5000)), Some("L".repeat(2000)), &"あ".repeat(5000), vec!["u".repeat(2000)], vec!["t".repeat(2000)]))]
    #[case(make_summary(&"🦀".repeat(300), Some("あ".repeat(5000)), Some("あ".repeat(2000)), &"あ".repeat(5000), vec!["あ".repeat(2000)], vec!["あ".repeat(2000)]))]
    fn p3_build_embed_respects_all_limits(#[case] summary: TrendingSummary) {
        // When
        let embed = build_embed(&summary);
        // Then
        assert_within_limits(&embed);
    }

    // ---- truncate_bytes (T35-T39, P5) ----

    #[test]
    fn t35_truncate_bytes_keeps_short_text() {
        assert_eq!(truncate_bytes("abc", 5), "abc");
    }

    #[test]
    fn t36_truncate_bytes_keeps_text_at_exact_limit() {
        // Given: 9 bytes
        // When / Then
        assert_eq!(truncate_bytes("あああ", 9), "あああ");
    }

    #[test]
    fn t37_truncate_bytes_cuts_on_char_boundary() {
        // Given: 12 bytes
        // When
        let result = truncate_bytes("ああああ", 9);
        // Then
        assert_eq!(result, "ああ…");
        assert_eq!(result.len(), 9);
    }

    #[test]
    fn t38_truncate_bytes_never_splits_a_char() {
        // When
        let result = truncate_bytes("aああ", 5);
        // Then: "aあ…" would be 7 bytes
        assert_eq!(result, "a…");
    }

    #[test]
    fn t39_truncate_bytes_four_byte_chars() {
        // When
        let result = truncate_bytes("🦀🦀🦀", 8);
        // Then
        assert_eq!(result, "🦀…");
        assert_eq!(result.len(), 7);
    }

    #[rstest]
    #[case("", 3)]
    #[case("abc", 3)]
    #[case("abcdef", 3)]
    #[case("abcdef", 4)]
    #[case("あいうえお", 3)]
    #[case("あいうえお", 4)]
    #[case("あいうえお", 8)]
    #[case("あいうえお", 14)]
    #[case("a🦀あ🦀a🦀", 5)]
    #[case("a🦀あ🦀a🦀", 9)]
    #[case("a🦀あ🦀a🦀", 12)]
    fn p5_truncate_bytes_invariants(#[case] text: &str, #[case] max: usize) {
        // When
        let result = truncate_bytes(text, max);
        // Then: within limit (and valid UTF-8 by construction of String)
        assert!(result.len() <= max);
        if text.len() <= max {
            assert_eq!(result, text);
        } else {
            assert!(result.ends_with('…'));
            assert!(text.starts_with(result.trim_end_matches('…')));
        }
    }

    // ---- byte_count (T40) ----

    #[test]
    fn t40_byte_count_sums_utf8_bytes_of_text_parts() {
        // Given
        let embed = DiscordEmbed {
            title: "abc".to_string(),
            url: "https://github.com/a/b".to_string(),
            description: "あいう".to_string(),
            color: EMBED_COLOR,
            fields: vec![DiscordEmbedField {
                name: "Stars".to_string(),
                value: "★1".to_string(),
                inline: true,
            }],
            footer: DiscordEmbedFooter {
                text: "GitHub Trending".to_string(),
            },
        };
        // When / Then
        assert_eq!(embed.byte_count(), 3 + 9 + 5 + 4 + 15);
    }

    // ---- build_embed byte limits (T41-T43) ----

    #[test]
    fn t41_build_embed_truncates_japanese_summary_to_8000_bytes() {
        // Given: 4000 chars, 12000 bytes
        let summary = make_summary("n", None, None, &"あ".repeat(4000), vec![], vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        assert!(embed.byte_count() <= 8000);
        assert!(embed.description.ends_with('…'));
        assert!(embed.description.chars().count() < 4000);
    }

    #[test]
    fn t42_build_embed_truncates_japanese_field_value_to_1024_bytes() {
        // Given
        let summary = make_summary("n", None, None, "Summary", vec!["あ".repeat(1000)], vec![]);
        // When
        let embed = build_embed(&summary);
        // Then
        let f = embed
            .fields
            .iter()
            .find(|f| f.name == "Key Use Cases")
            .unwrap();
        assert!(f.value.len() <= 1024);
        assert!(f.value.ends_with('…'));
    }

    #[test]
    fn t43_build_embed_fits_8000_bytes_and_6000_chars_when_maxed_with_multibyte() {
        // Given
        let summary = make_summary(
            &"🦀".repeat(300),
            Some("あ".repeat(5000)),
            Some("あ".repeat(2000)),
            &"あ".repeat(5000),
            vec!["あ".repeat(2000)],
            vec!["あ".repeat(2000)],
        );
        // When
        let embed = build_embed(&summary);
        // Then
        assert!(embed.byte_count() <= 8000);
        assert!(embed.char_count() <= 6000);
        assert_within_limits(&embed);
    }

    // ---- parse_retry_after (T22-T25) ----

    #[test]
    fn t22_parse_retry_after_reads_seconds() {
        let body = r#"{"message":"You are being rate limited.","retry_after":1.5,"global":false}"#;
        assert_eq!(parse_retry_after(body), Duration::from_millis(1500));
    }

    #[rstest]
    #[case("oops")]
    #[case(r#"{"message":"x"}"#)]
    fn t23_parse_retry_after_defaults_to_one_second(#[case] body: &str) {
        assert_eq!(parse_retry_after(body), Duration::from_secs(1));
    }

    #[test]
    fn t24_parse_retry_after_caps_at_60_seconds() {
        assert_eq!(
            parse_retry_after(r#"{"retry_after":3600}"#),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn t25_parse_retry_after_negative_is_zero() {
        assert_eq!(parse_retry_after(r#"{"retry_after":-1}"#), Duration::ZERO);
    }

    // ---- build_failure_embed (T39-T42, P7) ----

    fn failure_report(message: &str) -> FailureReport {
        FailureReport {
            message: message.to_string(),
            language: None,
            execution: None,
        }
    }

    #[test]
    fn t39_build_failure_embed_full() {
        // Given
        let report = FailureReport {
            message: "GitHub API error: 500".to_string(),
            language: None,
            execution: Some("github-trend-digest-abc12".to_string()),
        };

        // When
        let embed = build_failure_embed(&report);

        // Then
        assert_eq!(embed.title, "GitHub Trending digest failed");
        assert!(embed.description.contains("GitHub API error: 500"));
        assert_eq!(embed.color, 0xD73A49);
        assert_eq!(embed.url, "https://github.com/trending");
        assert_eq!(
            embed.fields,
            vec![
                field("Language", "all", true),
                field("Execution", "github-trend-digest-abc12", true),
            ]
        );
        assert_eq!(embed.footer.text, "GitHub Trending");
    }

    #[test]
    fn t40_build_failure_embed_with_language_without_execution() {
        // Given
        let report = FailureReport {
            message: "x".to_string(),
            language: Some("rust".to_string()),
            execution: None,
        };

        // When
        let embed = build_failure_embed(&report);

        // Then
        assert_eq!(embed.fields, vec![field("Language", "rust", true)]);
    }

    #[test]
    fn t41_build_failure_embed_truncates_long_multibyte_message() {
        // Given
        let report = failure_report(&"あ".repeat(5000));

        // When
        let embed = build_failure_embed(&report);

        // Then
        assert!(embed.description.chars().count() <= DESCRIPTION_LIMIT);
        assert!(embed.description.ends_with('…'));
        assert!(embed.char_count() <= TOTAL_CHARS_PER_MESSAGE);
        assert!(embed.byte_count() <= MAX_EMBED_BYTES);
    }

    #[test]
    fn t42_build_failure_embed_empty_message() {
        // Given
        let report = failure_report("");

        // When
        let embed = build_failure_embed(&report);

        // Then
        assert_eq!(embed.description, "(no error message)");
    }

    #[rstest]
    #[case::ascii("GitHub API error: 500".to_string())]
    #[case::multibyte("障害".repeat(3000))]
    #[case::empty(String::new())]
    #[case::ten_thousand("x".repeat(10_000))]
    fn p7_build_failure_embed_respects_discord_limits(#[case] message: String) {
        // Given
        let report = FailureReport {
            message,
            language: Some("rust".to_string()),
            execution: Some("exec-1".to_string()),
        };

        // When
        let embed = build_failure_embed(&report);

        // Then
        assert!(embed.char_count() <= 6000);
        assert!(embed.byte_count() <= MAX_EMBED_BYTES);
        assert!(embed.description.chars().count() <= 4096);
    }
}
