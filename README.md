# GitHub Trending Summary

## 1. Project Overview

This project is a Rust application that daily fetches trending repository information from GitHub, summarizes it using the Gemini AI, and notifies a Discord channel. It serves as a tool to efficiently keep up with the latest trends in the developer community.

## 2. Architecture

This project adopts the **Hexagonal Architecture** (also known as Ports and Adapters).

### Why Hexagonal Architecture?
- **Separation of Concerns**: The core business logic (domain) is completely isolated from external infrastructure details (like web APIs, databases, or notification clients). This makes the domain pure and easy to reason about.
- **Testability**: With the domain logic decoupled from I/O, it can be tested with simple unit tests. Dependencies are injected via "ports" (defined as Rust traits), which allows for easy mocking using libraries like `mockall`.
- **Interchangeability**: Adapters for external services can be swapped out with minimal impact on the core application. For example, the notification target could be switched from Discord to Slack by simply implementing a new adapter.
- **Clear Dependency Rule**: Dependencies always point inward towards the domain, preventing the business logic from getting entangled with implementation details.

### Layers
- **`domain`**: Contains the core business logic, entities (e.g., `Repository`, `Summary`), and ports (traits) that define contracts for external services. It has no dependencies on other layers.
- **`application`**: Orchestrates the business logic. It contains use cases (e.g., "fetch and summarize trending repos") that are triggered by the presentation layer. It depends only on the `domain` layer.
- **`infrastructure`**: Implements the adapters for the ports defined in the `domain` layer. This is where all external communication happens (e.g., HTTP requests to GitHub, calls to the Gemini API, sending messages to Discord).
- **`presentation`**: The entry point of the application. It's responsible for triggering the application's use cases. This could be a command-line interface, a scheduled job runner, or a web server.

## 3. Setup Instructions

### Prerequisites
- Rust 1.70 or higher
- Git
- Discord account (for notifications)
- Google Cloud account (for Gemini API)

### Quick Start

1.  **Clone the repository:**
    ```bash
    git clone <repository-url>
    cd github-trending-summary
    ```

2.  **Set up environment variables:**
    Copy the example environment file and fill in the required values.
    ```bash
    cp .env.example .env
    # Edit .env and fill in your API keys
    ```

    Required environment variables:
    - `DISCORD_WEBHOOK_URL`: Discord webhook URL for notifications
    - `GEMINI_API_KEY`: Google Gemini API key for AI summarization

    Optional environment variables:
    - `GITHUB_TOKEN`: GitHub personal access token (recommended for higher rate limits)
    - `TARGET_LANGUAGE`: Language filter (e.g., `rust`, `python`, `javascript`)
    - `LOG_LEVEL`: Logging level (default: `info`)

    📖 **For detailed setup instructions, see [docs/SETUP_DEV.md](docs/SETUP_DEV.md)**

3.  **Test the scraper:**
    ```bash
    # Test scraping GitHub Trending
    cargo run --example test_scraper

    # Test with language filter
    cargo run --example test_scraper -- rust
    ```

4.  **Build the application:**
    ```bash
    cargo build --release
    ```

5.  **Run the application:**
    ```bash
    ./target/release/github-trending-summary
    ```

### Run with Docker

Build the image and run it with secrets injected at runtime (secrets are never baked into the image; `.env*` is excluded by `.dockerignore`).

```bash
docker build -t github-trending-summary .

# Run with secrets loaded from .env (the argument is the language filter)
docker run --rm --env-file .env github-trending-summary rust

# Without the required environment variables, the process exits with a non-zero code
docker run --rm github-trending-summary rust; echo "exit=$?"   # exit=1
```

### Scheduled Execution (Cloud Run Jobs + Cloud Scheduler)

The digest runs as the Cloud Run Job `github-trend-digest`, started by Cloud Scheduler at 08:00 JST and 18:00 JST. The Job takes no arguments and covers all languages. The `Deploy Cloud Run Job` workflow (`.github/workflows/deploy.yml`) builds the image on every push to `main`, updates the Job (it does not run it), and deletes every image except the deployed one.

Setup, secrets, the `GITHUB_TOKEN` (PAT) rotation and rollback are described in **[docs/DEPLOY_CLOUD_RUN.md](docs/DEPLOY_CLOUD_RUN.md)**.

**Run manually:**

```bash
gcloud run jobs execute github-trend-digest --region=<REGION> --wait
```

**Failure notifications:** the app posts a failure message to Discord when

- the run ends with an error (exit code 1), or
- no summary was produced: Trending returned 0 repositories, or every summary failed. The exit code stays 0 in these two cases, so Cloud Run shows `Succeeded`; the Discord message is the only signal.

Failures the app cannot report itself (container fails to start, secrets unreadable, timeout, OOM, Scheduler cannot start the Job, Discord outage, configuration error, panic, partial summary failures) are listed with ways to detect them in section 16 of the guide. The basic check is noticing that the 08:00 / 18:00 digest did not arrive.

## 4. Development Guidelines

### Coding Style
- Follow the official [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
- Run `cargo fmt` before committing to ensure consistent code formatting.
- Run `cargo clippy` to catch common mistakes and improve code quality.

### Testing
- **Unit Tests**: Place unit tests within the module they are testing. The domain logic should be thoroughly covered by unit tests.
- **Integration Tests**: Place integration tests in the `tests/` directory. These tests should cover the interaction between different components, especially the application and infrastructure layers. Use mocks for external services where appropriate.
- All new features should be accompanied by relevant tests.

## 5. Project Status

### Completed ✅
- [x] Domain layer entities and repository traits
- [x] Application layer use case structure
- [x] Infrastructure layer configuration and logging
- [x] **GitHub Trending scraper implementation**
  - Complete with comprehensive unit and integration tests
  - Resilient CSS selector strategy with fallbacks
  - Full error handling and logging
  - See [docs/SCRAPING_IMPLEMENTATION.md](docs/SCRAPING_IMPLEMENTATION.md) for details

### In Progress 🚧
- [ ] Implement Gemini AI summarizer in the `infrastructure` layer
- [ ] Implement Discord notifier in the `infrastructure` layer
- [ ] Wire up the main use case with all components

### Planned 📋
- [ ] Set up a scheduler in the `presentation` layer to run the process daily
- [ ] Add support for more summarization models
- [ ] Add more configuration options (e.g., summary format, notification frequency)
- [ ] Implement caching mechanism for rate limit optimization
- [ ] Add retry logic with exponential backoff

## 6. Documentation

- **[Quick Start](docs/QUICK_START.md)**: 5-minute setup guide
- **[Cloud Run Jobs Deployment](docs/DEPLOY_CLOUD_RUN.md)**: Scheduled execution setup and operations
- **[Setup Guide](docs/SETUP_DEV.md)**: Detailed development environment setup
- **[Environment Files](docs/ENV_FILES.md)**: Environment file management and security best practices
- **[Scraping Implementation](docs/SCRAPING_IMPLEMENTATION.md)**: GitHub Trending scraper technical details
