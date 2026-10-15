use core::ffi::c_void;
use gmod_dglab_common::pe::{self, ImageIdentity};
use std::io;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetCurrentProcess() -> *mut c_void;
    fn ReadProcessMemory(
        process: *mut c_void,
        address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> i32;
}

pub fn read<const N: usize>(address: usize) -> Option<[u8; N]> {
    address.checked_add(N)?;
    let mut bytes = [0u8; N];
    let mut count = 0usize;
    // SAFETY: Windows 负责检查源地址，不在 Rust 中解引用未经检查的原生指针。
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            N,
            &mut count,
        )
    };
    (ok != 0 && count == N).then_some(bytes)
}

// Wildcards are restricted to relocatable displacements; opcodes and ABI stay checked.
const ANY: i16 = -1;
const READ: u32 = 0x4000_0000;
const EXECUTE: u32 = 0x2000_0000;
const WRITE: u32 = 0x8000_0000;
const LOCAL_PLAYER: &[i16] = &[0x8b, 0x05, ANY, ANY, ANY, ANY, 0xff, 0xc0, 0xc3];
const IN_GAME: &[i16] = &[0x83, 0x3d, ANY, ANY, ANY, ANY, 6, 0x0f, 0x94, 0xc0, 0xc3];
const CLASSES: &[i16] = &[0x48, 0x8b, 0x05, ANY, ANY, ANY, ANY, 0xc3];
const FACTORY: &[i16] = &[0x48, 0x8d, 0x05, ANY, ANY, ANY, ANY, 0xc3];
const REGISTRATION: &[i16] = &[
    0x4c, 0x8d, 0x05, ANY, ANY, ANY, ANY, 0x48, 0x8d, 0x15, ANY, ANY, ANY, ANY, 0x48, 0x8d, 0x0d,
    ANY, ANY, ANY, ANY, 0xe9, ANY, ANY, ANY, ANY,
];
const REGISTER: &[i16] = &[
    0x4c, 0x89, 0x41, 8, 0x48, 0x89, 0x11, 0x48, 0x8b, 0x05, ANY, ANY, ANY, ANY, 0x48, 0x89, 0x41,
    0x10, 0x48, 0x8b, 0xc1, 0x48, 0x89, 0x0d, ANY, ANY, ANY, ANY, 0xc3,
];
const CREATE_INTERFACE: &[i16] = &[
    0x48, 0x89, 0x5c, 0x24, 8, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48,
    0x8b, 0x1d, ANY, ANY, ANY, ANY, 0x48, 0x8b, 0xfa, 0x48, 0x8b, 0xf1, 0x48, 0x85, 0xdb, 0x74,
    0x19,
];
const GET_ENTITY: &[i16] = &[
    0x81, 0xfa, 0xff, 0x3f, 0, 0, 0x77, 0x1d, 0x48, 0x63, 0xc2, 0x48, 0x2d, 1, 0x40, 0, 0, 0x48,
    0xc1, 0xe0, 5, 0x48, 0x8b, 0x0c, 8, 0x48, 0x85, 0xc9, 0x74, 7, 0x48, 0x8b, 1, 0x48, 0xff, 0x60,
    0x30, 0x33, 0xc0, 0xc3,
];
const FRAME: &[i16] = &[
    0x48, 0x83, 0xec, 0x28, 0x89, 0x15, ANY, ANY, ANY, ANY, 0x83, 0xfa, 6, 0x0f, 0x87, ANY, ANY,
    ANY, ANY, 0x48, 0x63, 0xc2, 0x48, 0x8d, 0x15, ANY, ANY, ANY, ANY, 0x8b, 0x8c, 0x82, ANY, ANY,
    ANY, ANY, 0x48, 3, 0xca, 0xff, 0xe1,
];
pub const BASE_PROXY: &[i16] = &[0x4c, 0x89, 2, 0xc3];
pub const INT_PROXY: &[i16] = &[0x8b, 0x41, 8, 0x41, 0x89, 0, 0xc3];
pub const BYTE_PROXY: &[i16] = &[0x0f, 0xb6, 0x41, 8, 0x41, 0x88, 0, 0xc3];

