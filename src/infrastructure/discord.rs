//! Discord Webhook notification sender implementation.
use async_trait::async_trait;
use reqwest::Client;
use tracing::{debug, error, info, warn};

mod embed;

use embed::{build_embed, build_failure_embed, parse_retry_after, DiscordWebhookPayload};

use crate::domain::{
    AppError, AppResult, FailureNotifier, FailureReport, NotificationSender, TrendingSummary,
};

/// Number of times a rate-limited (429) message is re-sent.
const MAX_RATE_LIMIT_RETRIES: u32 = 3;

/// Client for sending notifications to Discord via Webhook.
pub struct DiscordNotificationSender {
    webhook_url: String,
    client: Client,
}

impl DiscordNotificationSender {
    /// Creates a new DiscordNotificationSender instance.
    ///
    /// # Returns
    /// - `Ok(DiscordNotificationSender)`: Successfully created sender
    /// - `Err(AppError)`: Failed to build the HTTP client
    pub fn new(webhook_url: String) -> AppResult<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::InternalError(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            webhook_url,
            client,
        })
    }
}

impl DiscordNotificationSender {
    /// Posts one message, retrying on HTTP 429 after the `retry_after` Discord asks for.
    async fn post_with_retry(&self, payload: &DiscordWebhookPayload) -> AppResult<()> {
        let mut retries = 0;
        loop {
            let response = self
                .client
                .post(&self.webhook_url)
                .json(payload)
                .send()
                .await
                .map_err(|e| {
                    error!("Network error sending Discord notification: {}", e);
                    AppError::NetworkError(e.to_string())
                })?;

            let status = response.status();
            if status.is_success() {
                debug!("Discord notification sent successfully");
                return Ok(());
            }

            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            if status.as_u16() == 429 && retries < MAX_RATE_LIMIT_RETRIES {
                retries += 1;
                let wait = parse_retry_after(&body);
                warn!(
                    "Discord rate limited (retry {}/{}), waiting {:?}",
                    retries, MAX_RATE_LIMIT_RETRIES, wait
                );
                tokio::time::sleep(wait).await;
                continue;
            }

            error!("Discord API error: status={}, body={}", status, body);
            return Err(AppError::NotificationError(format!(
                "Discord API error {}: {}",
                status.as_u16(),
                body
            )));
        }
    }
}

#[async_trait]
impl NotificationSender for DiscordNotificationSender {
    async fn send(&self, summary: &TrendingSummary) -> AppResult<()> {
        self.send_batch(std::slice::from_ref(summary)).await
    }

    async fn send_batch(&self, summaries: &[TrendingSummary]) -> AppResult<()> {
        // One repository per message keeps each payload well under Discord's size limit.
        for summary in summaries {
            let embed = build_embed(summary);
            info!("Sending notification to Discord for {}", embed.title);
            self.post_with_retry(&DiscordWebhookPayload {
                embeds: vec![embed],
            })
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl FailureNotifier for DiscordNotificationSender {
    async fn notify_failure(&self, report: &FailureReport) -> AppResult<()> {
        info!("Sending failure notification to Discord");
        self.post_with_retry(&DiscordWebhookPayload {
            embeds: vec![build_failure_embed(report)],
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Repository;
    use httpmock::prelude::*;
    use std::time::Duration;

    fn make_sender(server: &MockServer) -> DiscordNotificationSender {
        DiscordNotificationSender {
            webhook_url: server.base_url(),
            client: Client::new(),
        }
    }

    fn make_summary(owner: &str, name: &str, ai_summary: &str) -> TrendingSummary {
        let repo = Repository::new(
            name.to_string(),
            owner.to_string(),
            format!("https://github.com/{owner}/{name}"),
            None,
            10,
            None,
            vec![],
        )
        .unwrap();
        TrendingSummary::new(repo, ai_summary.to_string(), vec![], vec![]).unwrap()
    }

    fn body_json(req: &HttpMockRequest) -> Option<serde_json::Value> {
        serde_json::from_slice(req.body().as_ref()).ok()
    }

    fn embeds_len_is(req: &HttpMockRequest, len: usize) -> bool {
        body_json(req).is_some_and(|b| {
            b.get("content").is_none()
                && b.get("embeds")
                    .and_then(|e| e.as_array())
                    .is_some_and(|e| e.len() == len)
        })
    }

    #[test]
    fn test_new_creates_sender() {
        // Given/When: creating a sender with a valid webhook URL
        let result = DiscordNotificationSender::new("https://example.com/webhook".to_string());

        // Then: construction succeeds
        assert!(result.is_ok());
    }

    fn title_is(req: &HttpMockRequest, title: &str) -> bool {
        embeds_len_is(req, 1) && body_json(req).is_some_and(|b| b["embeds"][0]["title"] == title)
    }

    #[tokio::test]
    async fn t26_send_batch_sends_one_post_per_summary_in_order() {
        // Given
        let server = MockServer::start();
        let mock_one = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| title_is(req, "a/one"));
            then.status(204);
        });
        let mock_two = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| title_is(req, "b/two"));
            then.status(204);
        });
        let sender = make_sender(&server);
        let summaries = [
            make_summary("a", "one", "S1"),
            make_summary("b", "two", "S2"),
        ];

        // When
        let result = sender.send_batch(&summaries).await;

        // Then
        assert!(result.is_ok());
        mock_one.assert_calls(1);
        mock_two.assert_calls(1);
    }

