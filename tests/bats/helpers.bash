# Shared helpers for cleanup-old-images.bats.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
IMG="asia-northeast1-docker.pkg.dev/p/r/github-trend-digest"

# dg N -> sha256: followed by 64 hex digits of N (N is one hex digit).
dg() {
  local d=$1 out="" i
  for ((i = 0; i < 64; i++)); do out+="$d"; done
  printf 'sha256:%s' "$out"
}

# rows N... -> gcloud JSON array of IMG versions in the given order.
rows() {
  local args=() n
  for n in "$@"; do args+=("$(dg "$n")"); done
  jq -nc --arg img "$IMG" '$ARGS.positional | map({package:$img, version:., tags:"", createTime:"2026-11-03T00:00:00Z", updateTime:"2026-11-03T00:00:00Z"})' --args "${args[@]}"
}

# Puts the stubs first in PATH and prepares an empty $STUB_LOG.
setup_stubs() {
  export STUB_LOG="$BATS_TEST_TMPDIR/stub.log"
  : >"$STUB_LOG"
  export PATH="$BATS_TEST_DIRNAME/stubs:$PATH"
  DOCKER_STUB_MANIFEST_JSON=$(cat "$BATS_TEST_DIRNAME/fixtures/manifest-docker.json")
  export DOCKER_STUB_MANIFEST_JSON
}

fixture() { cat "$BATS_TEST_DIRNAME/fixtures/$1"; }
