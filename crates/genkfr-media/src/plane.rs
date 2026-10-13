use anyhow::{Result, bail};

/// Validate a positive 8-bit plane layout before passing it across FFI.
pub(crate) fn plane_len(stride: i32, width: i32, height: i32) -> Result<usize> {
    if width <= 0 || height <= 0 || stride < width {
        bail!("invalid plane layout: {width}x{height}, stride {stride}");
    }
    (stride as usize)
        .checked_mul(height as usize)
        .filter(|&len| len <= isize::MAX as usize)
        .ok_or_else(|| anyhow::anyhow!("plane size exceeds the addressable range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_tight_and_padded_rows() {
        assert_eq!(plane_len(4, 4, 2).unwrap(), 8);
        assert_eq!(plane_len(8, 4, 2).unwrap(), 16);
    }

    #[test]
    fn rejects_invalid_layouts() {
        for (stride, width, height) in [
            (0, 4, 2),
            (-4, 4, 2),
            (3, 4, 2),
            (4, 0, 2),
            (4, 4, 0),
            (4, 4, -1),
        ] {
            assert!(plane_len(stride, width, height).is_err());
        }
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn rejects_unaddressable_planes() {
        assert!(plane_len(i32::MAX, 4, 2).is_err());
    }
}
