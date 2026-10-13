#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageIdentity {
    pub timestamp: u32,
    pub image_size: u32,
}
pub fn identity(headers: &[u8]) -> Option<ImageIdentity> {
    let u16_at = |offset: usize| {
        Some(u16::from_le_bytes(
            headers
                .get(offset..offset.checked_add(2)?)?
                .try_into()
                .ok()?,
        ))
    };
    let u32_at = |offset: usize| {
        Some(u32::from_le_bytes(
            headers
                .get(offset..offset.checked_add(4)?)?
                .try_into()
                .ok()?,
        ))
    };
    if headers.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(60)? as usize;
    if headers.get(pe..pe.checked_add(4)?)? != b"PE\0\0"
        || u16_at(pe.checked_add(4)?)? != 0x8664
        || u16_at(pe.checked_add(24)?)? != 0x20b
    {
        return None;
    }
    let result = ImageIdentity {
        timestamp: u32_at(pe.checked_add(8)?)?,
        image_size: u32_at(pe.checked_add(80)?)?,
    };
    (result.image_size >= 4096).then_some(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pe_headers_are_bounded_and_require_amd64_pe32_plus() {
        let mut h = vec![0; 512];
        h[..2].copy_from_slice(b"MZ");
        h[60..64].copy_from_slice(&128u32.to_le_bytes());
        h[128..132].copy_from_slice(b"PE\0\0");
        h[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        h[136..140].copy_from_slice(&0x6ab2b381u32.to_le_bytes());
        h[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        h[208..212].copy_from_slice(&0xe52000u32.to_le_bytes());
        assert_eq!(
            identity(&h),
            Some(ImageIdentity {
                timestamp: 0x6ab2b381,
                image_size: 0xe52000
            })
        );
        assert_eq!(identity(&h[..150]), None);
        h[132..134].copy_from_slice(&0x14cu16.to_le_bytes());
        assert_eq!(identity(&h), None);
        h[60..64].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(identity(&h), None);
    }
}
