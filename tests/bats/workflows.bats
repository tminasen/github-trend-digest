#!/usr/bin/env bats
# Static safety checks for workflows and the Dockerfile (T27-T29, T33, T58).
# Syntax is covered by actionlint; wording by review.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
DEPLOY="$REPO_ROOT/.github/workflows/deploy.yml"

# Prints the 1-based line number of the first line matching the fixed string $1.
line_of() { grep -n -F -- "$1" "$DEPLOY" | head -1 | cut -d: -f1; }

@test "T27: deploy updates the Job by digest and never runs it" {
  # Given: deploy.yml
  # When / Then
  grep -q 'job: github-trend-digest' "$DEPLOY"
  grep -Eq 'image: .*@\$\{\{ steps\.build\.outputs\.digest \}\}$' "$DEPLOY"
  run ! grep -Eq 'wait: *true' "$DEPLOY"
  run ! grep -q -- '--execute-now' "$DEPLOY"
  grep -q 'env_vars_update_strategy: overwrite' "$DEPLOY"
  grep -q 'secrets_update_strategy: overwrite' "$DEPLOY"
}

@test "T28: secrets come from Secret Manager, not from GitHub" {
  run ! grep -q '\${{ *secrets\.' "$DEPLOY"
  grep -q 'DISCORD_WEBHOOK_URL=github-trend-digest-discord-webhook-url:latest' "$DEPLOY"
  grep -q 'GEMINI_API_KEY=github-trend-digest-gemini-api-key:latest' "$DEPLOY"
  grep -q 'GITHUB_TOKEN=github-trend-digest-github-token:latest' "$DEPLOY"
}

@test "T29: cleanup runs after deploy, only on success, with the deployed digest" {
  # Given
  local deploy_line cleanup_line
  deploy_line=$(line_of 'google-github-actions/deploy-cloudrun@')
  cleanup_line=$(line_of 'scripts/cleanup-old-images.sh')
  # Then: order
  [ -n "$deploy_line" ] && [ -n "$cleanup_line" ]
  [ "$cleanup_line" -gt "$deploy_line" ]
  # Then: two arguments - the image path and the digest of the same build step
  grep -Eq 'scripts/cleanup-old-images\.sh "\$IMAGE" "\$\{\{ steps\.build\.outputs\.digest \}\}"' "$DEPLOY"
  # Then: no always() / continue-on-error
  run ! grep -Eq 'if: *always\(\)' "$DEPLOY"
  run ! grep -Eq 'continue-on-error: *true' "$DEPLOY"
}

@test "T33: secrets are not baked into the image" {
  grep -q '^\.env\*' "$REPO_ROOT/.dockerignore"
  run ! grep -Eq 'DISCORD_WEBHOOK_URL|GEMINI_API_KEY|GITHUB_TOKEN' "$REPO_ROOT/Dockerfile"
}

@test "T58: no workflow has a schedule" {
  [ ! -e "$REPO_ROOT/.github/workflows/cron.yml" ]
  run ! grep -Eq '^\s*schedule:' "$REPO_ROOT"/.github/workflows/*.yml
}
