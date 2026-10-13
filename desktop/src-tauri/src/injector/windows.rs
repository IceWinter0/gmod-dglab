use super::{image, GameCandidate};
use std::{ffi::c_void, mem::size_of, path::Path, ptr, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::*,
    System::{
        Diagnostics::{
            Debug::{ReadProcessMemory, WriteProcessMemory},
            ToolHelp::*,
        },
        LibraryLoader::*,
        Memory::*,
        Threading::*,
    },
};
struct Handle(HANDLE);
impl Handle {
    fn from(raw: HANDLE, operation: &str) -> Result<Self, String> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(format!("{operation}：{}", std::io::Error::last_os_error()))
        } else {
            Ok(Self(raw))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn text(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(wide.len())])
}
fn query(handle: HANDLE) -> Result<(String, u64), String> {
    let mut wide = vec![0u16; 32768];
    let mut length = wide.len() as u32;
    // SAFETY: 所有输出缓冲在 Windows 调用期间有效，句柄由调用者持有。
    unsafe {
        if QueryFullProcessImageNameW(handle, 0, wide.as_mut_ptr(), &mut length) == 0 {
            return Err(format!(
                "无法读取游戏映像路径：{}",
                std::io::Error::last_os_error()
            ));
        }
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
            return Err("无法核对进程创建时间".into());
        }
        let (mut process_machine, mut native_machine) = (0, 0);
        if IsWow64Process2(handle, &mut process_machine, &mut native_machine) == 0
            || process_machine != 0
            || native_machine != 0x8664
        {
            return Err("仅支持 Windows AMD64 的 GMod 进程".into());
        }
        Ok((
            String::from_utf16_lossy(&wide[..length as usize]),
            ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        ))
    }
}
pub fn is_game_path(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("gmod.exe"))
}
fn process_exited(handle: HANDLE) -> Result<bool, String> {
    let mut code = 0;
    // SAFETY: 借用的查询句柄仍由调用者持有；GetExitCodeProcess 只读取进程状态。
    if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 {
        return Err("无法核对进程是否已退出".into());
    }
    Ok(code != STILL_ACTIVE as u32)
}
fn probe_game(pid: u32) -> Result<Option<GameCandidate>, String> {
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() && unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
        return Ok(None);
    }
    let handle = Handle::from(raw, "无法查询 GMod 进程")?;
    if process_exited(handle.0)? {
        return Ok(None);
    }
    let result = (|| {
        let (path, created_at) = query(handle.0)?;
        if !is_game_path(&path) || !is_game_client(&modules_for_pid(pid)?) {
            return Ok(None);
        }
        Ok(Some(GameCandidate {
            pid,
            path,
            created_at,
        }))
    })();
    if result.is_err() && process_exited(handle.0)? {
        return Ok(None);
    }
    result.map_err(|error: String| format!("PID {pid}：{error}"))
}
pub fn find_games() -> Result<Vec<GameCandidate>, String> {
    // SAFETY: 快照句柄由 RAII 管理，结构大小按 Windows SDK 定义传入。
    unsafe {
        let snapshot = Handle::from(
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0),
            "无法枚举游戏进程",
        )?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snapshot.0, &mut entry);
        let mut games = Vec::new();
        let mut errors = Vec::new();
        while more != 0 {
            if text(&entry.szExeFile).eq_ignore_ascii_case("gmod.exe") {
                match probe_game(entry.th32ProcessID) {
                    Ok(Some(game)) => games.push(game),
                    Ok(None) => {}
                    Err(e) => errors.push(e),
                }
            }
            more = Process32NextW(snapshot.0, &mut entry);
        }
        if !errors.is_empty() {
            return Err(format!(
                "部分 GMod 进程无法验证：{}；请检查游戏与面板的运行权限",
                errors.join("；")
            ));
        }
        games.sort_by_key(|g| g.pid);
        Ok(games)
    }
}
struct Module {
    name: String,
    base: usize,
    size: usize,
}
fn is_game_client(modules: &[Module]) -> bool {
    ["engine.dll", "client.dll"].iter().all(|required| {
        modules
            .iter()
            .any(|module| module.name.eq_ignore_ascii_case(required))
    })
}
fn modules_for_pid(pid: u32) -> Result<Vec<Module>, String> {
    let mut snapshot = None;
    for _ in 0..3 {
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid) };
        if raw != INVALID_HANDLE_VALUE {
            snapshot = Some(Handle::from(raw, "无法读取游戏模块")?);
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let snapshot =
        snapshot.ok_or_else(|| format!("无法读取游戏模块：{}", std::io::Error::last_os_error()))?;
    let mut entry = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut modules = Vec::new();
    unsafe {
        let mut more = Module32FirstW(snapshot.0, &mut entry);
        if more == 0 {
            return Err("游戏模块快照为空，请等待游戏完成启动".into());
        }
        while more != 0 {
            modules.push(Module {
                name: text(&entry.szModule),
                base: entry.modBaseAddr as usize,
                size: entry.modBaseSize as usize,
            });
            if modules.len() > 4096 {
                return Err("游戏模块数量异常".into());
            }
            more = Module32NextW(snapshot.0, &mut entry);
        }
    }
    Ok(modules)
}
pub struct GameProcess {
    handle: Handle,
    pub candidate: GameCandidate,
}
impl GameProcess {
    pub fn open(candidate: &GameCandidate) -> Result<Self, String> {
        // SAFETY: 仅持有刚验证过的真实 gmod.exe，未申请 PROCESS_ALL_ACCESS。
        let handle = Handle::from(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_INFORMATION
                        | PROCESS_QUERY_LIMITED_INFORMATION
                        | PROCESS_VM_READ
                        | PROCESS_VM_WRITE
                        | PROCESS_VM_OPERATION
                        | PROCESS_CREATE_THREAD
                        | PROCESS_SYNCHRONIZE,
                    0,
                    candidate.pid,
                )
            },
            "无法打开 GMod；请检查权限",
        )?;
        let (path, created_at) = query(handle.0)?;
        if !is_game_path(&path)
            || created_at != candidate.created_at
            || !path.eq_ignore_ascii_case(&candidate.path)
        {
            return Err("游戏进程已退出或 PID 已被复用，请重新查找".into());
        }
        let result = Self {
            handle,
            candidate: candidate.clone(),
        };
        if !result.alive() {
            return Err("游戏进程已退出".into());
        }
        if !is_game_client(&result.modules()?) {
            return Err("所选进程不是已启动的 GMod 游戏客户端，请等待游戏启动后重试".into());
        }
        Ok(result)
    }
    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle.0, 0) == WAIT_TIMEOUT }
    }
    fn modules(&self) -> Result<Vec<Module>, String> {
        modules_for_pid(self.candidate.pid)
    }
    fn read(&self, address: usize, len: usize) -> Option<Vec<u8>> {
        read_memory(self.handle.0, address, len)
    }
    pub fn has_collector(&self) -> Result<bool, String> {
        for module in self.modules()? {
            if ["gmod_dglab_collector.dll", "gmod_dglab_sampling.dll"]
                .iter()
                .any(|name| module.name.eq_ignore_ascii_case(name))
            {
                return Ok(true);
            }
            let Some(headers) = self.read(module.base, 4096) else {
                continue;
            };
            if image::has_collector_exports(&headers, |rva, len| {
                if (rva as usize).checked_add(len)? > module.size {
                    return None;
                }
                self.read(module.base.checked_add(rva as usize)?, len)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn loader_address(&self) -> Result<usize, String> {
        // SAFETY: Windows 返回本进程已加载的系统导出，使用 FROM_ADDRESS 定位实际所属模块。
        unsafe {
            let kernel_name: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
            let kernel = GetModuleHandleW(kernel_name.as_ptr());
            let loader = GetProcAddress(kernel, c"LoadLibraryW".as_ptr().cast())
                .ok_or("无法取得 LoadLibraryW")? as usize;
            let mut owner = ptr::null_mut();
            if GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                loader as *const u16,
                &mut owner,
            ) == 0
            {
                return Err("无法确定加载函数所属模块".into());
            }
            let mut path = [0u16; 32768];
            let length = GetModuleFileNameW(owner, path.as_mut_ptr(), path.len() as u32);
            if length == 0 || length as usize >= path.len() {
                return Err("系统模块路径无效".into());
            }
            let path = String::from_utf16_lossy(&path[..length as usize]);
            let filename = Path::new(&path)
                .file_name()
                .ok_or("系统模块名无效")?
                .to_string_lossy();
            let local =
                read_memory(GetCurrentProcess(), owner as usize, 4096).ok_or("系统模块头不可读")?;
            let identity = gmod_dglab_common::pe::identity(&local).ok_or("系统模块身份无效")?;
            let offset = loader
                .checked_sub(owner as usize)
                .filter(|offset| *offset < identity.image_size as usize)
                .ok_or("加载函数 RVA 越界")?;
            for module in self.modules()? {
                if module.name.eq_ignore_ascii_case(&filename) {
                    let headers = self.read(module.base, 4096).ok_or("远程系统模块头不可读")?;
                    if gmod_dglab_common::pe::identity(&headers) != Some(identity) {
                        return Err("本机与游戏系统模块版本不匹配".into());
                    }
                    return module
                        .base
                        .checked_add(offset)
                        .filter(|_| offset < module.size)
                        .ok_or("远程加载地址越界".into());
                }
            }
            Err("游戏尚未加载所需系统模块".into())
        }
    }
    pub fn begin_load(&self, path: &Path) -> Result<LoadJob<'_>, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("采样 DLL 缺失：{e}。请完整解压程序和 bin 文件夹"))?;
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > 64 * 1024 * 1024 {
            return Err("采样 DLL 文件过大".into());
        }
        image::validate_collector(&std::fs::read(&path).map_err(|e| e.to_string())?)?;
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if wide[..wide.len() - 1].contains(&0) {
            return Err("DLL 路径含 NUL".into());
        }
        let bytes = wide.len().checked_mul(2).ok_or("DLL 路径过长")?;
        let loader = self.loader_address()?;
        let mut marker = PendingMarker::at(
            &gmod_dglab_common::runtime::path().map_err(|e| e.to_string())?,
            self.candidate.pid,
            self.candidate.created_at,
        )?;
        // SAFETY: 只在已验证目标内分配可读写路径缓冲；加载函数地址来自匹配的系统映像。
        unsafe {
            let memory = VirtualAllocEx(
                self.handle.0,
                ptr::null(),
                bytes,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            );
            if memory.is_null() {
                return Err(format!(
                    "无法分配 DLL 路径：{}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut written = 0;
            if WriteProcessMemory(
                self.handle.0,
                memory,
                wide.as_ptr().cast(),
                bytes,
                &mut written,
            ) == 0
                || written != bytes
            {
                VirtualFreeEx(self.handle.0, memory, 0, MEM_RELEASE);
                return Err("无法写入 DLL 路径".into());
            }
            let start: LPTHREAD_START_ROUTINE = Some(std::mem::transmute::<
                usize,
                unsafe extern "system" fn(*mut c_void) -> u32,
            >(loader));
            let raw = CreateRemoteThread(
                self.handle.0,
                ptr::null(),
                0,
                start,
                memory,
                0,
                ptr::null_mut(),
            );
            if raw.is_null() {
                let error = std::io::Error::last_os_error();
                VirtualFreeEx(self.handle.0, memory, 0, MEM_RELEASE);
                return Err(format!("无法启动 DLL 加载线程：{error}"));
            }
            marker.started = true;
            Ok(LoadJob {
                process: self,
                thread: Handle(raw),
                memory,
                finished: false,
                marker,
            })
        }
    }
}
fn read_memory(handle: HANDLE, address: usize, len: usize) -> Option<Vec<u8>> {
    if len > 1024 * 1024 || address.checked_add(len).is_none() {
        return None;
    }
    let mut bytes = vec![0u8; len];
    let mut count = 0;
    unsafe {
        if ReadProcessMemory(
            handle,
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            len,
            &mut count,
        ) == 0
            || count != len
        {
            return None;
        }
    }
    Some(bytes)
}
pub enum LoadProgress {
    Running,
    Completed,
}
pub struct LoadJob<'a> {
    process: &'a GameProcess,
    thread: Handle,
    memory: *mut c_void,
    finished: bool,
    marker: PendingMarker,
}
impl LoadJob<'_> {
    pub fn finish(&mut self) -> Result<(), String> {
        if !self.finished {
            return Err("加载线程尚未确认结束，不能释放路径参数".into());
        }
        if self.memory.is_null() {
            return Ok(());
        }
        let released =
            unsafe { VirtualFreeEx(self.process.handle.0, self.memory, 0, MEM_RELEASE) } != 0;
        if !released && unsafe { WaitForSingleObject(self.process.handle.0, 0) } != WAIT_OBJECT_0 {
            return Err(format!(
                "释放 DLL 路径参数失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        self.memory = ptr::null_mut();
        self.marker.started = false;
        Ok(())
    }
    pub fn poll(&mut self) -> Result<LoadProgress, String> {
        let status = unsafe { WaitForSingleObject(self.thread.0, 0) };
        if status == WAIT_OBJECT_0 {
            self.finished = true;
            return Ok(LoadProgress::Completed);
        }
        if unsafe { WaitForSingleObject(self.process.handle.0, 0) } == WAIT_OBJECT_0 {
            self.finished = true;
            return Err("游戏在加载过程中退出".into());
        }
        // WAIT_FAILED 不证明线程结束；未知状态继续保留单次尝试与远程参数。
        Ok(LoadProgress::Running)
    }
}
impl Drop for LoadJob<'_> {
    fn drop(&mut self) {
        if self.finished || unsafe { WaitForSingleObject(self.thread.0, 0) } == WAIT_OBJECT_0 {
            self.finished = true;
            let _ = self.finish();
        } // 未确认结束时保留远程参数和跨会话标记。
    }
}
struct PendingMarker {
    path: std::path::PathBuf,
    started: bool,
}
impl PendingMarker {
    fn at(dir: &Path, pid: u32, birth: u64) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("load-pending-{pid}-{birth}.lock"));
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    "此游戏还有未确认的 DLL 加载，请等待或重启游戏；不会重复注入".into()
                } else {
                    format!("无法建立加载标记：{e}")
                }
            })?;
        Ok(Self {
            path,
            started: false,
        })
    }
}
impl Drop for PendingMarker {
    fn drop(&mut self) {
        if !self.started {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exited_native_process_is_not_a_query_failure() {
        use std::os::windows::process::CommandExt;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());
        assert!(probe_game(pid).unwrap().is_none());
    }
    #[test]
    fn only_exact_game_name_is_eligible() {
        assert!(is_game_path(r"L:\Steam\bin\win64\gmod.exe"));
        assert!(!is_game_path(r"L:\Steam\gmod.exe.fake"));
        assert!(!is_game_path(r"L:\Steam\gmod-dglab.exe"));
    }
    #[test]
    fn chromium_helpers_are_not_game_clients() {
        let modules = |names: &[&str]| {
            names
                .iter()
                .map(|name| Module {
                    name: (*name).into(),
                    base: 0,
                    size: 0,
                })
                .collect::<Vec<_>>()
        };
        assert!(!is_game_client(&modules(&["gmod.exe", "libcef.dll"])));
        assert!(!is_game_client(&modules(&["gmod.exe", "engine.dll"])));
        assert!(!is_game_client(&modules(&["gmod.exe", "client.dll"])));
        assert!(is_game_client(&modules(&[
            "gmod.exe",
            "ENGINE.DLL",
            "client.dll"
        ])));
    }
    #[test]
    fn actual_enumeration_is_read_only() {
        let games = find_games().unwrap();
        eprintln!(
            "verified game client PIDs: {:?}",
            games.iter().map(|game| game.pid).collect::<Vec<_>>()
        );
        for game in games {
            assert!(game.pid > 0);
            assert!(is_game_path(&game.path));
            assert!(game.created_at > 0);
            assert!(is_game_client(&modules_for_pid(game.pid).unwrap()));
        }
        let me = GameCandidate {
            pid: std::process::id(),
            path: std::env::current_exe().unwrap().to_string_lossy().into(),
            created_at: 0,
        };
        assert!(GameProcess::open(&me).is_err());
    }

    #[test]
    fn pending_load_marker_survives_app_close_and_prevents_duplicates() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "pending-marker-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let first = PendingMarker::at(&dir, 1, 10).unwrap();
        assert!(PendingMarker::at(&dir, 1, 10).is_err());
        drop(first);
        let mut started = PendingMarker::at(&dir, 1, 10).unwrap();
        started.started = true;
        drop(started);
        assert!(PendingMarker::at(&dir, 1, 10).is_err());
        std::fs::remove_file(dir.join("load-pending-1-10.lock")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn finished_native_thread_releases_path_and_pending_marker() {
        unsafe extern "system" fn done(_: *mut c_void) -> u32 {
            0
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "loader-cleanup-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let handle = Handle::from(
            unsafe {
                OpenProcess(
                    PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    std::process::id(),
                )
            },
            "test process",
        )
        .unwrap();
        let process = GameProcess {
            handle,
            candidate: GameCandidate {
                pid: std::process::id(),
                path: std::env::current_exe().unwrap().to_string_lossy().into(),
                created_at: 1,
            },
        };
        let mut marker = PendingMarker::at(&dir, process.candidate.pid, 1).unwrap();
        let marker_path = marker.path.clone();
        marker.started = true;
        let memory = unsafe {
            VirtualAllocEx(
                process.handle.0,
                ptr::null(),
                16,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        assert!(!memory.is_null());
        let thread = Handle::from(
            unsafe { CreateThread(ptr::null(), 0, Some(done), ptr::null(), 0, ptr::null_mut()) },
            "test thread",
        )
        .unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(thread.0, 2000) },
            WAIT_OBJECT_0
        );
        let mut job = LoadJob {
            process: &process,
            thread,
            memory,
            finished: false,
            marker,
        };
        assert!(matches!(job.poll().unwrap(), LoadProgress::Completed));
        job.finish()
            .expect("Windows released the owned path allocation");
        drop(job);
        assert!(!marker_path.exists());
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn actual_loader_rva_resolves_in_own_test_process_without_loading() {
        let handle = Handle::from(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
                    0,
                    std::process::id(),
                )
            },
            "test reader",
        )
        .unwrap();
        let process = GameProcess {
            handle,
            candidate: GameCandidate {
                pid: std::process::id(),
                path: std::env::current_exe().unwrap().to_string_lossy().into(),
                created_at: 1,
            },
        };
        let name: Vec<u16> = "kernel32.dll".encode_utf16().chain(Some(0)).collect();
        let expected = unsafe {
            GetProcAddress(
                GetModuleHandleW(name.as_ptr()),
                c"LoadLibraryW".as_ptr().cast(),
            )
        }
        .unwrap() as usize;
        assert_eq!(process.loader_address().unwrap(), expected);
    }
}