    #[tokio::test]
    async fn t27_send_batch_sends_eleven_messages_for_eleven_summaries() {
        // Given
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| embeds_len_is(req, 1));
            then.status(204);
        });
        let sender = make_sender(&server);
        let summaries: Vec<_> = (0..11)
            .map(|i| make_summary("o", &format!("repo{i}"), "S"))
            .collect();

        // When
        let result = sender.send_batch(&summaries).await;

        // Then
        assert!(result.is_ok());
        mock.assert_calls(11);
    }

    #[tokio::test]
    async fn t28_send_batch_sends_large_summaries_as_separate_messages() {
        // Given: each embed is over 4000 chars
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| embeds_len_is(req, 1));
            then.status(204);
        });
        let sender = make_sender(&server);
        let text = "a".repeat(4000);
        let summaries: Vec<_> = (0..3)
            .map(|i| make_summary("o", &format!("repo{i}"), &text))
            .collect();

        // When
        let result = sender.send_batch(&summaries).await;

        // Then
        assert!(result.is_ok());
        mock.assert_calls(3);
    }

    #[tokio::test]
    async fn t44_send_batch_stops_at_first_failing_message() {
        // Given
        let server = MockServer::start();
        let mock_one = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| title_is(req, "a/one"));
            then.status(204);
        });
        let mock_two = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| title_is(req, "b/two"));
            then.status(400).body("Bad Request");
        });
        let mock_three = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .is_true(|req| title_is(req, "c/three"));
            then.status(204);
        });
        let sender = make_sender(&server);
        let summaries = [
            make_summary("a", "one", "S1"),
            make_summary("b", "two", "S2"),
            make_summary("c", "three", "S3"),
        ];

        // When
        let result = sender.send_batch(&summaries).await;

        // Then
        match result.unwrap_err() {
            AppError::NotificationError(msg) => assert!(msg.contains("400")),
            _ => panic!("Expected NotificationError"),
        }
        mock_one.assert_calls(1);
        mock_two.assert_calls(1);
        mock_three.assert_calls(0);
    }

    #[tokio::test]
    async fn t29_send_batch_empty_makes_no_request() {
        // Given
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(204);
        });
        let sender = make_sender(&server);

        // When
        let result = sender.send_batch(&[]).await;

        // Then
        assert!(result.is_ok());
        mock.assert_calls(0);
    }

    #[tokio::test]
    async fn t30_send_sends_one_embed() {
        // Given
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/")
                .header("content-type", "application/json")
                .is_true(|req| embeds_len_is(req, 1));
            then.status(204);
        });
        let sender = make_sender(&server);
        let summary = make_summary("rust-lang", "rust", "A systems programming language.");

        // When
        let result = sender.send(&summary).await;

        // Then
        assert!(result.is_ok());
        mock.assert_calls(1);
    }

    #[tokio::test]
    async fn t31_send_api_error() {
        // Given
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(400).body("Bad Request");
        });
        let sender = make_sender(&server);
        let summary = make_summary("rust-lang", "rust", "Summary");

        // When
        let result = sender.send(&summary).await;

        // Then
        mock.assert();
        match result.unwrap_err() {
            AppError::NotificationError(msg) => {
                assert!(msg.contains("400"));
                assert!(msg.contains("Bad Request"));
            }
            _ => panic!("Expected NotificationError"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn t32_send_retries_after_429_and_succeeds() {
        // Given: the first response is 429; it is swapped for 204 once it has been hit
        let server = MockServer::start();
        let mut mock_429 = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(429).body(
                r#"{"message":"You are being rate limited.","retry_after":0.2,"global":false}"#,
            );
        });
        let sender = make_sender(&server);
        let summary = make_summary("rust-lang", "rust", "Summary");

        // When
        let handle = tokio::spawn(async move { sender.send(&summary).await });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while mock_429.calls() < 1 {
            assert!(std::time::Instant::now() < deadline, "429 was never hit");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        mock_429.delete();
        let mock_204 = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(204);
        });
        let result = handle.await.unwrap();

        // Then: one 429 followed by one successful retry
        assert!(result.is_ok());
        mock_204.assert_calls(1);
    }

    #[tokio::test]
    async fn t33_send_gives_up_after_three_retries_on_429() {
        // Given
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(429).body(r#"{"retry_after":0.01}"#);
        });
        let sender = make_sender(&server);
        let summary = make_summary("rust-lang", "rust", "Summary");

        // When
        let result = sender.send(&summary).await;

        // Then: 1 initial attempt + 3 retries
        match result.unwrap_err() {
            AppError::NotificationError(msg) => assert!(msg.contains("429")),
            _ => panic!("Expected NotificationError"),
        }
        mock.assert_calls(4);
    }

    fn failure_report() -> FailureReport {
        FailureReport {
            message: "GitHub API error: 500".to_string(),
            language: None,
            execution: Some("github-trend-digest-abc12".to_string()),
        }
    }

    #[tokio::test]
    async fn t46_notify_failure_posts_one_failure_embed() {
        // Given
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/").is_true(|req| {
                embeds_len_is(req, 1)
                    && body_json(req)
                        .is_some_and(|b| b["embeds"][0]["title"] == "GitHub Trending digest failed")
            });
            then.status(204);
        });
        let sender = make_sender(&server);

        // When
        let result = sender.notify_failure(&failure_report()).await;

        // Then
        assert!(result.is_ok());
        mock.assert_calls(1);
    }

    #[tokio::test]
    async fn t47_notify_failure_returns_error_on_500() {
        // Given
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(500).body("oops");
        });
        let sender = make_sender(&server);

        // When
        let result = sender.notify_failure(&failure_report()).await;

        // Then
        match result {
            Err(AppError::NotificationError(m)) => assert!(m.contains("500"), "{m}"),
            other => panic!("expected NotificationError, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn t48_notify_failure_retries_after_429() {
        // Given: first 429, then 204
        let server = MockServer::start();
        let mut rate_limited = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(429).body(r#"{"retry_after":0.2}"#);
        });
        let sender = make_sender(&server);

        // When: the 429 mock is replaced by a 204 mock after the first request
        let handle = tokio::spawn(async move { sender.notify_failure(&failure_report()).await });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while rate_limited.calls() < 1 {
            assert!(std::time::Instant::now() < deadline, "429 was never hit");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        rate_limited.delete();
        let ok = server.mock(|when, then| {
            when.method(POST).path("/");
            then.status(204);
        });
        let result = handle.await.unwrap();

        // Then
        assert!(result.is_ok());
        ok.assert_calls(1);
    }
}
