mod scheduler;
mod settings;
mod transport;
mod v3;
mod v4;
use serde::{Deserialize, Serialize};
pub use settings::DeviceSettings;
pub use transport::DeviceManager;
pub use v3::V3Session;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    #[default]
    App3,
    App4,
}
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevicePhase {
    #[default]
    Idle,
    Connecting,
    WaitingForApp,
    WaitingForDevice,
    Ready,
    Disconnected,
    Failed,
}
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopStatus {
    #[default]
    None,
    Requested,
    Confirmed,
    Unconfirmed,
}
#[derive(Clone, Debug, Serialize)]
pub struct DeviceChoice {
    pub key: String,
    pub name: String,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct DeviceSnapshot {
    pub mode: DeviceMode,
    pub phase: DevicePhase,
    pub message: String,
    pub service_url: String,
    pub qr_data_url: Option<String>,
    pub strengths: [Option<u16>; 2],
    pub limits: [Option<u16>; 2],
    pub choices: Vec<DeviceChoice>,
    pub selected_key: Option<String>,
    pub stop_status: StopStatus,
}
#[derive(Clone, Debug)]
pub enum DeviceCommand {
    RequestDevices,
    SetStrength {
        channel: usize,
        value: u16,
    },
    Pulse {
        channel: usize,
        frames: Vec<String>,
        duration_ms: u64,
    },
    Clear {
        channel: usize,
    },
    ResetAll,
}
#[derive(Debug)]
pub enum DeviceEvent {
    Assigned,
    Paired,
    Feedback {
        strengths: [u16; 2],
        limits: [u16; 2],
    },
    Disconnected,
    Error(String),
    State(DeviceState),
    RpcComplete {
        stop_complete: bool,
    },
}
#[derive(Clone, Debug, Default)]
pub struct DeviceState {
    pub choices: Vec<DeviceChoice>,
    pub selected_key: Option<String>,
    pub strengths: [Option<u16>; 2],
    pub limits: [Option<u16>; 2],
    pub ready: bool,
}
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
pub(crate) fn frames_valid(frames: &[String]) -> bool {
    !frames.is_empty()
        && frames.len() <= 100
        && frames.iter().all(|frame| {
            frame.len() == 16
                && frame.bytes().all(|b| b.is_ascii_hexdigit())
                && (0..8).all(|i| {
                    let v = u8::from_str_radix(&frame[i * 2..i * 2 + 2], 16).unwrap();
                    if i < 4 {
                        v == 0 || (10..=240).contains(&v)
                    } else {
                        v <= 100
                    }
                })
        })
}
pub(crate) fn qr_data(url: &str) -> Result<String, String> {
    let qr = qrcode::QrCode::new(url.as_bytes())
        .map_err(|_| "配对地址过长，无法生成二维码".to_string())?;
    let svg = qr
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .build();
    Ok(format!(
        "data:image/svg+xml,{}",
        url::form_urlencoded::byte_serialize(svg.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v3_reverse_report_routes_are_supported() {
        let mut session = V3Session::default();
        session
            .receive(r#"{"type":"bind","clientId":"panel","targetId":"","message":"targetId"}"#)
            .unwrap();
        session
            .receive(r#"{"type":"bind","clientId":"panel","targetId":"app","message":"200"}"#)
            .unwrap();
        let events=session.receive(r#"{"type":"msg","clientId":"app","targetId":"panel","message":"strength-11+7+100+35"}"#).unwrap();
        assert!(matches!(events[0], DeviceEvent::Feedback { .. }));
        assert!(session.receive(r#"{"type":"msg","clientId":"other","targetId":"panel","message":"strength-11+7+100+35"}"#).is_err());
    }
    #[test]
    fn v3_pair_then_strength_feedback() {
        let mut session = V3Session::default();
        session
            .receive(r#"{"type":"bind","clientId":"panel","targetId":"","message":"targetId"}"#)
            .unwrap();
        assert!(session
            .pairing_url("wss://ws.dungeon-lab.cn/")
            .unwrap()
            .ends_with("/panel"));
        session
            .receive(r#"{"type":"bind","clientId":"panel","targetId":"app","message":"200"}"#)
            .unwrap();
        let events = session.receive(r#"{"type":"msg","clientId":"panel","targetId":"app","message":"strength-11+7+100+35"}"#).unwrap();
        assert!(
            matches!(&events[0], DeviceEvent::Feedback { strengths, limits } if *strengths == [11,7] && *limits == [100,35])
        );
    }
    #[test]
    fn v3_rejects_wrong_peer_and_invalid_feedback() {
        let mut session = V3Session::default();
        session
            .receive(r#"{"type":"bind","clientId":"panel","targetId":"","message":"targetId"}"#)
            .unwrap();
        session
            .receive(r#"{"type":"bind","clientId":"panel","targetId":"app","message":"200"}"#)
            .unwrap();
        for message in [
            "strength-11+7+100",
            "strength-11+7+100+201",
            "strength--1+7+100+35",
        ] {
            let text = serde_json::json!({"type":"msg","clientId":"panel","targetId":"app","message":message}).to_string();
            assert!(session.receive(&text).is_err());
        }
        assert!(session.receive(r#"{"type":"msg","clientId":"panel","targetId":"other","message":"strength-11+7+100+35"}"#).is_err());
        let command = DeviceCommand::SetStrength {
            channel: 0,
            value: 35,
        };
        let wire = session.encode(&command).unwrap();
        let wire: serde_json::Value = serde_json::from_str(&wire[0]).unwrap();
        assert_eq!(wire["type"], "msg");
        assert_eq!(wire["message"], "strength-1+2+35");
    }
    #[test]
    fn device_settings_reject_invalid_addresses() {
        let mut settings = DeviceSettings::default();
        settings.validate().unwrap();
        for address in [
            "http://example.com",
            "wss://example.com/path",
            "wss://user:pass@example.com/",
            "wss://example.com/?tid=x",
        ] {
            settings.v3_url = address.into();
            assert!(settings.validate().is_err(), "{address}");
        }
    }
}
