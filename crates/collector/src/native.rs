use core::ffi::{c_char, c_void};
use gmod_dglab_common::pe::{self, ImageIdentity};
use std::io;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
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

fn module(
    name: &str,
    expected: ImageIdentity,
    factory_rva: usize,
    interfaces: &[(&str, usize)],
) -> io::Result<String> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: 模块名是有效的以零结尾 UTF-16；不加载新的游戏 DLL。
    let handle = unsafe { GetModuleHandleW(wide.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::other(format!("尚未加载 {name}")));
    }
    let base = handle as usize;
    let headers = read::<4096>(base).ok_or_else(|| io::Error::other("模块头不可读"))?;
    let actual =
        pe::identity(&headers).ok_or_else(|| io::Error::other("模块不是有效 AMD64 PE32+"))?;
    if actual != expected {
        return Err(io::Error::other(format!(
            "{name} 版本尚未适配，时间戳 {:08x}，映像大小 {:x}",
            actual.timestamp, actual.image_size
        )));
    }
    // SAFETY: 此名称是静态零结尾字符串，返回地址必须与已经反汇编的目标一致。
    let factory = unsafe { GetProcAddress(handle, c"CreateInterface".as_ptr().cast()) };
    if factory as usize != base + factory_rva {
        return Err(io::Error::other("CreateInterface 与已核对版本不符"));
    }
    let create: unsafe extern "C" fn(*const c_char, *mut i32) -> *mut c_void =
        unsafe { std::mem::transmute(factory) };
    let mut report = format!("{name}：版本身份匹配");
    for (interface, object_rva) in interfaces {
        let name = std::ffi::CString::new(*interface).map_err(io::Error::other)?;
        let mut code = -1;
        // SAFETY: 当前版本工厂 ABI 已通过静态代码核对，仅取得对象，不调用虚函数。
        let object = unsafe { create(name.as_ptr(), &mut code) } as usize;
        if code != 0 || object != base + object_rva {
            return Err(io::Error::other(format!(
                "接口 {interface} 对象与已核对版本不符"
            )));
        }
        let vtable = usize::from_le_bytes(
            read::<8>(object).ok_or_else(|| io::Error::other("接口虚表不可读"))?,
        );
        if vtable < base || vtable >= base + actual.image_size as usize {
            return Err(io::Error::other("接口虚表不在所属模块内"));
        }
        report.push_str(&format!(
            "\n  {interface}：已取得，虚表 RVA {:x}",
            vtable - base
        ));
    }
    Ok(report)
}
pub fn inspect() -> io::Result<String> {
    let engine = module(
        "engine.dll",
        ImageIdentity {
            timestamp: 0x6ab2b381,
            image_size: 0xe52000,
        },
        0x2a2d60,
        &[("VEngineClient015", 0x4993c0)],
    )?;
    let client = module(
        "client.dll",
        ImageIdentity {
            timestamp: 0x6ab2b438,
            image_size: 0xb78000,
        },
        0x519540,
        &[("VClient017", 0x87a6d0), ("VClientEntityList003", 0x9df7a8)],
    )?;
    let wide: Vec<u16> = "client.dll".encode_utf16().chain(Some(0)).collect();
    // SAFETY: 上面已核对该已加载模块，这里只取得基址做只读元数据诊断。
    let client_base = unsafe { GetModuleHandleW(wide.as_ptr()) } as usize;
    let properties = match crate::netvars::inspect(client_base) {
        Ok(report) => report,
        Err(error) => format!("属性表诊断未完成：{error}"),
    };
    Ok(format!(
        "{engine}\n{client}\n{properties}\n当前仅诊断接口与属性元数据，未启用玩家采样或设备输出。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_pointer_is_an_error_not_a_dereference() {
        assert_eq!(read::<8>(1), None);
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(read::<8>(data.as_ptr() as usize), Some(data));
    }
}