fn matches_code(bytes: &[u8], pattern: &[i16]) -> bool {
    bytes.len() >= pattern.len()
        && bytes
            .iter()
            .zip(pattern)
            .all(|(byte, expected)| *expected == ANY || *expected == i16::from(*byte))
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let slice = bytes
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or_else(|| invalid("PE 偏移溢出"))?,
        )
        .ok_or_else(|| invalid("PE 数据截断"))?;
    Ok(u32::from_le_bytes(
        slice.try_into().map_err(io::Error::other)?,
    ))
}
fn relative(address: usize, bytes: &[u8], displacement: usize, next: usize) -> io::Result<usize> {
    let delta = u32_at(bytes, displacement)? as i32 as isize;
    address
        .checked_add(next)
        .and_then(|value| value.checked_add_signed(delta))
        .ok_or_else(|| invalid("相对地址溢出"))
}
#[derive(Clone)]
struct Section {
    rva: usize,
    size: usize,
    flags: u32,
}
pub struct Module {
    base: usize,
    identity: ImageIdentity,
    sections: Vec<Section>,
    bytes: Vec<u8>,
    name: String,
}
struct Registration {
    object: usize,
    node: usize,
    factory: usize,
    name: usize,
}
impl Module {
    fn layout(name: &str, base: usize, headers: &[u8]) -> io::Result<Self> {
        let identity = pe::identity(headers).ok_or_else(|| invalid("模块不是有效 AMD64 PE32+"))?;
        if identity.image_size > 256 * 1024 * 1024
            || base.checked_add(identity.image_size as usize).is_none()
        {
            return Err(invalid("模块映像大小异常"));
        }
        let pe = u32_at(headers, 60)? as usize;
        let count = u32_at(headers, pe + 4)? >> 16;
        let optional_size = u32_at(headers, pe + 20)? & 0xffff;
        if !(1..=96).contains(&count) || optional_size < 240 {
            return Err(invalid("PE 节表无效"));
        }
        let table = pe + 24 + optional_size as usize;
        let mut sections: Vec<Section> = Vec::new();
        for i in 0..count as usize {
            let at = table + i * 40;
            let section = Section {
                rva: u32_at(headers, at + 12)? as usize,
                size: u32_at(headers, at + 8)? as usize,
                flags: u32_at(headers, at + 36)?,
            };
            let end = section
                .rva
                .checked_add(section.size)
                .filter(|end| *end <= identity.image_size as usize)
                .ok_or_else(|| invalid("PE 节越界"))?;
            if section.rva < headers.len()
                || sections
                    .iter()
                    .any(|other| section.rva < other.rva + other.size && other.rva < end)
            {
                return Err(invalid("PE 节重叠"));
            }
            sections.push(section);
        }
        let mut bytes = vec![0; identity.image_size as usize];
        bytes
            .get_mut(..headers.len())
            .ok_or_else(|| invalid("PE 头越界"))?
            .copy_from_slice(headers);
        Ok(Self {
            base,
            identity,
            sections,
            bytes,
            name: name.to_owned(),
        })
    }
    fn load(name: &str) -> io::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: Only query an already loaded module, without loading a disk copy.
        let base = unsafe { GetModuleHandleW(wide.as_ptr()) } as usize;
        if base == 0 {
            return Err(invalid(format!("{name} 尚未加载")));
        }
        let headers = read::<4096>(base).ok_or_else(|| invalid("模块头不可读"))?;
        let mut module = Self::layout(name, base, &headers)?;
        for section in &module.sections {
            // Discardable sections (e.g. relocations) may already be unmapped.
            if section.flags & READ == 0 || section.flags & 0x0200_0000 != 0 {
                continue;
            }
            let output = &mut module.bytes[section.rva..section.rva + section.size];
            let mut count = 0;
            // SAFETY: Windows validates the source; output is an owned, bounded buffer.
            let ok = unsafe {
                ReadProcessMemory(
                    GetCurrentProcess(),
                    (base + section.rva) as *const c_void,
                    output.as_mut_ptr().cast(),
                    output.len(),
                    &mut count,
                )
            };
            if ok == 0 || count != output.len() {
                return Err(invalid(format!("{name} 模块节不可读")));
            }
        }
        Ok(module)
    }
    fn at(&self, address: usize, length: usize, flags: u32) -> io::Result<&[u8]> {
        let rva = address
            .checked_sub(self.base)
            .ok_or_else(|| invalid("地址不在模块内"))?;
        let end = rva
            .checked_add(length)
            .ok_or_else(|| invalid("模块地址溢出"))?;
        if !self.sections.iter().any(|s| {
            s.rva <= rva
                && end <= s.rva + s.size
                && s.flags & (READ | flags) == READ | flags
                && s.flags & 0x0200_0000 == 0
        }) {
            return Err(invalid(format!(
                "{} 地址 0x{rva:x} 不在所需类型的节内",
                self.name
            )));
        }
        self.bytes
            .get(rva..end)
            .ok_or_else(|| invalid("模块地址越界"))
    }
    fn pointer(&self, address: usize) -> io::Result<usize> {
        Ok(usize::from_le_bytes(
            self.at(address, 8, 0)?
                .try_into()
                .map_err(io::Error::other)?,
        ))
    }
    pub fn code(&self, address: usize, pattern: &[i16]) -> io::Result<()> {
        let bytes = self.at(address, pattern.len(), EXECUTE)?;
        if !matches_code(bytes, pattern) {
            return Err(invalid(format!(
                "{} RVA 0x{:x} 函数特征不符",
                self.name,
                address - self.base
            )));
        }
        Ok(())
    }
    fn export(&self, name: &[u8]) -> io::Result<usize> {
        let pe = u32_at(&self.bytes, 60)? as usize;
        let rva = u32_at(&self.bytes, pe + 24 + 112)? as usize;
        let data = self.at(self.base + rva, 40, 0)?;
        let count = u32_at(data, 24)? as usize;
        if count > 65536 {
            return Err(invalid("导出表数量异常"));
        }
        let functions = self.base + u32_at(data, 28)? as usize;
        let names = self.base + u32_at(data, 32)? as usize;
        let ordinals = self.base + u32_at(data, 36)? as usize;
        for i in 0..count {
            let string = self.base + u32_at(self.at(names + i * 4, 4, 0)?, 0)? as usize;
            if self.at(string, name.len(), 0)? == name {
                let index = u16::from_le_bytes(
                    self.at(ordinals + i * 2, 2, 0)?
                        .try_into()
                        .map_err(io::Error::other)?,
                ) as usize;
                if index >= u32_at(data, 20)? as usize {
                    return Err(invalid("导出序号越界"));
                }
                return Ok(self.base + u32_at(self.at(functions + index * 4, 4, 0)?, 0)? as usize);
            }
        }
        Err(invalid("缺少 CreateInterface 导出"))
    }
    fn registration(&self, name: &str) -> io::Result<Registration> {
        let exported = self.export(b"CreateInterface\0")?;
        self.code(exported, CREATE_INTERFACE)?;
        let head = relative(exported, self.at(exported, 22, EXECUTE)?, 18, 22)?;
        self.at(head, 8, WRITE)?;
        let wanted: Vec<u8> = name.bytes().chain(Some(0)).collect();
        let mut found = None;
        for section in self
            .sections
            .iter()
            .filter(|s| s.flags & (READ | EXECUTE) == READ | EXECUTE)
        {
            for (offset, bytes) in self.bytes[section.rva..section.rva + section.size]
                .windows(REGISTRATION.len())
                .enumerate()
            {
                if !matches_code(bytes, REGISTRATION) {
                    continue;
                }
                let address = self.base + section.rva + offset;
                let string = relative(address, bytes, 3, 7)?;
                if self.at(string, wanted.len(), 0).ok() != Some(wanted.as_slice()) {
                    continue;
                }
                if found.is_some() {
                    return Err(invalid(format!("接口 {name} 的注册特征不唯一")));
                }
                let factory = relative(address, bytes, 10, 14)?;
                let node = relative(address, bytes, 17, 21)?;
                let register = relative(address, bytes, 22, 26)?;
                self.code(register, REGISTER)?;
                let code = self.at(register, REGISTER.len(), EXECUTE)?;
                if relative(register, code, 10, 14)? != head
                    || relative(register, code, 24, 28)? != head
                {
                    return Err(invalid(format!("接口 {name} 注册链表与导出不一致")));
                }
                self.code(factory, FACTORY)?;
                let object = relative(factory, self.at(factory, 8, EXECUTE)?, 3, 7)?;
                self.at(object, 8, WRITE)?;
                self.at(node, 24, WRITE)?;
                found = Some(Registration {
                    object,
                    node,
                    factory,
                    name: string,
                });
            }
        }
        found.ok_or_else(|| invalid(format!("未找到接口 {name} 的注册特征")))
    }
    fn object(&self, name: &str) -> io::Result<usize> {
        let registration = self.registration(name)?;
        if self.pointer(registration.node)? != registration.factory
            || self.pointer(registration.node + 8)? != registration.name
        {
            return Err(invalid(format!("接口 {name} 尚未初始化或注册记录不符")));
        }
        let table = self.pointer(registration.object)?;
        self.at(table, 8, 0)?;
        Ok(registration.object)
    }
    fn method(&self, object: usize, index: usize, pattern: &[i16]) -> io::Result<(usize, usize)> {
        self.at(object, 8, WRITE)?;
        let table = self.pointer(object)?;
        if table % 8 != 0 {
            return Err(invalid("虚表未对齐"));
        }
        let slot = table
            .checked_add(index.checked_mul(8).ok_or_else(|| invalid("虚表槽溢出"))?)
            .ok_or_else(|| invalid("虚表地址溢出"))?;
        let function = self.pointer(slot)?;
        self.code(function, pattern)
            .map_err(|e| invalid(format!("{} 接口虚表槽 {index}：{e}", self.name)))?;
        Ok((slot, function))
    }
    fn writable_reference(
        &self,
        function: usize,
        displacement: usize,
        next: usize,
        size: usize,
    ) -> io::Result<usize> {
        let target = relative(
            function,
            self.at(function, next, EXECUTE)?,
            displacement,
            next,
        )?;
        self.at(target, size, WRITE)?;
        if !target.is_multiple_of(size) {
            return Err(invalid("全局数据未对齐"));
        }
        Ok(target)
    }
    fn frame(&self, object: usize) -> io::Result<(usize, usize)> {
        let (slot, function) = self.method(object, 35, FRAME)?;
        self.writable_reference(function, 6, 10, 4)?;
        let bytes = self.at(function, FRAME.len(), EXECUTE)?;
        if relative(function, bytes, 25, 29)? != self.base {
            return Err(invalid("帧回调跳转基址不符"));
        }
        let default = relative(function, bytes, 15, 19)?;
        self.code(default, &[0x48, 0x83, 0xc4, 0x28, 0xc3])?;
        let table = self.base + u32_at(bytes, 32)? as usize;
        for stage in 0..7 {
            let target = self.base + u32_at(self.at(table + stage * 4, 4, EXECUTE)?, 0)? as usize;
            self.at(target, 1, EXECUTE)?;
        }
        Ok((slot, function))
    }
}
pub struct Resolved {
    pub engine: usize,
    pub entities: usize,
    pub local_fn: usize,
    pub in_game_fn: usize,
    pub entity_fn: usize,
    pub frame_slot: usize,
    pub frame_fn: usize,
    pub offsets: crate::netvars::PlayerOffsets,
}
pub fn inspect() -> io::Result<(String, Resolved)> {
    let engine = Module::load("engine.dll")?;
    let client = Module::load("client.dll")?;
    let engine_object = engine.object("VEngineClient015")?;
    let client_object = client.object("VClient017")?;
    let entities = client.object("VClientEntityList003")?;
    let (_, local_fn) = engine.method(engine_object, 12, LOCAL_PLAYER)?;
    engine.writable_reference(local_fn, 2, 6, 4)?;
    let (_, in_game_fn) = engine.method(engine_object, 26, IN_GAME)?;
    engine.writable_reference(in_game_fn, 2, 7, 4)?;
    let (_, entity_fn) = client.method(entities, 3, GET_ENTITY)?;
    let (_, classes_fn) = client.method(client_object, 8, CLASSES)?;
    let classes = client.writable_reference(classes_fn, 3, 7, 8)?;
    let (frame_slot, frame_fn) = client.frame(client_object)?;
    let offsets = crate::netvars::resolve_player(classes, &client)?;
    let properties = match crate::netvars::inspect(classes) {
        Ok(report) => report,
        Err(error) => format!("附加属性诊断未完成：{error}"),
    };
    let report = format!(
        "特征兼容性检查通过（时间戳仅作记录，不作为接入限制）\nengine.dll：时间戳 {:08x}，映像大小 0x{:x}，接口 RVA 0x{:x}\nclient.dll：时间戳 {:08x}，映像大小 0x{:x}，接口 RVA 0x{:x}，实体列表 RVA 0x{:x}\n函数 RVA：本地玩家 0x{:x}，连接状态 0x{:x}，实体访问 0x{:x}，类列表 0x{:x}，帧回调 0x{:x}\n{properties}",
        engine.identity.timestamp, engine.identity.image_size, engine_object - engine.base,
        client.identity.timestamp, client.identity.image_size, client_object - client.base, entities - client.base,
        local_fn - engine.base, in_game_fn - engine.base, entity_fn - client.base, classes_fn - client.base, frame_fn - client.base,
    );
    Ok((
        report,
        Resolved {
            engine: engine_object,
            entities,
            local_fn,
            in_game_fn,
            entity_fn,
            frame_slot,
            frame_fn,
            offsets,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_addresses_may_move_but_instructions_must_match() {
        let bytes = [0x8b, 0x05, 0xff, 0xff, 0xff, 0xff, 0xff, 0xc0, 0xc3];
        assert!(matches_code(&bytes, LOCAL_PLAYER));
        let mut changed = bytes;
        changed[7] = 0xc8;
        assert!(!matches_code(&changed, LOCAL_PLAYER));
        assert!(!matches_code(&bytes[..8], LOCAL_PLAYER));
        assert_eq!(
            relative(0x1000, &[0xfc, 0xff, 0xff, 0xff], 0, 4).unwrap(),
            0x1000
        );
        assert!(relative(0, &[0xf0, 0xff, 0xff, 0xff], 0, 4).is_err());
        assert!(relative(usize::MAX, &[0; 4], 0, 4).is_err());
    }

    // Map bytes only: no LoadLibrary, DllMain, engine calls, or fake engine state.
    fn disk_image(path: &std::path::Path, timestamp: Option<u32>) -> Module {
        let mut disk = std::fs::read(path).expect("read actual game DLL");
        let pe = u32_at(&disk, 60).unwrap() as usize;
        if let Some(timestamp) = timestamp {
            disk[pe + 8..pe + 12].copy_from_slice(&timestamp.to_le_bytes());
        }
        let base = u64::from_le_bytes(disk[pe + 48..pe + 56].try_into().unwrap()) as usize;
        let mut module = Module::layout(
            path.file_name().unwrap().to_str().unwrap(),
            base,
            &disk[..4096],
        )
        .unwrap();
        let table = pe + 24 + (u32_at(&disk, pe + 20).unwrap() & 0xffff) as usize;
        for (i, section) in module.sections.iter().enumerate() {
            let raw_size = u32_at(&disk, table + i * 40 + 16).unwrap() as usize;
            let raw = u32_at(&disk, table + i * 40 + 20).unwrap() as usize;
            let size = raw_size.min(section.size);
            module.bytes[section.rva..section.rva + size].copy_from_slice(&disk[raw..raw + size]);
        }
        module
    }

    #[test]
    #[ignore = "设置 GMOD_BIN 为实际游戏 bin/win64 路径后运行，不加载或执行游戏 DLL"]
    fn installed_game_binary_features_ignore_timestamp() {
        let folder = std::path::PathBuf::from(std::env::var_os("GMOD_BIN").expect("GMOD_BIN"));
        let engine = disk_image(&folder.join("engine.dll"), None);
        let changed = disk_image(&folder.join("engine.dll"), Some(0));
        let object = engine.registration("VEngineClient015").unwrap().object;
        assert_eq!(
            changed.registration("VEngineClient015").unwrap().object,
            object
        );
        let (_, local) = engine.method(object, 12, LOCAL_PLAYER).unwrap();
        engine.writable_reference(local, 2, 6, 4).unwrap();
        let (_, connected) = engine.method(object, 26, IN_GAME).unwrap();
        engine.writable_reference(connected, 2, 7, 4).unwrap();
        let mut client = disk_image(&folder.join("client.dll"), Some(u32::MAX));
        let object = client.registration("VClient017").unwrap().object;
        let entities = client.registration("VClientEntityList003").unwrap().object;
        let (_, classes) = client.method(object, 8, CLASSES).unwrap();
        let classes_global = client.writable_reference(classes, 3, 7, 8).unwrap();
        let (_, frame) = client.frame(object).unwrap();
        let matches: Vec<_> = client
            .sections
            .iter()
            .filter(|s| s.flags & EXECUTE != 0)
            .flat_map(|s| {
                client.bytes[s.rva..s.rva + s.size]
                    .windows(GET_ENTITY.len())
                    .enumerate()
                    .filter_map(|(i, bytes)| matches_code(bytes, GET_ENTITY).then_some(s.rva + i))
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "entity access instruction feature must be unique in installed image"
        );
        println!("static features: local={:x}, in_game={:x}, classes={:x}, classes_global={:x}, frame={:x}, entities={:x}, get_entity={:x}",
            local-engine.base, connected-engine.base, classes-client.base, classes_global-client.base, frame-client.base, entities-client.base, matches[0]);
        // An instruction change must still fail, even when metadata is accepted.
        client.bytes[frame - client.base] = 0xcc;
        assert!(client.frame(object).is_err());
    }

    #[test]
    fn unreadable_pointer_is_an_error_not_a_dereference() {
        assert_eq!(read::<8>(1), None);
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(read::<8>(data.as_ptr() as usize), Some(data));
    }
}
