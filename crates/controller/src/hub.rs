use crate::rules::{ControllerCore, ControllerSnapshot, RuleConfig};
use crate::waveforms::{self, Waveform};
use gmod_dglab_common::{
    protocol::Message,
    receiver,
    runtime::{self, RuntimeConfig},
};
use serde::{Deserialize, Serialize};
use std::{
    net::{SocketAddr, TcpListener},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct HubSnapshot {
    pub controller: ControllerSnapshot,
    pub game_connected: bool,
    pub game_process_id: Option<u32>,
    pub heartbeat_age_ms: Option<u64>,
    pub listen_address: String,
    pub listener_error: Option<String>,
}
struct Shared {
    core: ControllerCore,
    connected: bool,
    process_id: Option<u32>,
    expected: Option<(u32, bool)>,
    allow_unselected_legacy: bool,
    heartbeat_at: Option<u64>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSettings {
    pub address: SocketAddr,
    pub config: RuleConfig,
}
impl Default for PanelSettings {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:47831".parse().unwrap(),
            config: RuleConfig::default(),
        }
    }
}
pub fn load_settings() -> (PanelSettings, Option<String>) {
    let result = (|| -> Result<PanelSettings, String> {
        load_settings_at(&runtime::path().map_err(|e| e.to_string())?)
    })();
    match result {
        Ok(s) => (s, None),
        Err(e) => (
            PanelSettings::default(),
            Some(format!("{e}，已使用默认停用配置")),
        ),
    }
}
fn load_settings_at(dir: &std::path::Path) -> Result<PanelSettings, String> {
    let path = dir.join("panel.json");
    if !path.exists() {
        return Ok(PanelSettings::default());
    }
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 65536 {
        return Err("配置文件过大".into());
    }
    let s: PanelSettings = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "配置文件损坏".to_string())?;
    s.config.validate()?;
    if !s.address.ip().is_loopback() || s.address.port() == 0 {
        return Err("配置监听地址无效".into());
    }
    Ok(s)
}
fn save_settings(settings: &PanelSettings) -> Result<(), String> {
    let dir = runtime::path().map_err(|e| e.to_string())?;
    save_settings_at(&dir, settings)
}
fn save_settings_at(dir: &std::path::Path, settings: &PanelSettings) -> Result<(), String> {
    settings.config.validate()?;
    if !settings.address.ip().is_loopback() || settings.address.port() == 0 {
        return Err("配置监听地址无效".into());
    }
    atomic_write(
        dir,
        "panel.json",
        &serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?,
    )
}
fn atomic_write(dir: &std::path::Path, name: &str, data: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let temp = dir.join(format!("{name}-{}.tmp", std::process::id()));
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    use std::os::windows::ffi::OsStrExt;
    let src: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    // SAFETY: 以 NUL 终止的本地路径；替换完成后才对调用者报告保存成功。
    if unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), 1 | 8) } == 0 {
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_file(temp);
        return Err(format!("配置保存失败：{error}"));
    }
    Ok(())
}
pub struct ControllerHub {
    imports: Vec<Waveform>,
    shared: Arc<Mutex<Shared>>,
    started: Instant,
    address: SocketAddr,
    error: Option<String>,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
    token: String,
    persist: bool,
}
impl ControllerHub {
    pub fn dormant(settings: PanelSettings, error: Option<String>) -> Self {
        let mut core = ControllerCore::new(settings.config).expect("validated settings");
        if let Some(e) = &error {
            core.note(e.clone());
        }
        Self {
            imports: vec![],
            shared: Arc::new(Mutex::new(Shared {
                core,
                connected: false,
                process_id: None,
                expected: None,
                allow_unselected_legacy: true,
                heartbeat_at: None,
            })),
            started: Instant::now(),
            address: settings.address,
            error,
            stop: Arc::new(AtomicBool::new(false)),
            workers: vec![],
            token: String::new(),
            persist: true,
        }
    }
    pub fn start(address: SocketAddr) -> Result<Self, String> {
        let mut hub = Self::dormant(
            PanelSettings {
                address,
                ..PanelSettings::default()
            },
            None,
        );
        hub.set_address(address)?;
        Ok(hub)
    }
    pub fn open(settings: PanelSettings, warning: Option<String>) -> Self {
        Self::open_with_policy(settings, warning, true)
    }
    pub fn open_identified(settings: PanelSettings, warning: Option<String>) -> Self {
        Self::open_with_policy(settings, warning, false)
    }
    fn open_with_policy(
        settings: PanelSettings,
        warning: Option<String>,
        allow_unselected_legacy: bool,
    ) -> Self {
        let address = settings.address;
        let mut hub = Self::dormant(settings, warning);
        hub.shared.lock().unwrap().allow_unselected_legacy = allow_unselected_legacy;
        let load = (|| -> Result<Vec<Waveform>, String> {
            let file = runtime::path()
                .map_err(|e| e.to_string())?
                .join("waveforms.json");
            if !file.exists() {
                return Ok(vec![]);
            }
            if std::fs::metadata(&file).map_err(|e| e.to_string())?.len()
                > waveforms::MAX_IMPORT_BYTES as u64
            {
                return Err("已保存波形库过大".into());
            }
            waveforms::parse_import(&std::fs::read_to_string(file).map_err(|e| e.to_string())?)
        })();
        match load {
            Ok(w) => hub.imports = w,
            Err(e) => hub
                .shared
                .lock()
                .unwrap()
                .core
                .note(format!("波形库加载失败：{e}")),
        }
        let mut config = hub.snapshot().controller.config;
        for channel in &mut config.channels {
            if !hub.waveforms().iter().any(|w| w.id == channel.waveform) {
                channel.waveform = waveforms::DEFAULT_WAVEFORM.into();
                hub.shared
                    .lock()
                    .unwrap()
                    .core
                    .note("原选定导入波形不可用，回退到官方呼吸波形");
            }
        }
        hub.shared
            .lock()
            .unwrap()
            .core
            .configure(config)
            .expect("validated config");
        if let Err(e) = hub.set_address(address) {
            hub.shared.lock().unwrap().core.note(e.clone());
            hub.error = Some(e);
        }
        hub
    }
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
    pub fn address(&self) -> Option<SocketAddr> {
        if self.workers.is_empty() {
            None
        } else {
            Some(self.address)
        }
    }
    pub fn snapshot(&self) -> HubSnapshot {
        let s = self.shared.lock().unwrap();
        HubSnapshot {
            controller: s.core.snapshot(),
            game_connected: s.connected,
            game_process_id: s.process_id,
            heartbeat_age_ms: s.heartbeat_at.map(|at| self.now().saturating_sub(at)),
            listen_address: self.address.to_string(),
            listener_error: self.error.clone(),
        }
    }
    pub fn configure(&mut self, config: RuleConfig) -> Result<(), String> {
        config.validate()?;
        if config
            .channels
            .iter()
            .any(|c| !self.waveforms().iter().any(|w| w.id == c.waveform))
        {
            return Err("所选波形不在当前波形库中，请先导入".into());
        }
        if self.persist {
            save_settings(&PanelSettings {
                address: self.address,
                config: config.clone(),
            })?;
        }
        self.shared.lock().unwrap().core.configure(config)
    }
    pub fn set_preview(&mut self, enabled: bool) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        if enabled && !s.connected {
            return Err("游戏尚未连接".into());
        }
        s.core.set_preview(enabled, self.now())
    }
    pub fn stop_preview(&mut self) {
        self.shared.lock().unwrap().core.stop();
    }
    pub fn waveforms(&self) -> Vec<Waveform> {
        let mut waves = waveforms::presets();
        waves.extend(self.imports.clone());
        waves
    }
    pub fn import_waveforms(&mut self, json: &str) -> Result<usize, String> {
        let incoming = waveforms::parse_import(json)?;
        let count = incoming.len();
        let mut updated = self.imports.clone();
        for wave in incoming {
            if !updated.iter().any(|w| w.id == wave.id) {
                updated.push(wave);
            }
        }
        let encoded = waveforms::encode_imports(&updated)?;
        if self.persist {
            atomic_write(
                &runtime::path().map_err(|e| e.to_string())?,
                "waveforms.json",
                encoded.as_bytes(),
            )?;
        }
        self.imports = updated;
        self.shared
            .lock()
            .unwrap()
            .core
            .note(format!("已处理 {count} 种导入波形，重复条目自动去重"));
        Ok(count)
    }
    pub fn note(&mut self, message: &str) {
        self.shared.lock().unwrap().core.note(message);
    }
    pub fn expect_process(&mut self, pid: u32, allow_legacy: bool) -> Result<(), String> {
        let mut s = self.shared.lock().unwrap();
        if pid == 0 {
            return Err("目标 PID 无效".into());
        }
        if s.connected {
            return Err("游戏已经连接，不重复注入".into());
        }
        s.expected = Some((pid, allow_legacy));
        Ok(())
    }
    pub fn retry_listener(&mut self) -> Result<(), String> {
        if self.address().is_none() {
            self.set_address(self.address)?;
        }
        self.error = None;
        Ok(())
    }
    pub fn set_address(&mut self, address: SocketAddr) -> Result<(), String> {
        if !address.ip().is_loopback() || (address.port() == 0 && self.persist) {
            return Err("仅允许回环地址与 1..65535 端口".into());
        }
        if self.address() == Some(address) {
            return Ok(());
        }
        let listener = TcpListener::bind(address)
            .map_err(|e| format!("无法监听 {address}：{e}。请关闭旧接收器或修改端口"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let actual = listener.local_addr().map_err(|e| e.to_string())?;
        let token = runtime::random_token().map_err(|e| e.to_string())?;
        if self.persist {
            // 成功绑定后才发布凭据；失败不会接管原接收器。
            runtime::save_config(&RuntimeConfig {
                address: actual,
                token: token.clone(),
            })
            .map_err(|e| format!("运行凭据保存失败：{e}"))?;
        }
        self.shutdown();
        self.address = actual;
        self.token = token.clone();
        self.error = None;
        self.stop = Arc::new(AtomicBool::new(false));
        let shared = self.shared.clone();
        let stop = self.stop.clone();
        let started = self.started;
        self.workers.push(thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let policy_shared = shared.clone();
                        let result = receiver::receive_checked(
                            stream,
                            token.clone(),
                            &stop,
                            |message| {
                                let policy = policy_shared.lock().unwrap();
                                if let Message::Hello { process_id, .. } = message {
                                    validate_hello_target(
                                        policy.expected,
                                        *process_id,
                                        policy.allow_unselected_legacy,
                                    )
                                    .map_err(std::io::Error::other)?;
                                }
                                Ok(())
                            },
                            |message| {
                                if stop.load(Ordering::Acquire) {
                                    return;
                                }
                                let now = started.elapsed().as_millis() as u64;
                                let mut s = shared.lock().unwrap();
                                match message {
                                    Message::Hello { process_id, .. } => {
                                        s.core.disconnect();
                                        s.connected = true;
                                        s.process_id = process_id;
                                        s.heartbeat_at = Some(now);
                                        s.core.note("游戏已认证连接");
                                    }
                                    Message::Heartbeat { .. } => {
                                        s.heartbeat_at = Some(now);
                                    }
                                    Message::Event { data, .. } => {
                                        s.core.handle(data, now);
                                    }
                                }
                            },
                        );
                        let mut s = shared.lock().unwrap();
                        s.connected = false;
                        s.process_id = None;
                        s.heartbeat_at = None;
                        s.core.disconnect();
                        if let Err(e) = result {
                            s.core.note(e.to_string());
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => {
                        let mut s = shared.lock().unwrap();
                        s.core.disconnect();
                        s.connected = false;
                        s.process_id = None;
                        s.core.note("监听失败，已停用预览");
                        break;
                    }
                }
            }
        }));
        let shared = self.shared.clone();
        let stop = self.stop.clone();
        let started = self.started;
        self.workers.push(thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let now = started.elapsed().as_millis() as u64;
                let mut s = shared.lock().unwrap();
                if s.connected
                    && s.heartbeat_at
                        .is_some_and(|at| now.saturating_sub(at) >= 3000)
                {
                    s.core.disconnect();
                    s.connected = false;
                    s.process_id = None;
                }
                s.core.tick(now);
                drop(s);
                thread::sleep(Duration::from_millis(20));
            }
        }));
        if self.persist {
            let config = self.snapshot().controller.config;
            if let Err(e) = save_settings(&PanelSettings {
                address: actual,
                config,
            }) {
                self.shared.lock().unwrap().core.note(e);
            }
        }
        Ok(())
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        {
            let mut s = self.shared.lock().unwrap();
            s.connected = false;
            s.process_id = None;
            s.heartbeat_at = None;
            s.core.disconnect();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        let mut s = self.shared.lock().unwrap();
        s.connected = false;
        s.process_id = None;
        s.heartbeat_at = None;
        s.core.disconnect();
    }
    #[cfg(test)]
    fn start_for_test(address: SocketAddr) -> Result<Self, String> {
        let mut hub = Self::dormant(
            PanelSettings {
                address,
                ..PanelSettings::default()
            },
            None,
        );
        hub.persist = false;
        hub.set_address(address)?;
        Ok(hub)
    }
    #[cfg(test)]
    fn token_for_test(&self) -> &str {
        &self.token
    }
}
impl Drop for ControllerHub {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn validate_hello_target(
    expected: Option<(u32, bool)>,
    pid: Option<u32>,
    allow_unselected_legacy: bool,
) -> Result<(), String> {
    if expected.is_none() && pid.is_none() && !allow_unselected_legacy {
        return Err(
            "旧版 DLL 未提供进程 ID，请点击连接游戏以确认唯一进程，或重启游戏使用新版 DLL".into(),
        );
    }
    validate_target(expected, pid)
}
fn validate_target(expected: Option<(u32, bool)>, pid: Option<u32>) -> Result<(), String> {
    match expected {
        None => Ok(()),
        Some((target, legacy)) if pid == Some(target) || (pid.is_none() && legacy) => Ok(()),
        _ => Err(
            "连接不属于所选游戏进程；旧版 DLL 在多进程下无法确认归属，请关闭其他游戏或重启目标游戏"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn imports_are_selectable_deduplicated_and_invalid_batch_is_atomic() {
        let mut hub = ControllerHub::start_for_test("127.0.0.1:0".parse().unwrap()).unwrap();
        let json = r#"{"name":"测试导入","frames":["0A0A0A0A00646400"]}"#;
        assert_eq!(hub.import_waveforms(json).unwrap(), 1);
        hub.import_waveforms(json).unwrap();
        let imports: Vec<_> = hub
            .waveforms()
            .into_iter()
            .filter(|w| w.id.starts_with("user:"))
            .collect();
        assert_eq!(imports.len(), 1);
        let mut config = hub.snapshot().controller.config;
        config.channels[0].waveform = imports[0].id.clone();
        hub.configure(config.clone()).unwrap();
        assert!(hub
            .import_waveforms(
                r#"[{"name":"好","frames":["0A0A0A0A00000000"]},{"name":"坏","frames":["bad"]}]"#
            )
            .is_err());
        assert_eq!(hub.waveforms().len(), 25);
        config.channels[0].waveform = format!("user:{}", "0".repeat(64));
        assert!(hub.configure(config).is_err());
        assert_eq!(
            hub.snapshot().controller.config.channels[0].waveform,
            imports[0].id
        );
    }
    use super::*;
    use std::net::{TcpListener, TcpStream};
    #[test]
    fn real_connection_disconnect_and_port_conflict() {
        let mut hub = ControllerHub::start_for_test("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(ControllerHub::start_for_test(hub.address().unwrap()).is_err());
        let (mut ws, _) = tungstenite::client(
            format!("ws://{}", hub.address().unwrap()),
            TcpStream::connect(hub.address().unwrap()).unwrap(),
        )
        .unwrap();
        ws.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"test","token":"{}","module_id":"test"}}"#, hub.token_for_test()).into())).unwrap();
        ws.read().unwrap();
        ws.send(tungstenite::Message::Text(r#"{"type":"event","version":1,"session_id":"test","seq":1,"age_ms":0,"data":{"kind":"player_state","sample":{"entity_key":1,"health":100,"armor":null,"life_state":0},"max_health":100,"thread_id":1}}"#.into())).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while hub.snapshot().controller.player.is_none() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(hub.snapshot().game_connected);
        let mut config = RuleConfig::default();
        config.channels[0].enabled = true;
        config.channels[0].max_strength = 100;
        hub.configure(config).unwrap();
        hub.set_preview(true).unwrap();
        ws.send(tungstenite::Message::Text(r#"{"type":"event","version":1,"session_id":"test","seq":2,"age_ms":0,"data":{"kind":"health_decreased","before":100,"after":80,"death_priority":false}}"#.into())).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while hub.snapshot().controller.targets[0] == 0 && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(hub.snapshot().controller.targets[0], 20);
        ws.close(None).unwrap();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while hub.snapshot().game_connected && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!hub.snapshot().controller.preview_enabled);
        assert_eq!(hub.snapshot().controller.targets, [0, 0]);
        let (mut reconnect, _) = tungstenite::client(
            format!("ws://{}", hub.address().unwrap()),
            TcpStream::connect(hub.address().unwrap()).unwrap(),
        )
        .unwrap();
        reconnect.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"second","token":"{}","module_id":"test"}}"#,hub.token_for_test()).into())).unwrap();
        reconnect.read().unwrap();
        assert!(!hub.snapshot().controller.preview_enabled);
        hub.shutdown();
    }
    #[test]
    fn port_switch_and_shutdown_join_workers() {
        let mut hub = ControllerHub::start_for_test("127.0.0.1:0".parse().unwrap()).unwrap();
        let old = hub.address().unwrap();
        hub.set_address("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(TcpListener::bind(old).is_ok());
        assert!(!hub.snapshot().controller.preview_enabled);
        let addr = hub.address().unwrap();
        hub.shutdown();
        assert!(TcpListener::bind(addr).is_ok());
    }
    #[test]
    fn switching_port_disconnects_active_old_session() {
        let mut hub = ControllerHub::start_for_test("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = hub.address().unwrap();
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://{addr}"), stream).unwrap();
        ws.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"old","token":"{}","module_id":"test"}}"#,hub.token_for_test()).into())).unwrap();
        ws.read().unwrap();
        let start = Instant::now();
        hub.set_address("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(start.elapsed() < Duration::from_secs(4));
        assert!(ws.read().is_err());
        assert!(!hub.snapshot().game_connected);
        assert!(!hub.snapshot().controller.preview_enabled);
    }
    #[test]
    fn settings_roundtrip_replaces_atomically_and_rejects_corruption() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "settings-test-{}-{}",
                std::process::id(),
                runtime::random_token().unwrap()
            ));
        let mut settings = PanelSettings::default();
        save_settings_at(&dir, &settings).unwrap();
        settings.config.duration_ms = 500;
        save_settings_at(&dir, &settings).unwrap();
        assert_eq!(load_settings_at(&dir).unwrap().config.duration_ms, 500);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::write(dir.join("panel.json"), b"corrupt").unwrap();
        assert!(load_settings_at(&dir).is_err());
        std::fs::remove_file(dir.join("panel.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn target_pid_policy_and_legacy_are_explicit() {
        assert!(validate_target(Some((123, false)), Some(456)).is_err());
        assert!(validate_target(Some((123, false)), None).is_err());
        assert!(validate_target(Some((123, true)), None).is_ok());
        assert!(validate_target(Some((123, false)), Some(123)).is_ok());
    }
    #[test]
    fn unidentified_legacy_requires_explicit_single_process_selection() {
        assert!(validate_hello_target(None, None, false).is_err());
        assert!(validate_hello_target(None, Some(123), false).is_ok());
        assert!(validate_hello_target(Some((123, true)), None, false).is_ok());
        assert!(validate_hello_target(None, None, true).is_ok());
    }
}
