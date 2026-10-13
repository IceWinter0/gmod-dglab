use std::{io, path::PathBuf};

pub fn path() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("gmod-dglab"))
        .ok_or_else(|| io::Error::other("LOCALAPPDATA 不存在"))
}
#[repr(C)]
struct Blob {
    len: u32,
    data: *mut u8,
}
#[link(name = "crypt32")]
extern "system" {
    fn CryptProtectData(
        input: *const Blob,
        description: *const u16,
        entropy: *const Blob,
        reserved: *mut core::ffi::c_void,
        prompt: *mut core::ffi::c_void,
        flags: u32,
        output: *mut Blob,
    ) -> i32;
    fn CryptUnprotectData(
        input: *const Blob,
        description: *mut *mut u16,
        entropy: *const Blob,
        reserved: *mut core::ffi::c_void,
        prompt: *mut core::ffi::c_void,
        flags: u32,
        output: *mut Blob,
    ) -> i32;
}
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
}
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut core::ffi::c_void,
        buffer: *mut u8,
        len: u32,
        flags: u32,
    ) -> i32;
}

fn crypt(bytes: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(io::Error::other("运行凭据长度无效"));
    }
    let input = Blob {
        len: bytes.len() as u32,
        data: bytes.as_ptr() as *mut u8,
    };
    let mut output = Blob {
        len: 0,
        data: std::ptr::null_mut(),
    };
    // SAFETY: 所有输入在调用期间有效；DPAPI 分配的输出由 LocalFree 释放，禁止弹出 UI。
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = std::slice::from_raw_parts(output.data, output.len as usize).to_vec();
        LocalFree(output.data.cast());
        Ok(result)
    }
}
pub fn protect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    crypt(bytes, true)
}
pub fn unprotect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    crypt(bytes, false)
}
pub fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    // SAFETY: 向系统随机源提供有效、足够大的输出缓冲区。
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), bytes.as_mut_ptr(), 32, 2) } < 0 {
        return Err(io::Error::other("系统随机源失败"));
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub address: std::net::SocketAddr,
    pub token: String,
}
pub fn load_config() -> io::Result<RuntimeConfig> {
    let encrypted = std::fs::read(path()?.join("runtime.bin"))?;
    let config: RuntimeConfig = serde_json::from_slice(&unprotect(&encrypted)?)
        .map_err(|_| io::Error::other("运行配置无效"))?;
    if !config.address.ip().is_loopback() || config.address.port() == 0 || config.token.len() != 64
    {
        return Err(io::Error::other("只允许本机地址与有效运行凭据"));
    }
    Ok(config)
}
pub fn save_config(config: &RuntimeConfig) -> io::Result<()> {
    let folder = path()?;
    std::fs::create_dir_all(&folder)?;
    let bytes = serde_json::to_vec(config).map_err(io::Error::other)?;
    let encrypted = protect(&bytes)?;
    std::fs::write(folder.join("runtime.bin"), encrypted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_uses_actual_windows_user_protection() {
        let token = random_token().unwrap();
        assert_eq!(token.len(), 64);
        let protected = protect(token.as_bytes()).unwrap();
        assert!(!protected
            .windows(token.len())
            .any(|s| s == token.as_bytes()));
        assert_eq!(unprotect(&protected).unwrap(), token.as_bytes());
        assert!(unprotect(b"corrupted").is_err());
    }
}
