//! Main entry point for the GitHub Trending Summary application.
use std::process::ExitCode;
use std::sync::Arc;

use tracing::{error, info, warn};

use github_trending_summary::{
    init_logging, redact_secrets, AppConfig, AppResult, DigestOutcome, DiscordNotificationSender,
    FailureNotifier, FailureReport, FetchAndSummarizeTrendingUseCase, GeminiClient,
    GithubApiClient, GithubApiClientImpl, TrendingScraperClient,
};

#[tokio::main]
async fn main() -> ExitCode {
    // Load configuration from environment (before logging is available)
    let config = match AppConfig::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Configuration error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = init_logging(config.log_level()) {
        eprintln!("Failed to initialize logging: {e}");
        return ExitCode::FAILURE;
    }

    info!("GitHub Trending Summary application started");

    let language = resolve_language(std::env::args().nth(1), config.target_language());
    info!("Language filter: {:?}", language);

    let notifier = match DiscordNotificationSender::new(config.discord_webhook_url().to_string()) {
        Ok(notifier) => notifier,
        Err(e) => {
            error!("Failed to create failure notifier: {e}");
            return ExitCode::FAILURE;
        }
    };
    let secrets = [
        config.discord_webhook_url(),
        config.gemini_api_key(),
        config.github_token().unwrap_or(""),
    ];
    let execution = std::env::var("CLOUD_RUN_EXECUTION").ok();

    let result = run(&config, language.clone()).await;
    finish(result, &notifier, &secrets, language, execution).await
}

/// Turns the run result into an exit code and, when something went wrong,
/// reports it through `notifier` (best-effort).
///
/// - `Ok` with summaries: no notification, `SUCCESS`.
/// - `Ok` without summaries (nothing fetched / every summary failed): notifies,
///   but still returns `SUCCESS`.
/// - `Err`: notifies and returns `FAILURE`.
///
/// Secrets are redacted before anything is logged or sent. A failing
/// notification is only logged; it never changes the exit code.
async fn finish(
    result: AppResult<DigestOutcome>,
    notifier: &dyn FailureNotifier,
    secrets: &[&str],
    language: Option<String>,
    execution: Option<String>,
) -> ExitCode {
    let (message, exit_code) = match result {
        Ok(outcome) => match outcome.empty_reason() {
            None => {
                info!(
                    "Successfully generated and sent {} summaries",
                    outcome.summaries.len()
                );
                return ExitCode::SUCCESS;
            }
            Some(reason) => {
                warn!("No summaries were generated: {reason}");
                (reason.to_string(), ExitCode::SUCCESS)
            }
        },
        Err(e) => {
            let message = redact_secrets(&e.to_string(), secrets);
            error!("Application failed: {message}");
            (message, ExitCode::FAILURE)
        }
    };

    let report = FailureReport {
        message,
        language,
        execution,
    };
    if let Err(e) = notifier.notify_failure(&report).await {
        // Best-effort: never change the exit code because of a failed notification.
        error!(
            "Failed to send failure notification: {}",
            redact_secrets(&e.to_string(), secrets)
        );
    }
    exit_code
}

/// Builds the dependency graph and executes the main use case.
async fn run(config: &AppConfig, language: Option<String>) -> AppResult<DigestOutcome> {
    let scraper = Arc::new(TrendingScraperClient::new()?);
    let github_client = Arc::new(GithubApiClientImpl::new(
        config.github_token().map(String::from),
    )?);
    let gemini_client = Arc::new(GeminiClient::new(
        config.gemini_api_key().to_string(),
        config.summarizer().clone(),
    )?);
    let notification_sender = Arc::new(DiscordNotificationSender::new(
        config.discord_webhook_url().to_string(),
    )?);

    let use_case = FetchAndSummarizeTrendingUseCase::new(
        scraper,
        github_client.clone(),
        gemini_client,
        notification_sender,
    );

    // Fail fast on misconfigured tokens; skip the check when unauthenticated
    if config.github_token().is_some() {
        github_client.verify_credentials().await?;
        info!("GitHub credentials verified");
    } else {
        info!("GITHUB_TOKEN not set; using unauthenticated GitHub API access");
    }

    use_case.execute_outcome(language).await
}

