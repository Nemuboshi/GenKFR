#!/usr/bin/env bash
set -euo pipefail

# Generate small videos with the system ffmpeg, then decode them with the ASan build.
: "${GENKFR_ASAN_BIN:?set GENKFR_ASAN_BIN to the instrumented genkfr executable}"
: "${RUNNER_TEMP:?}"
corpus="$RUNNER_TEMP/asan-corpus"
mkdir -p "$corpus"
ffmpeg -hide_banner -loglevel error -y -f lavfi \
  -i 'testsrc2=size=258x130:rate=5,noise=alls=100:allf=t+u:all_seed=42' \
  -frames:v 20 -c:v ffv1 "$corpus/noise.mkv"
ffmpeg -hide_banner -loglevel error -y -f lavfi \
  -i 'testsrc=size=65x49:rate=3' -frames:v 3 -pix_fmt yuv420p -c:v ffv1 "$corpus/odd.mkv"
ffmpeg -hide_banner -loglevel error -y -f lavfi \
  -i 'testsrc2=size=66x50:rate=3' -frames:v 3 -pix_fmt yuv420p10le -c:v ffv1 "$corpus/10bit.mkv"

ffprobe -v error -select_streams v -show_packets -show_entries packet=pos,size \
  -of json "$corpus/noise.mkv" > "$corpus/packets.json"
python3 - "$corpus" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
video = (root / 'noise.mkv').read_bytes()
packets = json.loads((root / 'packets.json').read_text())['packets']
assert len(packets) == 20
# Keep the complete container header but cut in the middle of a video packet.
packet = packets[10]
position, size = int(packet['pos']), int(packet['size'])
assert 0 < position < position + size <= len(video)
(root / 'truncated.mkv').write_bytes(video[:position + size // 2])
corrupt = bytearray(video)
for i in range(position + size // 4, position + size // 2):
    corrupt[i] ^= 0xff
(root / 'corrupted-packet.mkv').write_bytes(corrupt)
(root / 'header-only.mkv').write_bytes(video[:32])
PY

for name in noise odd 10bit; do
  "$GENKFR_ASAN_BIN" "$corpus/$name.mkv" > "$corpus/$name.log" 2>&1
done
for name in truncated corrupted-packet header-only; do
  status=0
  "$GENKFR_ASAN_BIN" "$corpus/$name.mkv" > "$corpus/$name.log" 2>&1 || status=$?
  # Broken media may decode partially or return an ordinary application error.
  # ASan uses exit code 86; a crash or sanitizer report must fail this check.
  [[ "$status" == 0 || "$status" == 1 ]]
  if [[ "$name" == header-only ]]; then
    [[ "$status" == 1 ]]
  fi
done
if grep -E 'AddressSanitizer|LeakSanitizer' "$corpus"/*.log; then
  echo 'sanitizer reported a memory error' >&2
  exit 1
fi
echo 'ASan media corpus passed'
