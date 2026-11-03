#!/usr/bin/env bats
# Tests for scripts/cleanup-old-images.sh (T1-T21, T53, T68-T73, P1-P5).

bats_require_minimum_version 1.5.0

load helpers

setup() {
  # shellcheck source=/dev/null
  source "$REPO_ROOT/scripts/cleanup-old-images.sh"
  setup_stubs
}

SCRIPT="$BATS_TEST_DIRNAME/../../scripts/cleanup-old-images.sh"

# ---------------------------------------------------------------- select_digests_to_delete

@test "T1: deployed first -> everything else is deleted in input order" {
  # Given
  local input; input=$(rows 5 4 3 2 1)
  # When
  run --separate-stderr select_digests_to_delete "$(dg 5)" "$IMG" <<<"$input"
  # Then
  [ "$status" -eq 0 ]
  [ "$output" = "$(printf '%s\n%s\n%s\n%s' "$(dg 4)" "$(dg 3)" "$(dg 2)" "$(dg 1)")" ]
}

@test "T2: deployed in the middle -> everything else is deleted in input order" {
  local input; input=$(rows 5 4 3 2 1)
  run --separate-stderr select_digests_to_delete "$(dg 4)" "$IMG" <<<"$input"
  [ "$status" -eq 0 ]
  [ "$output" = "$(printf '%s\n%s\n%s\n%s' "$(dg 5)" "$(dg 3)" "$(dg 2)" "$(dg 1)")" ]
}

@test "T3: deployed last is always kept" {
  local input; input=$(rows 5 4 3 2 1)
  run --separate-stderr select_digests_to_delete "$(dg 1)" "$IMG" <<<"$input"
  [ "$status" -eq 0 ]
  [ "$output" = "$(printf '%s\n%s\n%s\n%s' "$(dg 5)" "$(dg 4)" "$(dg 3)" "$(dg 2)")" ]
}

@test "T5: only the deployed digest -> nothing to delete" {
  local input; input=$(rows 1)
  run --separate-stderr select_digests_to_delete "$(dg 1)" "$IMG" <<<"$input"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "T7: rows of other packages are ignored even when malformed" {
  # Given
  local input
  input=$(rows 5 4 3 2 | jq -c --arg img "$IMG-other" '. + [{package:$img, version:"d9"}, {package:$img, version:null}]')
  # When
  run --separate-stderr select_digests_to_delete "$(dg 5)" "$IMG" <<<"$input"
  # Then
  [ "$status" -eq 0 ]
  [ "$output" = "$(printf '%s\n%s\n%s' "$(dg 4)" "$(dg 3)" "$(dg 2)")" ]
}

@test "T8: duplicated digests are output once" {
  local input; input=$(rows 4 3 3 2 1)
  run --separate-stderr select_digests_to_delete "$(dg 4)" "$IMG" <<<"$input"
  [ "$status" -eq 0 ]
  [ "$output" = "$(printf '%s\n%s\n%s' "$(dg 3)" "$(dg 2)" "$(dg 1)")" ]
}

@test "T9: deployed digest missing from the list is an error" {
  local input; input=$(rows 3 2 1)
  run --separate-stderr select_digests_to_delete "$(dg 9)" "$IMG" <<<"$input"
  [ "$status" -ne 0 ]
  [ -z "$output" ]
  [[ "$stderr" == *"deployed digest $(dg 9) not found"* ]]
}

@test "T10: empty list is the same error as T9" {
  run --separate-stderr select_digests_to_delete "$(dg 1)" "$IMG" <<<"[]"
  [ "$status" -ne 0 ]
  [ -z "$output" ]
  [[ "$stderr" == *"not found"* ]]
}

@test "T11: non-JSON stdin is an error" {
  run --separate-stderr select_digests_to_delete "$(dg 1)" "$IMG" <<<"not-json"
  [ "$status" -ne 0 ]
  [ -z "$output" ]
}

# ---------------------------------------------------------------- assert_single_manifest

@test "T73: manifest without config is rejected" {
  run assert_single_manifest <"$BATS_TEST_DIRNAME/fixtures/manifest-no-config.json"
  [ "$status" -ne 0 ]
}

@test "T74: assert_single_manifest accepts Docker v2 and OCI manifests" {
  run assert_single_manifest <"$BATS_TEST_DIRNAME/fixtures/manifest-docker.json"
  [ "$status" -eq 0 ]
  run assert_single_manifest <"$BATS_TEST_DIRNAME/fixtures/manifest-oci.json"
  [ "$status" -eq 0 ]
}

# ---------------------------------------------------------------- main

# Runs the script as a subprocess so that exit codes and the stub log are observable.
run_script() { run --separate-stderr bash "$SCRIPT" "$@"; }

@test "T12: inspect -> list -> delete everything but the deployed digest, in order" {
  # Given
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4 3 2 1)
  # When
  run_script "$IMG" "$(dg 5)"
  # Then
  [ "$status" -eq 0 ]
  local expected
  expected=$(printf '%s\n' \
    "docker buildx imagetools inspect --raw $IMG@$(dg 5)" \
    "gcloud artifacts docker images list $IMG --format=json" \
    "gcloud artifacts docker images delete $IMG@$(dg 4) --delete-tags --quiet" \
    "gcloud artifacts docker images delete $IMG@$(dg 3) --delete-tags --quiet" \
    "gcloud artifacts docker images delete $IMG@$(dg 2) --delete-tags --quiet" \
    "gcloud artifacts docker images delete $IMG@$(dg 1) --delete-tags --quiet")
  [ "$(cat "$STUB_LOG")" = "$expected" ]
}

