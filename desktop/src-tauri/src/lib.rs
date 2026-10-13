mod injector;
use gmod_dglab_controller::{
    device::{DeviceManager, DeviceSettings, DeviceSnapshot},
    hub::{load_settings, ControllerHub, HubSnapshot},
    rules::RuleConfig,
    waveforms::Waveform,
};
use injector::{ConnectSnapshot, Injector};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
type Hub = Arc<Mutex<ControllerHub>>;
struct DeviceConfig(Mutex<DeviceSettings>);
#[derive(serde::Serialize)]
struct DesktopSnapshot {
    #[serde(flatten)]
    hub: HubSnapshot,
    connection: ConnectSnapshot,
    device: DeviceSnapshot,
    device_settings: DeviceSettings,
}
#[tauri::command]
fn get_state(
    hub: State<'_, Hub>,
    injector: State<'_, Injector>,
    device: State<'_, DeviceManager>,
    settings: State<'_, DeviceConfig>,
) -> DesktopSnapshot {
    let snapshot = hub.lock().unwrap().snapshot();
    DesktopSnapshot {
        hub: snapshot,
        connection: injector.snapshot(),
        device: device.snapshot(),
        device_settings: settings.0.lock().unwrap().clone(),
    }
}
#[tauri::command]
fn update_config(config: RuleConfig, hub: State<'_, Hub>) -> Result<(), String> {
    hub.lock().unwrap().configure(config)
}
#[tauri::command]
fn set_preview(enabled: bool, device: State<'_, DeviceManager>) -> Result<(), String> {
    device.set_output(enabled)
}
#[tauri::command]
fn stop_preview(device: State<'_, DeviceManager>) -> Result<(), String> {
    device.reset()
}
#[tauri::command]
fn connect_device(
    settings: DeviceSettings,
    device: State<'_, DeviceManager>,
    config: State<'_, DeviceConfig>,
) -> Result<(), String> {
    if !matches!(
        device.snapshot().phase,
        gmod_dglab_controller::device::DevicePhase::Idle
            | gmod_dglab_controller::device::DevicePhase::Disconnected
            | gmod_dglab_controller::device::DevicePhase::Failed
    ) {
        return Err("请先断开当前设备连接".into());
    }
    settings.save()?;
    *config.0.lock().unwrap() = settings.clone();
    device.connect(settings)
}
#[tauri::command]
fn disconnect_device(device: State<'_, DeviceManager>) -> Result<(), String> {
    device.disconnect()
}
#[tauri::command]
fn update_device_settings(
    settings: DeviceSettings,
    config: State<'_, DeviceConfig>,
    device: State<'_, DeviceManager>,
) -> Result<(), String> {
    if !matches!(
        device.snapshot().phase,
        gmod_dglab_controller::device::DevicePhase::Idle
            | gmod_dglab_controller::device::DevicePhase::Disconnected
            | gmod_dglab_controller::device::DevicePhase::Failed
    ) {
        return Err("请先断开设备，再修改连接设置".into());
    }
    settings.save()?;
    *config.0.lock().unwrap() = settings;
    Ok(())
}
#[tauri::command]
fn select_device(key: String, device: State<'_, DeviceManager>) -> Result<(), String> {
    device.select_device(&key)
}
#[tauri::command]
async fn set_listen_address(address: String, hub: State<'_, Hub>) -> Result<(), String> {
    let address = address
        .parse()
        .map_err(|_| "监听地址格式无效，例如 127.0.0.1:47831".to_string())?;
    hub.lock().unwrap().set_address(address)
}
#[tauri::command]
fn get_waveforms(hub: State<'_, Hub>) -> Vec<Waveform> {
    hub.lock().unwrap().waveforms()
}
#[tauri::command]
fn import_waveforms(json: String, hub: State<'_, Hub>) -> Result<usize, String> {
    hub.lock().unwrap().import_waveforms(&json)
}
#[tauri::command]
fn connect_game(
    selected_pid: Option<u32>,
    hub: State<'_, Hub>,
    injector: State<'_, Injector>,
) -> Result<(), String> {
    injector.request_connect(selected_pid, hub.inner().clone())
}
#[tauri::command]
fn cancel_game_selection(injector: State<'_, Injector>) {
    injector.cancel_selection();
}
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let (settings, warning) = load_settings();
            let hub = Arc::new(Mutex::new(ControllerHub::open_identified(
                settings, warning,
            )));
            let (device_settings, device_warning) = match DeviceSettings::load() {
                Ok(settings) => (settings, None),
                Err(error) => (DeviceSettings::default(), Some(error)),
            };
            if let Some(warning) = device_warning {
                hub.lock().unwrap().note(&warning);
            }
            app.manage(DeviceManager::new(hub.clone()));
            app.manage(DeviceConfig(Mutex::new(device_settings)));
            app.manage(hub);
            app.manage(Injector::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            update_config,
            set_preview,
            stop_preview,
            set_listen_address,
            get_waveforms,
            import_waveforms,
            connect_game,
            cancel_game_selection,
            connect_device,
            disconnect_device,
            update_device_settings,
            select_device
        ])
        .build(tauri::generate_context!())
        .expect("桌面程序初始化失败")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                app.state::<DeviceManager>().shutdown();
                app.state::<Injector>().shutdown();
                app.state::<Hub>().lock().unwrap().shutdown();
            }
        });
}
