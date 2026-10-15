#[cfg(not(all(windows, target_arch = "x86_64")))]
compile_error!("采集 DLL 仅支持 Windows x64");
mod native;
mod netvars;
mod sampling;
mod transport;
use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicU32 = AtomicU32::new(0);
#[link(name = "kernel32")]
extern "system" {
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        arg: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetModuleHandleExW(flags: u32, address: *const u16, handle: *mut *mut c_void) -> i32;
}
fn log(message: &str) {
    use std::io::Write;
    if let Ok(folder) = gmod_dglab_common::runtime::path() {
        let _ = std::fs::create_dir_all(&folder);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder.join("collector-diagnostic.log"))
        {
            let _ = writeln!(file, "{message}");
        }
    }
}
unsafe extern "system" fn worker(_arg: *mut c_void) -> u32 {
    let mut pinned = std::ptr::null_mut();
    // SAFETY: 使用本模块函数地址定位并固定 DLL，诊断版仅在游戏退出时卸载。
    if unsafe { GetModuleHandleExW(5, worker as *const () as *const u16, &mut pinned) } == 0 {
        STATUS.store(3, Ordering::Release);
        return 1;
    }
    log("正在连接游戏，检查游戏接口、函数特征与玩家属性。");
    for attempt in 0..60 {
        if STOP.load(Ordering::Acquire) {
            break;
        }
        match native::inspect() {
            Ok((report, resolved)) => {
                log(&report);
                let hook = match sampling::Hook::install(resolved) {
                    Ok(hook) => {
                        log("玩家状态采样已启动，在游戏网络更新后读取血量、生命状态及可用的护甲数据。");
                        Some(hook)
                    }
                    Err(error) => {
                        log(&format!("玩家采样未启用：{error}"));
                        None
                    }
                };
                while !STOP.load(Ordering::Acquire) {
                    if let Err(error) = transport::connect_and_heartbeat() {
                        log(&format!("与本机接收程序的连接中断或失败：{error}"));
                    }
                    log(&sampling::statistics());
                    for _ in 0..20 {
                        if STOP.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                if let Some(hook) = hook {
                    if let Err(error) = hook.stop() {
                        log(&format!("恢复游戏原始回调失败：{error}"));
                    }
                }
                break;
            }
            Err(error) => {
                if attempt == 0 || attempt == 59 {
                    log(&format!("暂时无法读取游戏状态：{error}"));
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    log("游戏状态读取已停止。如需重新连接，请重启游戏后在面板中点击“连接游戏”。");
    STATUS.store(3, Ordering::Release);
    0
}

/// 请求结束诊断线程；不支持热卸载，模块固定至进程退出。
#[no_mangle]
pub extern "C" fn GmodDglabRequestStop() -> u32 {
    STOP.store(true, Ordering::Release);
    STATUS.load(Ordering::Acquire)
}
#[no_mangle]
pub extern "C" fn GmodDglabStatus() -> u32 {
    STATUS.load(Ordering::Acquire)
}

/// # Safety
/// 仅由 Windows 加载器调用，不能从其他线程手动调用此入口。
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _module: *mut c_void,
    reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    if reason == 1
        && STATUS
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        // SAFETY: 线程仅在加载器回调返回后开始运行，此处不等待、不访问游戏接口。
        let thread = unsafe {
            CreateThread(
                std::ptr::null_mut(),
                0,
                worker,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
            )
        };
        if thread.is_null() {
            STATUS.store(0, Ordering::Release);
            return 0;
        }
        unsafe { CloseHandle(thread) };
    } else if reason == 0 {
        STOP.store(true, Ordering::Release);
    }
    1
}
