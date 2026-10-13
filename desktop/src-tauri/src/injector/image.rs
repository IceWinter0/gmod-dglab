use gmod_dglab_common::pe;
fn u16_at(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(p..p.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32_at(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(p..p.checked_add(4)?)?.try_into().ok()?,
    ))
}
struct Image<'a> {
    headers: &'a [u8],
    sections: usize,
    count: usize,
    size: u32,
    header_size: u32,
    exports: u32,
    export_size: u32,
}
impl<'a> Image<'a> {
    fn parse(headers: &'a [u8]) -> Option<Self> {
        let identity = pe::identity(headers)?;
        let nt = u32_at(headers, 60)? as usize;
        let count = u16_at(headers, nt + 6)? as usize;
        let optional = u16_at(headers, nt + 20)? as usize;
        if count == 0
            || count > 96
            || optional < 120
            || u16_at(headers, nt + 22)? & 0x2000 == 0
            || u32_at(headers, nt + 132)? == 0
        {
            return None;
        }
        let sections = nt.checked_add(24 + optional)?;
        headers.get(sections..sections.checked_add(count * 40)?)?;
        let exports = u32_at(headers, nt + 136)?;
        let export_size = u32_at(headers, nt + 140)?;
        if exports == 0
            || export_size < 40
            || exports.checked_add(export_size)? > identity.image_size
        {
            return None;
        }
        Some(Self {
            headers,
            sections,
            count,
            size: identity.image_size,
            header_size: u32_at(headers, nt + 84)?,
            exports,
            export_size,
        })
    }
    fn offset(&self, rva: u32, len: usize) -> Option<usize> {
        if rva.checked_add(u32::try_from(len).ok()?)? > self.size {
            return None;
        }
        if rva < self.header_size {
            return Some(rva as usize);
        }
        for i in 0..self.count {
            let p = self.sections + i * 40;
            let va = u32_at(self.headers, p + 12)?;
            let raw_size = u32_at(self.headers, p + 16)?;
            let raw = u32_at(self.headers, p + 20)?;
            if rva >= va {
                let delta = rva - va;
                if delta.checked_add(u32::try_from(len).ok()?)? <= raw_size {
                    return (raw as usize).checked_add(delta as usize);
                }
            }
        }
        None
    }
    fn executable(&self, rva: u32) -> bool {
        (0..self.count).any(|i| {
            let p = self.sections + i * 40;
            let va = u32_at(self.headers, p + 12).unwrap_or(0);
            let size = u32_at(self.headers, p + 8)
                .unwrap_or(0)
                .max(u32_at(self.headers, p + 16).unwrap_or(0));
            rva >= va
                && rva.checked_sub(va).is_some_and(|offset| offset < size)
                && u32_at(self.headers, p + 36).is_some_and(|flags| flags & 0x20000000 != 0)
        })
    }
}
pub fn has_collector_exports(
    headers: &[u8],
    mut read: impl FnMut(u32, usize) -> Option<Vec<u8>>,
) -> bool {
    let Some(image) = Image::parse(headers) else {
        return false;
    };
    let check = |rva: u32, len: usize| {
        rva.checked_add(u32::try_from(len).unwrap_or(u32::MAX))
            .is_some_and(|end| end <= image.size)
    };
    let mut get = |rva: u32, len: usize| {
        if check(rva, len) {
            read(rva, len).filter(|v| v.len() == len)
        } else {
            None
        }
    };
    let result = (|| -> Option<bool> {
        let export = get(image.exports, 40)?;
        let functions = u32_at(&export, 20)? as usize;
        let names = u32_at(&export, 24)? as usize;
        if functions == 0 || functions > 16384 || !(2..=4096).contains(&names) {
            return None;
        }
        let f = get(u32_at(&export, 28)?, functions * 4)?;
        let n = get(u32_at(&export, 32)?, names * 4)?;
        let o = get(u32_at(&export, 36)?, names * 2)?;
        let mut found = [false; 2];
        for i in 0..names {
            let name_rva = u32_at(&n, i * 4)?;
            let bytes = get(name_rva, 64)?;
            let end = bytes.iter().position(|b| *b == 0)?;
            let name = &bytes[..end];
            if let Some(index) = [
                b"GmodDglabStatus".as_slice(),
                b"GmodDglabRequestStop".as_slice(),
            ]
            .iter()
            .position(|needle| name == *needle)
            {
                let ordinal = u16_at(&o, i * 2)? as usize;
                if ordinal >= functions {
                    return None;
                }
                let rva = u32_at(&f, ordinal * 4)?;
                if rva == 0
                    || rva >= image.size
                    || (rva >= image.exports && rva < image.exports + image.export_size)
                    || !image.executable(rva)
                {
                    return None;
                }
                found[index] = true;
            }
        }
        Some(found.iter().all(|found| *found))
    })();
    result.unwrap_or(false)
}
pub fn validate_collector(bytes: &[u8]) -> Result<(), String> {
    let image = Image::parse(bytes).ok_or("采样 DLL 不是有效的 Windows x64 DLL")?;
    if has_collector_exports(bytes, |rva, len| {
        let offset = image.offset(rva, len)?;
        Some(bytes.get(offset..offset.checked_add(len)?)?.to_vec())
    }) {
        Ok(())
    } else {
        Err("采样 DLL 缺少有效的项目导出标识".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    pub fn fixture() -> Vec<u8> {
        let mut b = vec![0u8; 2048];
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&128u32.to_le_bytes());
        b[128..132].copy_from_slice(b"PE\0\0");
        fn w16(b: &mut [u8], p: usize, v: u16) {
            b[p..p + 2].copy_from_slice(&v.to_le_bytes());
        }
        fn w32(b: &mut [u8], p: usize, v: u32) {
            b[p..p + 4].copy_from_slice(&v.to_le_bytes());
        }
        w16(&mut b, 132, 0x8664);
        w16(&mut b, 134, 2);
        w16(&mut b, 148, 240);
        w16(&mut b, 150, 0x2000);
        w16(&mut b, 152, 0x20b);
        w32(&mut b, 208, 0x4000);
        w32(&mut b, 212, 0x400);
        w32(&mut b, 260, 16);
        w32(&mut b, 264, 0x2000);
        w32(&mut b, 268, 0x100);
        for (p, va, raw, size, flags) in [
            (392, 0x1000, 0x400, 0x100, 0x60000020),
            (432, 0x2000, 0x500, 0x300, 0x40000040),
        ] {
            w32(&mut b, p + 8, size);
            w32(&mut b, p + 12, va);
            w32(&mut b, p + 16, size);
            w32(&mut b, p + 20, raw);
            w32(&mut b, p + 36, flags);
        }
        w32(&mut b, 0x500 + 20, 2);
        w32(&mut b, 0x500 + 24, 2);
        w32(&mut b, 0x500 + 28, 0x2028);
        w32(&mut b, 0x500 + 32, 0x2030);
        w32(&mut b, 0x500 + 36, 0x2038);
        w32(&mut b, 0x528, 0x1010);
        w32(&mut b, 0x52c, 0x1020);
        w32(&mut b, 0x530, 0x2040);
        w32(&mut b, 0x534, 0x2080);
        w16(&mut b, 0x538, 0);
        w16(&mut b, 0x53a, 1);
        let a = b"GmodDglabStatus\0";
        b[0x540..0x540 + a.len()].copy_from_slice(a);
        let a = b"GmodDglabRequestStop\0";
        b[0x580..0x580 + a.len()].copy_from_slice(a);
        b
    }
    #[test]
    fn collector_identity_and_export_boundaries() {
        let b = fixture();
        assert!(validate_collector(&b).is_ok());
        assert!(validate_collector(&b[..160]).is_err());
        let mut bad = b.clone();
        bad[132..134].copy_from_slice(&0x14cu16.to_le_bytes());
        assert!(validate_collector(&bad).is_err());
        let mut bad = b.clone();
        bad[0x518..0x51c].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_collector(&bad).is_err());
        let mut bad = b.clone();
        bad[0x528..0x52c].copy_from_slice(&0x2000u32.to_le_bytes());
        assert!(validate_collector(&bad).is_err());
    }

    #[test]
    #[ignore = "先构建 target/injector-collector/release 的采样 DLL"]
    fn built_collector_exports_are_valid() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/injector-collector/release/gmod_dglab_collector.dll");
        let bytes = std::fs::read(path).expect("release collector built");
        validate_collector(&bytes).expect("actual DLL accepted by injector");
    }
}
