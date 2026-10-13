pub mod image;
pub mod windows;
use gmod_dglab_controller::hub::ControllerHub;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize)]
pub struct GameCandidate {
    pub pid: u32,
    pub path: String,
    #[serde(skip)]
    pub created_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectPhase {
    Idle,
    Selecting,
    Injecting,
    Waiting,
    Failed,
}
#[derive(Clone, Serialize)]
pub struct ConnectSnapshot {
    pub phase: ConnectPhase,
    pub message: String,
    pub candidates: Vec<GameCandidate>,
    pub target_pid: Option<u32>,
}
impl Default for ConnectSnapshot {
    fn default() -> Self {
        Self {
            phase: ConnectPhase::Idle,
            message: String::new(),
            candidates: vec![],
            target_pid: None,
        }
    }
}
struct Inner {
    state: Mutex<ConnectSnapshot>,
    running: AtomicBool,
    stop: AtomicBool,
}
pub struct Injector {
    inner: Arc<Inner>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}
fn choose_game(
    games: &[GameCandidate],
    selected: Option<&GameCandidate>,
) -> Result<Option<GameCandidate>, String> {
    if games.is_empty() {
        return Err("未找到已启动的 GMod x64 游戏客户端，请先启动游戏并等待加载完成后重试".into());
    }
    if let Some(selected) = selected {
        return games
            .iter()
            .find(|g| {
                g.pid == selected.pid
                    && g.created_at == selected.created_at
                    && g.path.eq_ignore_ascii_case(&selected.path)
            })
            .cloned()
            .map(Some)
            .ok_or_else(|| "所选游戏已退出或 PID 已变化，请重新查找".into());
    }
    if games.len() == 1 {
        Ok(Some(games[0].clone()))
    } else {
        Ok(None)
    }
}
fn begin_attempt(running: &AtomicBool) -> Result<(), String> {
    running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "连接尝试正在进行，请等待".into())
}
impl Inner {
    fn phase(&self, phase: ConnectPhase, message: impl Into<String>) {
        let mut s = self.state.lock().unwrap();
        s.phase = phase;
        s.message = message.into();
    }
    fn attempt(
        &self,
        selected: Option<GameCandidate>,
        hub: &Arc<Mutex<ControllerHub>>,
    ) -> Result<(), String> {
        if self.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if hub.lock().unwrap().snapshot().game_connected {
            self.phase(ConnectPhase::Idle, "游戏已经连接");
            return Ok(());
        }
        hub.lock().unwrap().retry_listener()?;
        let games = windows::find_games()?;
        let legacy = games.len() == 1;
        let Some(candidate) = choose_game(&games, selected.as_ref())? else {
            let mut s = self.state.lock().unwrap();
            s.phase = ConnectPhase::Selecting;
            s.message = "检测到多个 GMod 进程，请选择要连接的游戏".into();
            s.candidates = games;
            return Ok(());
        };
        {
            let mut s = self.state.lock().unwrap();
            s.target_pid = Some(candidate.pid);
            s.candidates.clear();
        }
        let process = windows::GameProcess::open(&candidate)?;
        hub.lock().unwrap().expect_process(candidate.pid, legacy)?;
        if !process.has_collector()? {
            self.phase(
                ConnectPhase::Injecting,
                format!("正在向游戏 PID {} 加载采样 DLL", candidate.pid),
            );
            let path = collector_path(&std::env::current_exe().map_err(|e| e.to_string())?)?;
            if self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let mut job = process.begin_load(&path)?;
            let started = Instant::now();
            let mut noted = false;
            loop {
                if self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                match job.poll()? {
                    windows::LoadProgress::Completed => break,
                    windows::LoadProgress::Running => {
                        if started.elapsed() >= Duration::from_secs(5) && !noted {
                            self.phase(ConnectPhase::Injecting,"加载线程仍在运行，将继续等待；不会重复注入。若游戏无响应请由你手动退出游戏");
                            noted = true;
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            job.finish()?;
            drop(job);
            if !process.has_collector()? {
                return Err(
                    "Windows 加载结束，但未发现采样模块。请检查 bin DLL、运行权限和本机诊断日志"
                        .into(),
                );
            }
        }
        self.phase(
            ConnectPhase::Waiting,
            "采样 DLL 已加载，等待游戏认证连接（最多 15 秒）",
        );
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(15) {
            if self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            if !process.alive() {
                return Err("游戏已退出，请启动后重新连接".into());
            }
            let state = hub.lock().unwrap().snapshot();
            if state.game_connected
                && (state.game_process_id == Some(candidate.pid)
                    || (state.game_process_id.is_none() && legacy))
            {
                self.phase(
                    ConnectPhase::Idle,
                    format!("游戏 PID {} 已连接", candidate.pid),
                );
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("等待游戏连接超时。已加载的 DLL 不会重复注入；请查看 collector-diagnostic.log，旧版或未适配版本需重启游戏后重试".into())
    }
}
fn collector_path(executable: &std::path::Path) -> Result<PathBuf, String> {
    Ok(executable
        .parent()
        .ok_or("应用程序目录无效")?
        .join("bin")
        .join("gmod_dglab_collector.dll"))
}
impl Injector {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(ConnectSnapshot::default()),
                running: AtomicBool::new(false),
                stop: AtomicBool::new(false),
            }),
            workers: Mutex::new(vec![]),
        }
    }
    pub fn snapshot(&self) -> ConnectSnapshot {
        self.inner.state.lock().unwrap().clone()
    }
    pub fn request_connect(
        &self,
        selected_pid: Option<u32>,
        hub: Arc<Mutex<ControllerHub>>,
    ) -> Result<(), String> {
        if self.inner.stop.load(Ordering::Acquire) {
            return Err("面板正在退出".into());
        }
        begin_attempt(&self.inner.running)?;
        let selected = if let Some(pid) = selected_pid {
            let selected = self
                .snapshot()
                .candidates
                .into_iter()
                .find(|g| g.pid == pid);
            if selected.is_none() {
                self.inner.running.store(false, Ordering::Release);
                return Err("候选进程已失效，请重新查找".into());
            }
            selected
        } else {
            None
        };
        self.inner
            .phase(ConnectPhase::Injecting, "正在查找 GMod x64 进程…");
        let inner = self.inner.clone();
        let worker = thread::Builder::new()
            .name("game-connect".into())
            .spawn(move || {
                let result = inner.attempt(selected, &hub);
                if let Err(error) = result {
                    let log = format!("游戏连接：{error}");
                    hub.lock().unwrap().note(&log);
                    inner.phase(ConnectPhase::Failed, error);
                }
                inner.running.store(false, Ordering::Release);
            })
            .map_err(|e| {
                self.inner.running.store(false, Ordering::Release);
                self.inner.phase(ConnectPhase::Failed, e.to_string());
                e.to_string()
            })?;
        let mut workers = self.workers.lock().unwrap();
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let _ = workers.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        workers.push(worker);
        Ok(())
    }
    pub fn cancel_selection(&self) {
        let mut s = self.inner.state.lock().unwrap();
        if s.phase == ConnectPhase::Selecting {
            *s = ConnectSnapshot::default();
        }
    }
    pub fn shutdown(&self) {
        self.inner.stop.store(true, Ordering::Release);
    }
}
impl Drop for Injector {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn game(pid: u32, birth: u64) -> GameCandidate {
        GameCandidate {
            pid,
            path: format!("L:/Steam/{pid}/gmod.exe"),
            created_at: birth,
        }
    }
    #[test]
    fn selection_and_pid_reuse_are_checked() {
        assert!(choose_game(&[], None).is_err());
        assert_eq!(choose_game(&[game(1, 10)], None).unwrap().unwrap().pid, 1);
        assert!(choose_game(&[game(1, 10), game(2, 20)], None)
            .unwrap()
            .is_none());
        assert!(choose_game(&[game(1, 11)], Some(&game(1, 10))).is_err());
    }
    #[test]
    fn repeated_requests_are_rejected() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        assert!(begin_attempt(&flag).is_ok());
        assert!(begin_attempt(&flag).is_err());
    }
}
