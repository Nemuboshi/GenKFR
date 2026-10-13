#!/usr/bin/env bash
set -euo pipefail

# A deliberate overflow must be caught both in Rust and across the C FFI boundary.
: "${RUNNER_TEMP:?}"
: "${RUSTFLAGS:?}"
probe="$RUNNER_TEMP/asan-runtime"
mkdir -p "$probe"
cat > "$probe/read.c" <<'C'
unsigned char native_read(const unsigned char *bytes) {
    return ((const volatile unsigned char *)bytes)[8];
}
C
cat > "$probe/read.rs" <<'RS'
unsafe extern "C" {
    fn native_read(bytes: *const u8) -> u8;
}
fn main() {
    let bytes = vec![7u8; 8];
    let value = if std::env::args().nth(1).as_deref() == Some("native") {
        unsafe { native_read(bytes.as_ptr()) }
    } else {
        unsafe { std::ptr::read_volatile(bytes.as_ptr().add(8)) }
    };
    println!("{value}");
}
RS
clang -g -O1 -fsanitize=address -fno-omit-frame-pointer -c "$probe/read.c" -o "$probe/read.o"
read -r -a flags <<< "$RUSTFLAGS"
rustc +nightly --edition=2024 --target x86_64-unknown-linux-gnu "${flags[@]}" \
  -C debuginfo=2 -C "link-arg=$probe/read.o" "$probe/read.rs" -o "$probe/read"
for language in rust native; do
  status=0
  "$probe/read" "$language" > "$probe/$language.log" 2>&1 || status=$?
  [[ "$status" == 86 ]]
  grep -q 'ERROR: AddressSanitizer: heap-buffer-overflow' "$probe/$language.log"
done
echo 'ASan detects Rust and native accesses to Rust allocations'
