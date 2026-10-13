#!/usr/bin/env bash
set -euo pipefail

helper="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/ffmpeg-asset.sh"
temp="$(mktemp -d)"
trap 'rm -rf "$temp"' EXIT
mkdir -p "$temp/bin"
cat > "$temp/bin/gh" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$MOCK_ASSETS"
printf '%s\n' "$MOCK_ERROR" >&2
exit "$MOCK_STATUS"
MOCK
chmod +x "$temp/bin/gh"
export PATH="$temp/bin:$PATH"
export MOCK_STATUS=0 MOCK_ASSETS='' MOCK_ERROR=''
tag=ffmpeg-libs-9.0.2

# Empty releases and unrelated assets must not skip the build.
[[ "$(bash "$helper" "$tag")" == false ]]
export MOCK_ASSETS=other.zip
[[ "$(bash "$helper" "$tag")" == false ]]
export MOCK_ASSETS=$'other.zip\nffmpeg-libs-9.0.2.zip'
[[ "$(bash "$helper" "$tag")" == true ]]

# A missing release is normal, despite gh's nonzero exit status.
export MOCK_STATUS=1 MOCK_ASSETS='' MOCK_ERROR='release not found'
[[ "$(bash "$helper" "$tag")" == false ]]

# Other failures must stop CI instead of being treated as a missing release.
for error in 'HTTP 401: Bad credentials' 'connection timed out'; do
  export MOCK_ERROR="$error"
  if bash "$helper" "$tag" > "$temp/output" 2> "$temp/error"; then
    echo "unexpected success for: $error" >&2
    exit 1
  fi
  [[ ! -s "$temp/output" ]]
  grep -Fq "$error" "$temp/error"
done

echo 'FFmpeg asset lookup: 6 cases passed'
