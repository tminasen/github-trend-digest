//! GitHub Trending Summary Library
//!
//! This library provides functionality for fetching trending GitHub repositories,
//! generating AI-powered summaries, and sending notifications.
//!
//! # Architecture
//! The application follows hexagonal (ports and adapters) architecture:
//! - `domain`: Core business logic and entities
//! - `application`: Use cases and application services
//! - `infrastructure`: External integrations (API clients, databases, etc.)
//! - `presentation`: User interface and entry points
// Declare the top-level modules in the hexagonal architecture.
pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod presentation;

// Re-export commonly used types for convenience
pub use application::{DigestOutcome, EmptyDigestReason, FetchAndSummarizeTrendingUseCase};
pub use domain::{
    redact_secrets, AppError, AppResult, FailureNotifier, FailureReport, GithubApiClient,
    Repository, TrendingSummary,
};
pub use infrastructure::{
    init_logging, AppConfig, DiscordNotificationSender, GeminiClient, GithubApiClientImpl,
    TrendingScraperClient,
};
