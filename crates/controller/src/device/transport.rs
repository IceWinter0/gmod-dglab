use super::scheduler::{OutputInput, OutputScheduler};
use super::v4::V4Session;
use super::{
    qr_data, DeviceCommand, DeviceEvent, DeviceMode, DevicePhase, DeviceSettings, DeviceSnapshot,
    StopStatus, V3Session,
};
use crate::hub::ControllerHub;
use std::{
    collections::HashMap,
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};
enum Session {
    V3(V3Session),
    V4(V4Session),
}
impl Session {
    fn receive(&mut self, text: &str) -> Result<Vec<DeviceEvent>, String> {
        match self {
            Self::V3(session) => session.receive(text),
            Self::V4(session) => session.receive(text),
        }
    }
    fn encode(&mut self, command: &DeviceCommand, now: u64) -> Result<Vec<String>, String> {
        match self {
            Self::V3(session) => session.encode(command),
            Self::V4(session) => session.encode(command, now),
        }
    }
    fn pairing_url(&self, base: &str) -> Result<String, String> {
        match self {
            Self::V3(session) => session.pairing_url(base),
            Self::V4(session) => session.pairing_url(base),
        }
    }
    fn stop_complete(&self) -> bool {
        match self {
            Self::V3(_) => true,
            Self::V4(session) => session.stop_complete(),
        }
    }
    fn check_timeouts(&self, now: u64) -> Result<(), String> {
        match self {
            Self::V3(_) => Ok(()),
            Self::V4(session) => session.check_timeouts(now),
        }
    }
}
struct Inner {
    hub: Arc<Mutex<ControllerHub>>,
    snapshot: Mutex<DeviceSnapshot>,
    running: AtomicBool,
    disconnect: AtomicBool,
    quit: AtomicBool,
    reset: AtomicBool,
    generation: AtomicU64,
    selection: Mutex<Option<String>>,
}
pub struct DeviceManager {
    inner: Arc<Inner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl DeviceManager {
    pub fn new(hub: Arc<Mutex<ControllerHub>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                hub,
                snapshot: Mutex::new(DeviceSnapshot::default()),
                running: AtomicBool::new(false),
                disconnect: AtomicBool::new(false),
                quit: AtomicBool::new(false),
                reset: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                selection: Mutex::new(None),
            }),
            worker: Mutex::new(None),
        }
    }
    pub fn snapshot(&self) -> DeviceSnapshot {
        self.inner.snapshot.lock().unwrap().clone()
    }
    pub fn connect(&self, settings: DeviceSettings) -> Result<(), String> {
        settings.validate()?;
        if self.inner.quit.load(Ordering::Acquire) {
            return Err("面板正在退出".into());
        }
        self.inner
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "设备连接正在进行，请先断开当前连接")?;
        self.inner.hub.lock().unwrap().stop_preview();
        self.inner.disconnect.store(false, Ordering::Release);
        self.inner.reset.store(false, Ordering::Release);
        let generation = self.inner.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *self.inner.snapshot.lock().unwrap() = DeviceSnapshot {
            mode: settings.mode,
            phase: DevicePhase::Connecting,
            message: "正在连接中继服务".into(),
            service_url: settings.endpoint().into(),
            ..Default::default()
        };
        let inner = self.inner.clone();
        let worker = thread::Builder::new()
            .name("dglab-device".into())
            .spawn(move || {
                if let Err(error) = run(&inner, &settings, generation) {
                    inner.hub.lock().unwrap().stop_preview();
                    let mut state = inner.snapshot.lock().unwrap();
                    state.phase = DevicePhase::Failed;
                    state.message = error;
                    state.qr_data_url = None;
                    state.strengths = [None; 2];
                    state.limits = [None; 2];
                    if state.stop_status == StopStatus::Requested {
                        state.stop_status = StopStatus::Unconfirmed;
                    }
                }
                inner.running.store(false, Ordering::Release);
            })
            .map_err(|_| {
                self.inner.running.store(false, Ordering::Release);
                "无法启动设备连接线程".to_string()
            })?;
        let mut slot = self.worker.lock().unwrap();
        if let Some(old) = slot.take() {
            if old.is_finished() {
                let _ = old.join();
            }
        }
        *slot = Some(worker);
        Ok(())
    }
    pub fn set_output(&self, enabled: bool) -> Result<(), String> {
        if !enabled {
            return self.reset();
        }
        let mut state = self.inner.snapshot.lock().unwrap();
        if state.phase != DevicePhase::Ready
            || !self.inner.running.load(Ordering::Acquire)
            || self.inner.disconnect.load(Ordering::Acquire)
        {
            return Err("请先连接设备并等待强度状态更新".into());
        }
        let mut hub = self.inner.hub.lock().unwrap();
        let snapshot = hub.snapshot();
        if snapshot
            .controller
            .config
            .channels
            .iter()
            .enumerate()
            .any(|(i, c)| c.enabled && state.limits[i].is_none())
        {
            return Err("尚未取得 APP 强度上限".into());
        }
        if !snapshot
            .controller
            .config
            .channels
            .iter()
            .any(|c| c.enabled)
        {
            return Err("请至少开启一个通道的输出".into());
        }
        if self.inner.reset.load(Ordering::Acquire) {
            return Err("正在重置强度，请稍后再启用输出".into());
        }
        hub.set_preview(true)?;
        state.stop_status = StopStatus::None;
        Ok(())
    }
    pub fn reset(&self) -> Result<(), String> {
        self.inner.hub.lock().unwrap().stop_preview();
        self.inner.reset.store(true, Ordering::Release);
        self.inner.snapshot.lock().unwrap().stop_status =
            if self.inner.running.load(Ordering::Acquire) {
                StopStatus::Requested
            } else {
                StopStatus::Unconfirmed
            };
        Ok(())
    }
    pub fn disconnect(&self) -> Result<(), String> {
        self.reset()?;
        self.inner.disconnect.store(true, Ordering::Release);
        Ok(())
    }
    pub fn select_device(&self, key: &str) -> Result<(), String> {
        if !self
            .snapshot()
            .choices
            .iter()
            .any(|device| device.key == key)
        {
            return Err("所选设备已离线，请重新选择".into());
        }
        self.reset()?;
        *self.inner.selection.lock().unwrap() = Some(key.into());
        Ok(())
    }
    pub fn shutdown(&self) {
        let _ = self.reset();
        self.inner.quit.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let start = Instant::now();
            while !worker.is_finished() && start.elapsed() < Duration::from_millis(1500) {
                thread::sleep(Duration::from_millis(10));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
impl Drop for DeviceManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn socket(settings: &DeviceSettings) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let url = super::settings::validate_url(settings.endpoint(), settings.mode)?;
    let start = Instant::now();
    let host = url.host_str().ok_or("中继主机无效")?;
    let addresses = (
        host.trim_start_matches('[').trim_end_matches(']'),
        url.port_or_known_default().ok_or("中继端口无效")?,
    )
        .to_socket_addrs()
        .map_err(|_| "无法解析中继地址")?;
    let mut stream = None;
    for address in addresses {
        let left = Duration::from_secs(8).saturating_sub(start.elapsed());
        if left.is_zero() {
            break;
        }
        if let Ok(tcp) = TcpStream::connect_timeout(&address, left) {
            stream = Some(tcp);
            break;
        }
    }
    let stream = stream.ok_or("无法连接中继，请检查网络和地址")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .map_err(|_| "无法设置连接超时")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(8)))
        .map_err(|_| "无法设置连接超时")?;
    let (mut ws, _) = tungstenite::client_tls(settings.endpoint(), stream)
        .map_err(|_| "中继握手失败，请检查地址或 TLS 证书")?;
    let tcp = match ws.get_ref() {
        MaybeTlsStream::Plain(tcp) => tcp,
        MaybeTlsStream::NativeTls(tls) => tls.get_ref(),
        _ => return Err("设备连接类型不支持".into()),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|_| "无法设置设备读取超时")?;
    tcp.set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| "无法设置设备写入超时")?;
    ws.set_config(|config| {
        config.max_message_size = Some(65536);
        config.max_frame_size = Some(65536);
        config.write_buffer_size = 0;
        config.max_write_buffer_size = 131072;
    });
    Ok(ws)
}
fn send(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    session: &mut Session,
    command: &DeviceCommand,
    now: u64,
) -> Result<(), String> {
    for text in session.encode(command, now)? {
        ws.send(Message::Text(text.into()))
            .map_err(|_| "设备指令发送失败")?;
    }
    Ok(())
}
fn run(inner: &Inner, settings: &DeviceSettings, generation: u64) -> Result<(), String> {
    let mut ws = socket(settings)?;
    if inner.quit.load(Ordering::Acquire) || inner.disconnect.load(Ordering::Acquire) {
        return Ok(());
    }
    {
        let mut state = inner.snapshot.lock().unwrap();
        state.phase = DevicePhase::WaitingForApp;
        state.message = "正在获取配对二维码".into();
    }
    let mut session = match settings.mode {
        DeviceMode::App3 => Session::V3(V3Session::default()),
        DeviceMode::App4 => Session::V4(V4Session::default()),
    };
    let mut scheduler = OutputScheduler::new(settings.mode);
    let started = Instant::now();
    let mut last_received = Instant::now();
    let mut cache: HashMap<String, Arc<[String]>> = HashMap::new();
    let mut expected: [Option<(u16, u64)>; 2] = [None; 2];
    let mut stop_deadline = None;
    'connection: loop {
        let now = started.elapsed().as_millis() as u64;
        if inner.generation.load(Ordering::Acquire) != generation {
            break;
        }
        if inner.reset.swap(false, Ordering::AcqRel) {
            scheduler.reset();
            expected = [None; 2];
            let result = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
            let mut state = inner.snapshot.lock().unwrap();
            state.stop_status = if result.is_err() {
                StopStatus::Unconfirmed
            } else if state.strengths == [Some(0); 2] && session.stop_complete() {
                StopStatus::Confirmed
            } else {
                StopStatus::Requested
            };
            stop_deadline = Some(now + 1000);
        }
        if let Some(key) = inner.selection.lock().unwrap().take() {
            if let Session::V4(protocol) = &mut session {
                protocol.select_device(&key)?;
                let device = protocol.device_state();
                let mut state = inner.snapshot.lock().unwrap();
                state.selected_key = device.selected_key;
                state.strengths = device.strengths;
                state.limits = device.limits;
                state.phase = if device.ready {
                    DevicePhase::Ready
                } else {
                    DevicePhase::WaitingForDevice
                };
            }
        }
        if inner.quit.load(Ordering::Acquire)
            || (inner.disconnect.load(Ordering::Acquire)
                && (stop_deadline.is_none_or(|at| now >= at)
                    || inner.snapshot.lock().unwrap().stop_status == StopStatus::Confirmed))
        {
            if stop_deadline.is_none() {
                let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
            }
            let _ = ws.close(None);
            break;
        }
        if last_received.elapsed() > Duration::from_secs(90) {
            return Err("设备连接超时，请重新连接".into());
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                last_received = Instant::now();
                let events = match session.receive(&text) {
                    Ok(events) => events,
                    Err(error) => {
                        let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
                        return Err(error);
                    }
                };
                for event in events {
                    match event {
                        DeviceEvent::Assigned => {
                            let qr = qr_data(&session.pairing_url(settings.endpoint())?)?;
                            let mut state = inner.snapshot.lock().unwrap();
                            state.qr_data_url = Some(qr);
                            state.message = "请在郊狼 APP 的 SOCKET 控制中扫描二维码".into();
                        }
                        DeviceEvent::Paired => {
                            if settings.mode == DeviceMode::App4 {
                                send(&mut ws, &mut session, &DeviceCommand::RequestDevices, now)?;
                            }
                            let mut state = inner.snapshot.lock().unwrap();
                            state.phase = DevicePhase::WaitingForDevice;
                            state.qr_data_url = None;
                            state.message = "APP 已配对，等待强度和上限回传".into();
                        }
                        DeviceEvent::Feedback { strengths, limits } => {
                            for i in 0..2 {
                                if expected[i].is_some_and(|(value, _)| strengths[i] == value) {
                                    expected[i] = None;
                                }
                            }
                            let mut state = inner.snapshot.lock().unwrap();
                            state.strengths = strengths.map(Some);
                            state.limits = limits.map(Some);
                            state.phase = DevicePhase::Ready;
                            state.message = "APP 已连接".into();
                            if state.stop_status == StopStatus::Requested && strengths == [0; 2] {
                                state.stop_status = StopStatus::Confirmed;
                            }
                            if state.stop_status == StopStatus::Confirmed && strengths != [0; 2] {
                                state.stop_status = StopStatus::None;
                            }
                        }
                        DeviceEvent::Disconnected => {
                            inner.hub.lock().unwrap().stop_preview();
                            break 'connection;
                        }
                        DeviceEvent::Error(error) => {
                            let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
                            return Err(error);
                        }
                        DeviceEvent::State(device) => {
                            let selection_changed =
                                inner.snapshot.lock().unwrap().selected_key != device.selected_key;
                            if !device.ready || selection_changed {
                                inner.hub.lock().unwrap().stop_preview();
                                scheduler.reset();
                                expected = [None; 2];
                            }
                            for (i, strength) in device.strengths.iter().enumerate() {
                                if expected[i].is_some_and(|(value, _)| *strength == Some(value)) {
                                    expected[i] = None;
                                }
                            }
                            let mut state = inner.snapshot.lock().unwrap();
                            state.choices = device.choices;
                            state.selected_key = device.selected_key;
                            state.strengths = device.strengths;
                            state.limits = device.limits;
                            state.phase = if device.ready {
                                DevicePhase::Ready
                            } else {
                                DevicePhase::WaitingForDevice
                            };
                            state.message = if device.ready {
                                "郊狼 3.0 已连接"
                            } else if state.choices.is_empty() {
                                "请在 APP 中连接郊狼 3.0"
                            } else if state.selected_key.is_none() {
                                "请选择要控制的郊狼 3.0"
                            } else {
                                "等待设备连接及强度状态更新"
                            }
                            .into();
                            if matches!(
                                state.stop_status,
                                StopStatus::Requested | StopStatus::Unconfirmed
                            ) && state.strengths == [Some(0); 2]
                                && session.stop_complete()
                            {
                                state.stop_status = StopStatus::Confirmed;
                            }
                            if state.stop_status == StopStatus::Confirmed
                                && state
                                    .strengths
                                    .iter()
                                    .any(|value| value.is_some_and(|value| value != 0))
                            {
                                state.stop_status = StopStatus::None;
                            }
                        }
                        DeviceEvent::RpcComplete { stop_complete } => {
                            let mut state = inner.snapshot.lock().unwrap();
                            if stop_complete
                                && state.strengths == [Some(0); 2]
                                && matches!(
                                    state.stop_status,
                                    StopStatus::Requested | StopStatus::Unconfirmed
                                )
                            {
                                state.stop_status = StopStatus::Confirmed;
                            }
                        }
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {
                let _ = ws.flush();
            }
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Err("设备连接已断开，请重新连接".into()),
        }
        if expected
            .iter()
            .any(|item| item.is_some_and(|(_, at)| now >= at))
        {
            inner.hub.lock().unwrap().stop_preview();
            let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
            return Err("APP 未确认强度更新，输出已停用".into());
        }
        if let Err(error) = session.check_timeouts(now) {
            let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
            return Err(error);
        }
        {
            let mut state = inner.snapshot.lock().unwrap();
            if stop_deadline.is_some_and(|at| now >= at)
                && state.stop_status == StopStatus::Requested
            {
                state.stop_status = StopStatus::Unconfirmed;
            }
        }
        let hub = inner.hub.lock().unwrap().snapshot();
        let mut frame_sets: [Arc<[String]>; 2] = Default::default();
        if hub.controller.preview_enabled {
            for (i, config) in hub.controller.config.channels.iter().enumerate() {
                if !cache.contains_key(&config.waveform) {
                    let waves = inner.hub.lock().unwrap().waveforms();
                    for wave in waves {
                        cache.insert(wave.id, Arc::from(wave.frames));
                    }
                }
                frame_sets[i] = cache
                    .get(&config.waveform)
                    .cloned()
                    .ok_or("所选波形不可用")?;
            }
        }
        let input = OutputInput {
            game_connected: hub.game_connected,
            enabled: hub.controller.preview_enabled && !inner.disconnect.load(Ordering::Acquire),
            strengths: hub.controller.targets,
            channels: hub.controller.config.channels,
            frames: frame_sets,
        };
        let device = inner.snapshot.lock().unwrap().clone();
        let commands = match scheduler.tick(now, &input, &device) {
            Ok(commands) => commands,
            Err(error) => {
                inner.hub.lock().unwrap().stop_preview();
                let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
                return Err(error);
            }
        };
        for command in commands {
            if inner.reset.load(Ordering::Acquire) || inner.quit.load(Ordering::Acquire) {
                break;
            }
            if let Err(error) = send(&mut ws, &mut session, &command, now) {
                inner.hub.lock().unwrap().stop_preview();
                let _ = send(&mut ws, &mut session, &DeviceCommand::ResetAll, now);
                return Err(error);
            }
            if let DeviceCommand::SetStrength { channel, value } = command {
                if device.strengths[channel] != Some(value) {
                    expected[channel] = Some((value, now + 3000));
                }
            }
        }
    }
    inner.hub.lock().unwrap().stop_preview();
    let mut state = inner.snapshot.lock().unwrap();
    state.phase = DevicePhase::Disconnected;
    state.message = "设备连接已断开".into();
    state.qr_data_url = None;
    state.strengths = [None; 2];
    state.limits = [None; 2];
    if state.stop_status == StopStatus::Requested {
        state.stop_status = StopStatus::Unconfirmed;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::{ControllerHub, PanelSettings};
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };
    fn wait_until(mut condition: impl FnMut() -> bool) {
        let start = Instant::now();
        while !condition() && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(condition());
    }
    #[test]
    fn real_game_and_device_limit_drop_and_reset() {
        let mut core = ControllerHub::start_for_test("127.0.0.1:0".parse().unwrap()).unwrap();
        let (mut game, _) = tungstenite::client(
            format!("ws://{}", core.address().unwrap()),
            std::net::TcpStream::connect(core.address().unwrap()).unwrap(),
        )
        .unwrap();
        game.send(Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"device-test","token":"{}","module_id":"test"}}"#,core.token_for_test()).into())).unwrap();
        game.read().unwrap();
        game.send(Message::Text(r#"{"type":"event","version":1,"session_id":"device-test","seq":1,"age_ms":0,"data":{"kind":"player_state","sample":{"entity_key":1,"health":100,"armor":null,"life_state":0},"max_health":100,"thread_id":1}}"#.into())).unwrap();
        wait_until(|| core.snapshot().controller.player.is_some());
        let mut config = core.snapshot().controller.config;
        config.channels[0].enabled = true;
        config.channels[0].base_strength = 50;
        config.channels[0].max_strength = 80;
        core.configure(config).unwrap();
        let hub = Arc::new(Mutex::new(core));
        let manager = DeviceManager::new(hub.clone());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        manager
            .connect(DeviceSettings {
                v3_url: format!("ws://{}", listener.local_addr().unwrap()),
                ..Default::default()
            })
            .unwrap();
        let (tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut app = tungstenite::accept(tcp).unwrap();
        for message in [
            r#"{"type":"bind","clientId":"panel","targetId":"","message":"targetId"}"#,
            r#"{"type":"bind","clientId":"panel","targetId":"app","message":"200"}"#,
            r#"{"type":"msg","clientId":"panel","targetId":"app","message":"strength-0+0+35+100"}"#,
        ] {
            app.send(Message::Text(message.into())).unwrap();
        }
        wait_until(|| manager.snapshot().phase == DevicePhase::Ready);
        assert!(manager.connect(DeviceSettings::default()).is_err());
        manager.set_output(true).unwrap();
        let read_strength = |app: &mut WebSocket<TcpStream>| loop {
            let message = app.read().unwrap();
            if let Message::Text(text) = message {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                let value = frame["message"].as_str().unwrap();
                if value.starts_with("strength-1+2+") {
                    break value.to_string();
                }
            }
        };
        assert_eq!(read_strength(&mut app), "strength-1+2+0");
        assert_eq!(read_strength(&mut app), "strength-1+2+35");
        app.send(Message::Text(r#"{"type":"msg","clientId":"panel","targetId":"app","message":"strength-35+0+35+100"}"#.into())).unwrap();
        wait_until(|| manager.snapshot().strengths[0] == Some(35));
        app.send(Message::Text(r#"{"type":"msg","clientId":"panel","targetId":"app","message":"strength-35+0+20+100"}"#.into())).unwrap();
        assert_eq!(read_strength(&mut app), "strength-1+2+20");
        manager.reset().unwrap();
        assert!(!hub.lock().unwrap().snapshot().controller.preview_enabled);
        assert_eq!(read_strength(&mut app), "strength-1+2+0");
        app.send(Message::Text(
            r#"{"type":"msg","clientId":"panel","targetId":"app","message":"strength-0+0+20+100"}"#
                .into(),
        ))
        .unwrap();
        wait_until(|| manager.snapshot().stop_status == StopStatus::Confirmed);
        manager.disconnect().unwrap();
        wait_until(|| !manager.inner.running.load(Ordering::Acquire));
        manager.shutdown();
        game.close(None).unwrap();
        hub.lock().unwrap().shutdown();
    }
    #[test]
    fn real_ws_pairing_does_not_enable_output() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let hub = Arc::new(Mutex::new(ControllerHub::dormant(
            PanelSettings::default(),
            None,
        )));
        let manager = DeviceManager::new(hub.clone());
        manager
            .connect(super::super::DeviceSettings {
                v3_url: format!("ws://{address}"),
                ..Default::default()
            })
            .unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        for text in [
            r#"{"type":"bind","clientId":"panel","targetId":"","message":"targetId"}"#,
            r#"{"type":"bind","clientId":"panel","targetId":"app","message":"200"}"#,
            r#"{"type":"msg","clientId":"panel","targetId":"app","message":"strength-0+0+100+100"}"#,
        ] {
            ws.send(tungstenite::Message::Text(text.into())).unwrap();
        }
        let start = Instant::now();
        while manager.snapshot().phase != super::super::DevicePhase::Ready
            && start.elapsed() < Duration::from_secs(3)
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(manager.snapshot().phase, super::super::DevicePhase::Ready);
        assert!(!hub.lock().unwrap().snapshot().controller.preview_enabled);
        assert!(ws.read().is_err());
        let start = Instant::now();
        manager.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn real_v4_device_get_and_disconnect_stays_disabled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let manager = DeviceManager::new(Arc::new(Mutex::new(ControllerHub::dormant(
            PanelSettings::default(),
            None,
        ))));
        manager
            .connect(DeviceSettings {
                mode: DeviceMode::App4,
                v4_url: format!("ws://{}/v4", listener.local_addr().unwrap()),
                ..Default::default()
            })
            .unwrap();
        let (tcp, _) = listener.accept().unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut app = tungstenite::accept(tcp).unwrap();
        for text in [
            r#"{"type":"hello","clientId":"panel"}"#,
            r#"{"type":"client_attached","clientId":"app"}"#,
        ] {
            app.send(Message::Text(text.into())).unwrap();
        }
        let request: serde_json::Value =
            serde_json::from_str(app.read().unwrap().into_text().unwrap().as_str()).unwrap();
        assert_eq!(request["data"]["m"], "devices.get");
        let response = serde_json::json!({"type":"message","clientId":"app","data":{"t":"resp","reqId":request["data"]["reqId"],"result":{"devices":[{"slotId":"coyote","name":"郊狼","type":"COYOTE_030","props":{"intensityA":0,"intensityB":0},"slotState":{"hasDevice":true,"channelA":{"intensityMax":100},"channelB":{"intensityMax":35}}}]}}});
        app.send(Message::Text(response.to_string().into()))
            .unwrap();
        wait_until(|| manager.snapshot().phase == DevicePhase::Ready);
        assert_eq!(manager.snapshot().limits, [Some(100), Some(35)]);
        assert!(manager.set_output(true).is_err());
        app.send(Message::Text(
            r#"{"type":"client_disconnected","clientId":"app"}"#.into(),
        ))
        .unwrap();
        wait_until(|| manager.snapshot().phase == DevicePhase::Disconnected);
        manager.shutdown();
    }
    #[test]
    fn v3_break_allows_a_fresh_disabled_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let manager = DeviceManager::new(Arc::new(Mutex::new(ControllerHub::dormant(
            PanelSettings::default(),
            None,
        ))));
        let settings = DeviceSettings {
            v3_url: format!("ws://{}", listener.local_addr().unwrap()),
            ..Default::default()
        };
        for client in ["first", "second"] {
            manager.connect(settings.clone()).unwrap();
            let (tcp, _) = listener.accept().unwrap();
            let mut app = tungstenite::accept(tcp).unwrap();
            for frame in [
                serde_json::json!({"type":"bind","clientId":client,"targetId":"","message":"targetId"}),
                serde_json::json!({"type":"bind","clientId":client,"targetId":"app","message":"200"}),
                serde_json::json!({"type":"msg","clientId":client,"targetId":"app","message":"strength-0+0+35+35"}),
            ] {
                app.send(Message::Text(frame.to_string().into())).unwrap();
            }
            wait_until(|| manager.snapshot().phase == DevicePhase::Ready);
            assert!(
                !manager
                    .inner
                    .hub
                    .lock()
                    .unwrap()
                    .snapshot()
                    .controller
                    .preview_enabled
            );
            app.send(Message::Text(serde_json::json!({"type":"break","clientId":client,"targetId":"app","message":"209"}).to_string().into())).unwrap();
            wait_until(|| !manager.inner.running.load(Ordering::Acquire));
            assert_eq!(manager.snapshot().phase, DevicePhase::Disconnected);
        }
        manager.shutdown();
    }
}