@test "T13: nothing to delete -> no delete call" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 2)
  run_script "$IMG" "$(dg 2)"
  [ "$status" -eq 0 ]
  [ "$(wc -l <"$STUB_LOG" | tr -d ' ')" = "2" ]
  [[ "$output" == *"nothing to delete"* ]]
}

@test "T15: missing arguments -> exit 2 with usage, no external call" {
  run_script
  [ "$status" -eq 2 ]
  [[ "$stderr" == *"Usage:"* ]]
  [ ! -s "$STUB_LOG" ]
  run_script "$IMG"
  [ "$status" -eq 2 ]
  [[ "$stderr" == *"Usage:"* ]]
  [ ! -s "$STUB_LOG" ]
}

@test "T17: malformed deployed digest -> exit 2, no external call" {
  local bad
  for bad in latest sha256:abc ""; do
    run_script "$IMG" "$bad"
    [ "$status" -eq 2 ]
    [[ "$stderr" == *"invalid digest"* ]]
    [ ! -s "$STUB_LOG" ]
  done
}

@test "T18: image with tag or digest -> exit 2, no external call" {
  local bad
  for bad in "$IMG:latest" "$IMG@$(dg 1)"; do
    run_script "$bad" "$(dg 1)"
    [ "$status" -eq 2 ]
    [[ "$stderr" == *"image must not include tag or digest"* ]]
    [ ! -s "$STUB_LOG" ]
  done
}

@test "T19: list failure -> no delete" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
  export GCLOUD_STUB_LIST_EXIT=1
  run_script "$IMG" "$(dg 5)"
  [ "$status" -ne 0 ]
  run ! grep -q "images delete" "$STUB_LOG"
}

@test "T20: deployed digest not in list -> no delete" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 3 2 1)
  run_script "$IMG" "$(dg 9)"
  [ "$status" -ne 0 ]
  run ! grep -q "images delete" "$STUB_LOG"
}

@test "T21: delete failure stops at the first failure" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4 3 2 1)
  export GCLOUD_STUB_FAIL_ON="$IMG@$(dg 4)"
  run_script "$IMG" "$(dg 5)"
  [ "$status" -ne 0 ]
  [ "$(grep -c "images delete" "$STUB_LOG")" = "1" ]
  grep -q "images delete $IMG@$(dg 4) " "$STUB_LOG"
}

@test "T53: invalid candidate version -> nothing is deleted" {
  local bad
  for bad in 'null' '""' '"sha256:abc"'; do
    : >"$STUB_LOG"
    export GCLOUD_STUB_LIST_JSON
    GCLOUD_STUB_LIST_JSON=$(rows 5 4 3 2 | jq -c --arg img "$IMG" --argjson v "$bad" '. + [{package:$img, version:$v}]')
    run_script "$IMG" "$(dg 5)"
    [ "$status" -ne 0 ]
    [[ "$stderr" == *"invalid version"* ]]
    run ! grep -q "images delete" "$STUB_LOG"
  done
}

assert_refused_before_list() {
  [ "$status" -ne 0 ]
  [[ "$stderr" == *"deployed image $IMG@$(dg 5) is an image index"* ]]
  [ "$(cat "$STUB_LOG")" = "docker buildx imagetools inspect --raw $IMG@$(dg 5)" ]
}

@test "T68: OCI image index -> nothing is deleted" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
  DOCKER_STUB_MANIFEST_JSON=$(fixture index-oci.json)
  run_script "$IMG" "$(dg 5)"
  assert_refused_before_list
}

@test "T69: Docker manifest list -> nothing is deleted" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
  DOCKER_STUB_MANIFEST_JSON=$(fixture index-docker.json)
  run_script "$IMG" "$(dg 5)"
  assert_refused_before_list
}

@test "T70: index without mediaType is still an index" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
  DOCKER_STUB_MANIFEST_JSON=$(fixture index-no-mediatype.json)
  run_script "$IMG" "$(dg 5)"
  assert_refused_before_list
}

