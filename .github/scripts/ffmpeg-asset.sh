#!/usr/bin/env bash
set -euo pipefail

tag="$1"
error_file="$(mktemp)"
trap 'rm -f "$error_file"' EXIT

# A missing release is expected; authentication and network errors are not.
if assets="$(gh release view "$tag" --json assets --jq '.assets[].name' 2>"$error_file")"; then
  if grep -Fxq "$tag.zip" <<< "$assets"; then
    echo true
  else
    echo false
  fi
elif grep -Fq 'release not found' "$error_file"; then
  echo false
else
  cat "$error_file" >&2
  exit 1
fi
