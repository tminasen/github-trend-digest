#!/usr/bin/env bash
# Deletes every image version in an Artifact Registry package except the one
# that was just deployed.
#
# Usage: cleanup-old-images.sh <image> <deployed_digest>
#   image            e.g. asia-northeast1-docker.pkg.dev/<project>/<repo>/github-trend-digest
#                    (no tag, no digest)
#   deployed_digest  sha256:<64 hex>  (the digest build-push-action produced)
#
# Safety net: nothing is deleted unless the deployed digest is a single
# manifest (not an image index). Children of an index are separate versions in
# Artifact Registry, and deleting them would break the deployed image.
#
# The functions can be sourced (tests do so); main runs only when executed.

DIGEST_RE='^sha256:[0-9a-f]{64}$'

# select_digests_to_delete <deployed_digest> <image>
# stdin : JSON from `gcloud artifacts docker images list <image> --format=json`
# stdout: every digest of <image> except <deployed_digest>, one per line,
#         deduplicated, in the order of first appearance.
# Fails (with empty stdout) when the input is not JSON, when any version of
# <image> is malformed, or when <deployed_digest> is not in the list.
select_digests_to_delete() {
  local deployed=$1 image=$2 selected
  selected=$(jq -r --arg image "$image" --arg deployed "$deployed" --arg re "$DIGEST_RE" '
    [.[] | select(.package == $image) | .version] as $versions
    | if ($versions | all(type == "string" and test($re))) | not
      then error("invalid version found in package \($image)")
      else . end
    | if ($versions | index($deployed)) == null
      then error("deployed digest \($deployed) not found in package \($image)")
      else . end
    | reduce $versions[] as $v ([]; if any(.[]; . == $v) then . else . + [$v] end)
    | .[] | select(. != $deployed)
  ') || return 1
  if [ -n "$selected" ]; then
    printf '%s\n' "$selected"
  fi
}

# assert_single_manifest
# stdin: manifest JSON (`docker buildx imagetools inspect --raw`)
# Succeeds only for a single Docker v2 / OCI image manifest: allowed mediaType,
# has `config`, has no `manifests`. Anything else (index, unknown, not JSON) fails.
assert_single_manifest() {
  jq -e '
    type == "object"
    and ((.mediaType // "") | IN("application/vnd.oci.image.manifest.v1+json", "application/vnd.docker.distribution.manifest.v2+json"))
    and ((.config // null) | type == "object")
    and (has("manifests") | not)
  ' >/dev/null 2>&1 || {
    echo "error: manifest is not a single image manifest" >&2
    return 1
  }
}

main() {
  if [ "$#" -ne 2 ]; then
    echo "Usage: cleanup-old-images.sh <image> <deployed_digest>" >&2
    return 2
  fi
  local image=$1 deployed=$2

  if ! [[ "$deployed" =~ $DIGEST_RE ]]; then
    echo "error: invalid digest: '$deployed'" >&2
    return 2
  fi
  case "${image##*/}" in
    *:* | *@*)
      echo "error: image must not include tag or digest: '$image'" >&2
      return 2
      ;;
  esac

  local manifest
  manifest=$(docker buildx imagetools inspect --raw "$image@$deployed") || {
    echo "error: cannot inspect $image@$deployed; refusing to delete" >&2
    return 1
  }
  printf '%s' "$manifest" | assert_single_manifest 2>/dev/null || {
    echo "error: deployed image $image@$deployed is an image index or its manifest type could not be verified; refusing to delete" >&2
    return 1
  }

  local list_json candidates
  list_json=$(gcloud artifacts docker images list "$image" --format=json) || return 1
  candidates=$(select_digests_to_delete "$deployed" "$image" <<<"$list_json") || return 1

  if [ -z "$candidates" ]; then
    echo "nothing to delete"
    return 0
  fi

  local digest
  while IFS= read -r digest; do
    echo "deleting $image@$digest"
    gcloud artifacts docker images delete "$image@$digest" --delete-tags --quiet || return 1
  done <<<"$candidates"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -euo pipefail
  main "$@"
fi
