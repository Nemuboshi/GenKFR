// Match the Rust layouts in src/xvid.rs against the header used to build xvid.
#include <stddef.h>
#include "xvid.h"

#define SIZE(T, S, A) _Static_assert(sizeof(T) == S && _Alignof(T) == A, #T " layout")
#define OFFSET(T, F, O) _Static_assert(offsetof(T, F) == O, #T "." #F " offset")
_Static_assert(sizeof(void *) == 8, "GenKFR requires a 64-bit target");
_Static_assert(XVID_VERSION == ((1 << 16) | (3 << 8) | 7), "Recheck encoder bounds when upgrading xvid");
SIZE(xvid_image_t, 56, 8);
OFFSET(xvid_image_t, plane, 8);
OFFSET(xvid_image_t, stride, 40);
SIZE(xvid_gbl_init_t, 12, 4);
SIZE(xvid_gbl_info_t, 24, 8);
OFFSET(xvid_gbl_info_t, build, 8);
OFFSET(xvid_gbl_info_t, num_threads, 20);
SIZE(xvid_enc_stats_t, 52, 4);
SIZE(xvid_enc_create_t, 128, 8);
OFFSET(xvid_enc_create_t, zones, 24);
OFFSET(xvid_enc_create_t, plugins, 40);
OFFSET(xvid_enc_create_t, min_quant, 84);
OFFSET(xvid_enc_create_t, handle, 112);
OFFSET(xvid_enc_create_t, num_slices, 124);
SIZE(xvid_enc_frame_t, 136, 8);
OFFSET(xvid_enc_frame_t, quant_intra_matrix, 8);
OFFSET(xvid_enc_frame_t, input, 48);
OFFSET(xvid_enc_frame_t, type, 104);
OFFSET(xvid_enc_frame_t, bitstream, 120);
OFFSET(xvid_enc_frame_t, out_flags, 132);
