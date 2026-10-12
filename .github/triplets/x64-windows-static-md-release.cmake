# Same as vcpkg's x64-windows-static-md, but builds the release configuration
# only.
#
# vcpkg builds debug and release by default. GenKFR links the release static
# libraries only, so the debug build doubles the time for no benefit. FFmpeg
# takes about 15 minutes per configuration, so this halves the wait.

set(VCPKG_TARGET_ARCHITECTURE x64)
# Dynamic CRT, because that is what Rust uses. A static CRT fails to link.
set(VCPKG_CRT_LINKAGE dynamic)
set(VCPKG_LIBRARY_LINKAGE static)
set(VCPKG_BUILD_TYPE release)
