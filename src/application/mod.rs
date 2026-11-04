//! Application layer containing use cases and application services.
//!
//! This layer orchestrates domain objects and coordinates the flow of data
//! between the domain and infrastructure layers.
pub mod use_cases;

// Re-export commonly used types
pub use use_cases::{DigestOutcome, EmptyDigestReason, FetchAndSummarizeTrendingUseCase};