@test "T71: undeterminable manifest -> fail closed (4 patterns)" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
  local pattern
  for pattern in exit1 empty notjson v1; do
    : >"$STUB_LOG"
    unset DOCKER_STUB_EXIT
    case "$pattern" in
      exit1) export DOCKER_STUB_EXIT=1 ;;
      empty) DOCKER_STUB_MANIFEST_JSON="" ;;
      notjson) DOCKER_STUB_MANIFEST_JSON="not-json" ;;
      v1) DOCKER_STUB_MANIFEST_JSON=$(fixture manifest-v1.json) ;;
    esac
    run_script "$IMG" "$(dg 5)"
    [ "$status" -ne 0 ]
    run ! grep -q "^gcloud" "$STUB_LOG"
  done
}

@test "T72: OCI single manifest proceeds to delete" {
  export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
  DOCKER_STUB_MANIFEST_JSON=$(fixture manifest-oci.json)
  run_script "$IMG" "$(dg 5)"
  [ "$status" -eq 0 ]
  [ "$(grep -c "images delete" "$STUB_LOG")" = "1" ]
  grep -q "images delete $IMG@$(dg 4) --delete-tags --quiet" "$STUB_LOG"
}

# ---------------------------------------------------------------- invariants

# Builds rows for $1 digests (digits 1..n) with the deployed digest at position $2
# (first|middle|last) and an optional duplicate of the last row ($3 = dup).
build_case() {
  local n=$1 pos=$2 dup=$3 i
  local seq=() deployed_idx
  for ((i = 1; i <= n; i++)); do seq+=("$i"); done
  case "$pos" in
    first) deployed_idx=1 ;;
    middle) deployed_idx=$(((n + 1) / 2)) ;;
    last) deployed_idx=$n ;;
  esac
  if [ "$dup" = dup ]; then seq+=("${seq[$((n - 1))]}"); fi
  CASE_DEPLOYED=$(dg "$deployed_idx")
  CASE_INPUT=$(rows "${seq[@]}")
}

@test "P1,P2,P3,P5: invariants of select_digests_to_delete over n x position x dup" {
  local n pos dup out
  for n in 1 2 3 4 5 6 7 8; do
    for pos in first middle last; do
      for dup in nodup dup; do
        build_case "$n" "$pos" "$dup"
        out=$(select_digests_to_delete "$CASE_DEPLOYED" "$IMG" <<<"$CASE_INPUT")
        # P1: deployed is never selected and output is a subset of the package versions
        # P2: output set == unique versions - {deployed}
        # P3: no duplicates and every element is well-formed
        jq -e -n --arg dep "$CASE_DEPLOYED" --argjson input "$CASE_INPUT" --arg out "$out" '
          ($out | split("\n") | map(select(. != ""))) as $o
          | ($input | map(.version) | unique) as $all
          | ($o | index($dep)) == null
          and ($o - $all | length) == 0
          and (($o | sort) == ($all - [$dep] | sort))
          and ($o == ($o | unique))
          and ($o | all(test("^sha256:[0-9a-f]{64}$")))' >/dev/null \
          || { echo "invariant violated: n=$n pos=$pos dup=$dup out=$out"; return 1; }
        # P5: idempotent - removing the selected rows and re-running selects nothing
        local rest
        rest=$(jq -c --arg out "$out" '($out | split("\n")) as $o | map(select(.version as $v | $o | index($v) | not))' <<<"$CASE_INPUT")
        out=$(select_digests_to_delete "$CASE_DEPLOYED" "$IMG" <<<"$rest")
        [ -z "$out" ] || { echo "not idempotent: n=$n pos=$pos dup=$dup"; return 1; }
      done
    done
  done
}

@test "P4: no delete call for every failure mode before deletion" {
  local mode
  for mode in noargs baddigest badimage index inspectfail listfail notfound badversion; do
    : >"$STUB_LOG"
    unset DOCKER_STUB_EXIT GCLOUD_STUB_LIST_EXIT
    DOCKER_STUB_MANIFEST_JSON=$(fixture manifest-docker.json)
    export GCLOUD_STUB_LIST_JSON; GCLOUD_STUB_LIST_JSON=$(rows 5 4)
    local args=("$IMG" "$(dg 5)")
    case "$mode" in
      noargs) args=() ;;
      baddigest) args=("$IMG" latest) ;;
      badimage) args=("$IMG:latest" "$(dg 5)") ;;
      index) DOCKER_STUB_MANIFEST_JSON=$(fixture index-oci.json) ;;
      inspectfail) export DOCKER_STUB_EXIT=1 ;;
      listfail) export GCLOUD_STUB_LIST_EXIT=1 ;;
      notfound) args=("$IMG" "$(dg 9)") ;;
      badversion) GCLOUD_STUB_LIST_JSON=$(rows 5 4 | jq -c --arg img "$IMG" '. + [{package:$img, version:null}]') ;;
    esac
    run_script "${args[@]}"
    [ "$status" -ne 0 ] || { echo "mode=$mode should fail"; return 1; }
    ! grep -q "images delete" "$STUB_LOG" || { echo "mode=$mode deleted"; return 1; }
  done
}
