mod injector;
use gmod_dglab_controller::{
    hub::{load_settings, ControllerHub, HubSnapshot},
    rules::RuleConfig,
    waveforms::Waveform,
};
use injector::{ConnectSnapshot, Injector};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
type Hub = Arc<Mutex<ControllerHub>>;
#[derive(serde::Serialize)]
struct DesktopSnapshot {
    #[serde(flatten)]
    hub: HubSnapshot,
    connection: ConnectSnapshot,
}
#[tauri::command]
fn get_state(hub: State<'_, Hub>, injector: State<'_, Injector>) -> DesktopSnapshot {
    let snapshot = hub.lock().unwrap().snapshot();
    DesktopSnapshot {
        hub: snapshot,
        connection: injector.snapshot(),
    }
}
#[tauri::command]
fn update_config(config: RuleConfig, hub: State<'_, Hub>) -> Result<(), String> {
    hub.lock().unwrap().configure(config)
}
#[tauri::command]
fn set_preview(enabled: bool, hub: State<'_, Hub>) -> Result<(), String> {
    hub.lock().unwrap().set_preview(enabled)
}
#[tauri::command]
fn stop_preview(hub: State<'_, Hub>) {
    hub.lock().unwrap().stop_preview();
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
            app.manage(Arc::new(Mutex::new(ControllerHub::open_identified(
                settings, warning,
            ))));
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
            cancel_game_selection
        ])
        .build(tauri::generate_context!())
        .expect("桌面程序初始化失败")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                app.state::<Injector>().shutdown();
                app.state::<Hub>().lock().unwrap().shutdown();
            }
        });
}
