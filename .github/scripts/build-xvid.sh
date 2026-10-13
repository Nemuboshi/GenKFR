#!/usr/bin/env bash
set -euo pipefail

# Run after MSVC setup; versions are set by the calling workflow.
temp="$(cygpath -u "$RUNNER_TEMP")"
zip="$temp/nasm.zip"
dir="$temp/nasm"
url="https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VERSION/win64/nasm-$NASM_VERSION-win64.zip"
curl --fail --location --retry 3 --output "$zip" "$url"
7z x "$zip" "-o$(cygpath -w "$dir")" -y
NASM="$dir/nasm-$NASM_VERSION/nasm.exe"
[[ -f "$NASM" ]] || { echo 'nasm.exe is missing' >&2; exit 1; }
"$NASM" -v

root="$temp/xvidcore"
tag="release-${XVIDCORE_VERSION//./_}"
git clone --depth 1 --branch "$tag" https://github.com/arthenica/xvidcore.git "$root"
obj="$temp/xvid-obj"
mkdir -p "$obj"

# The C sources, from build/generic/sources.inc.
c=(
  decoder.c encoder.c xvid.c
  bitstream/bitstream.c bitstream/cbp.c bitstream/mbcoding.c
  dct/fdct.c dct/idct.c dct/simple_idct.c
  image/colorspace.c image/image.c image/interpolate8x8.c
  image/font.c image/postprocessing.c image/qpel.c image/reduced.c
  motion/estimation_bvop.c motion/estimation_common.c motion/estimation_gmc.c
  motion/estimation_pvop.c motion/estimation_rd_based.c
  motion/estimation_rd_based_bvop.c motion/gmc.c motion/motion_comp.c
  motion/vop_type_decision.c motion/sad.c
  prediction/mbprediction.c
  plugins/plugin_single.c plugins/plugin_2pass1.c plugins/plugin_2pass2.c
  plugins/plugin_lumimasking.c plugins/plugin_dump.c plugins/plugin_psnr.c
  plugins/plugin_ssim.c plugins/plugin_psnrhvsm.c
  quant/quant_h263.c quant/quant_matrix.c quant/quant_mpeg.c
  utils/emms.c utils/mbtransquant.c utils/mem_align.c
  utils/mem_transfer.c utils/timer.c
)
# Disable MSYS argument conversion so /O2 and /Fo reach MSVC unchanged.
export MSYS_NO_PATHCONV=1
cd "$root"
cp xvidcore/src/xvid.h "$temp/xvid.h"
for f in "${c[@]}"; do
  out="$obj/${f//\//_}"
  cl.exe /nologo /c /O2 /MD /DWIN32 /D_WINDOWS /DARCH_IS_X86_64 \
    /DARCH_IS_64BIT /D_CRT_SECURE_NO_WARNINGS /DNDEBUG \
    "/Fo$(cygpath -w "${out%.c}.obj")" "$(cygpath -w "xvidcore/src/$f")"
done

# The assembly, from the same sources file.
asm=(
  bitstream/x86_asm/cbp_mmx.asm bitstream/x86_asm/cbp_sse2.asm
  dct/x86_asm/fdct_mmx_ffmpeg.asm dct/x86_asm/fdct_mmx_skal.asm
  dct/x86_asm/fdct_sse2_skal.asm dct/x86_asm/idct_3dne.asm
  dct/x86_asm/idct_mmx.asm dct/x86_asm/idct_sse2_dmitry.asm
  image/x86_asm/colorspace_rgb_mmx.asm image/x86_asm/colorspace_yuv_mmx.asm
  image/x86_asm/colorspace_yuyv_mmx.asm image/x86_asm/interpolate8x8_3dn.asm
  image/x86_asm/interpolate8x8_3dne.asm image/x86_asm/interpolate8x8_mmx.asm
  image/x86_asm/interpolate8x8_xmm.asm image/x86_asm/postprocessing_mmx.asm
  image/x86_asm/postprocessing_sse2.asm image/x86_asm/reduced_mmx.asm
  image/x86_asm/qpel_mmx.asm image/x86_asm/gmc_mmx.asm
  image/x86_asm/deintl_sse.asm
  motion/x86_asm/sad_xmm.asm motion/x86_asm/sad_sse2.asm
  motion/x86_asm/sad_mmx.asm motion/x86_asm/sad_3dne.asm
  motion/x86_asm/sad_3dn.asm
  quant/x86_asm/quantize_h263_mmx.asm quant/x86_asm/quantize_h263_3dne.asm
  quant/x86_asm/quantize_mpeg_xmm.asm quant/x86_asm/quantize_mpeg_mmx.asm
  utils/x86_asm/mem_transfer_mmx.asm utils/x86_asm/mem_transfer_3dne.asm
  utils/x86_asm/interlacing_mmx.asm utils/x86_asm/cpuid.asm
  plugins/x86_asm/plugin_ssim-a.asm
)
for f in "${asm[@]}"; do
  src="xvidcore/src/$f"
  out="$obj/${f//\//_}"
  # WINDOWS selects the MS calling convention; NO_PREFIX omits '_'.
  "$NASM" "-I$(cygpath -m "$(dirname "$src")")/" "-I$(cygpath -m xvidcore/src)/" \
    -f win64 -DWINDOWS -DNO_PREFIX -DARCH_IS_X86_64 \
    "$(cygpath -w "$src")" -o "$(cygpath -w "${out%.asm}.obj")"
done

objects=()
for f in "$obj"/*.obj; do objects+=("$(cygpath -w "$f")"); done
lib.exe /nologo "/OUT:$(cygpath -w "$temp/xvidcore.lib")" "${objects[@]}"
echo 'xvidcore.lib built'
