// Match the Rust layouts in decode.rs against the vendored C header.
#include <cstddef>
#include "ffms.h"

static_assert(sizeof(void *) == 8, "GenKFR requires a 64-bit target");
static_assert(sizeof(FFMS_ErrorInfo) == 24 && alignof(FFMS_ErrorInfo) == 8);
static_assert(offsetof(FFMS_ErrorInfo, Buffer) == 16);
// Rust declares only these prefixes, never allocates or passes them for writes.
static_assert(offsetof(FFMS_Frame, EncodedWidth) == 48);
static_assert(offsetof(FFMS_Frame, RepeatPict) == 76);
static_assert(offsetof(FFMS_Frame, InterlacedFrame) == 80);
static_assert(alignof(FFMS_Frame) == 8);
static_assert(offsetof(FFMS_VideoProperties, TopFieldFirst) == 44);
static_assert(offsetof(FFMS_VideoProperties, ColorSpace) == 48);
static_assert(alignof(FFMS_VideoProperties) == 8);