/// Resolves the language filter with priority: CLI argument > config value.
///
/// Values are trimmed; an empty result is treated as `None` (all languages).
fn resolve_language(cli_arg: Option<String>, config_lang: Option<&str>) -> Option<String> {
    cli_arg
        .as_deref()
        .or(config_lang)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_language_cli_arg_wins() {
        // Given: CLI引数とconfigの両方に言語が設定されている
        let cli_arg = Some("rust".to_string());
        let config_lang = Some("go");

        // When: 言語を解決する
        let result = resolve_language(cli_arg, config_lang);

        // Then: CLI引数が優先される
        assert_eq!(result, Some("rust".to_string()));
    }

    #[test]
    fn test_resolve_language_falls_back_to_config() {
        // Given: CLI引数がなくconfigに言語が設定されている
        let cli_arg = None;
        let config_lang = Some("go");

        // When: 言語を解決する
        let result = resolve_language(cli_arg, config_lang);

        // Then: configの値にフォールバックする
        assert_eq!(result, Some("go".to_string()));
    }

    #[test]
    fn test_resolve_language_empty_string_is_none() {
        // Given: CLI引数もconfigも空文字列
        let cli_arg = Some("".to_string());
        let config_lang = Some("");

        // When: 言語を解決する
        let result = resolve_language(cli_arg, config_lang);

        // Then: Noneとして扱う（全言語トレンド）
        assert_eq!(result, None);
    }

    // ---- finish (T49-T52, T65-T67) ----

    use std::sync::{Arc as StdArc, Mutex};

    use async_trait::async_trait;
    use github_trending_summary::{AppError, Repository, TrendingSummary};

    mockall::mock! {
        pub Notifier {}

        #[async_trait]
        impl FailureNotifier for Notifier {
            async fn notify_failure(&self, report: &FailureReport) -> AppResult<()>;
        }
    }

    fn summary() -> TrendingSummary {
        let repo = Repository::new(
            "n".to_string(),
            "o".to_string(),
            "https://github.com/o/n".to_string(),
            None,
            1,
            None,
            vec![],
        )
        .unwrap();
        TrendingSummary::new(repo, "S".to_string(), vec![], vec![]).unwrap()
    }

    /// A mock that expects exactly one call and records the report it receives.
    fn recording_notifier(
        reply: AppResult<()>,
    ) -> (MockNotifier, StdArc<Mutex<Vec<FailureReport>>>) {
        let reports = StdArc::new(Mutex::new(Vec::new()));
        let sink = reports.clone();
        let mut notifier = MockNotifier::new();
        let mut reply = Some(reply);
        notifier
            .expect_notify_failure()
            .times(1)
            .returning(move |report| {
                sink.lock().unwrap().push(report.clone());
                reply.take().unwrap()
            });
        (notifier, reports)
    }

    fn empty_outcome(fetched: usize) -> AppResult<DigestOutcome> {
        Ok(DigestOutcome {
            fetched,
            summaries: vec![],
        })
    }

    #[tokio::test]
    async fn t49_finish_does_not_notify_on_success_with_summaries() {
        // Given
        let mut notifier = MockNotifier::new();
        notifier.expect_notify_failure().times(0);
        let result = Ok(DigestOutcome {
            fetched: 25,
            summaries: vec![summary()],
        });

        // When
        let code = finish(result, &notifier, &[], None, None).await;

        // Then
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[tokio::test]
    async fn t50_finish_notifies_once_and_fails_on_error() {
        // Given
        let (notifier, reports) = recording_notifier(Ok(()));
        let result = Err(AppError::GithubApiError("500".into()));

        // When
        let code = finish(result, &notifier, &[], None, Some("exec-1".into())).await;

        // Then
        assert_eq!(code, ExitCode::FAILURE);
        assert_eq!(
            *reports.lock().unwrap(),
            vec![FailureReport {
                message: "GitHub API error: 500".to_string(),
                language: None,
                execution: Some("exec-1".to_string()),
            }]
        );
    }

    #[tokio::test]
    async fn t51_finish_stays_failure_when_notification_fails() {
        // Given: the notifier itself is down
        let (notifier, _) = recording_notifier(Err(AppError::NotificationError("down".into())));
        let result = Err(AppError::ScrapingError("x".into()));

        // When
        let code = finish(result, &notifier, &[], None, None).await;

        // Then: still FAILURE, exactly one attempt (mock enforces times(1))
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[tokio::test]
    async fn t52_finish_redacts_secrets_in_the_notified_message() {
        // Given
        let (notifier, reports) = recording_notifier(Ok(()));
        let result = Err(AppError::NetworkError(
            "error for url (https://discord.com/api/webhooks/1/tok)".into(),
        ));
        let secrets = ["https://discord.com/api/webhooks/1/tok"];

        // When
        finish(result, &notifier, &secrets, None, None).await;

        // Then
        let message = reports.lock().unwrap()[0].message.clone();
        assert!(!message.contains("webhooks/1/tok"), "{message}");
        assert!(message.contains("[REDACTED]"), "{message}");
    }

    #[tokio::test]
    async fn t65_finish_notifies_but_succeeds_when_nothing_was_fetched() {
        // Given
        let (notifier, reports) = recording_notifier(Ok(()));

        // When
        let code = finish(
            empty_outcome(0),
            &notifier,
            &[],
            None,
            Some("exec-1".into()),
        )
        .await;

        // Then
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(
            *reports.lock().unwrap(),
            vec![FailureReport {
                message: "No trending repositories found (0 fetched)".to_string(),
                language: None,
                execution: Some("exec-1".to_string()),
            }]
        );
    }

    #[tokio::test]
    async fn t66_finish_notifies_but_succeeds_when_every_summary_failed() {
        // Given
        let (notifier, reports) = recording_notifier(Ok(()));

        // When
        let code = finish(
            empty_outcome(25),
            &notifier,
            &[],
            None,
            Some("exec-1".into()),
        )
        .await;

        // Then
        assert_eq!(code, ExitCode::SUCCESS);
        assert_eq!(
            reports.lock().unwrap()[0].message,
            "All 25 repositories failed to summarize (0/25 succeeded)"
        );
    }

    #[tokio::test]
    async fn t67_finish_stays_success_when_notifying_an_empty_run_fails() {
        // Given
        let (notifier, _) = recording_notifier(Err(AppError::NotificationError("down".into())));

        // When
        let code = finish(empty_outcome(0), &notifier, &[], None, None).await;

        // Then
        assert_eq!(code, ExitCode::SUCCESS);
    }
}
