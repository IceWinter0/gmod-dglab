use super::{frames_valid, valid_id, DeviceCommand, DeviceEvent, DeviceMode};
use serde::Deserialize;
use serde_json::json;
#[derive(Default)]
pub struct V3Session {
    client: String,
    peer: String,
}
#[derive(Deserialize)]
struct Frame {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "clientId")]
    client: String,
    #[serde(rename = "targetId")]
    peer: String,
    message: String,
}
impl V3Session {
    pub fn receive(&mut self, text: &str) -> Result<Vec<DeviceEvent>, String> {
        if text.len() > 65536 {
            return Err("设备协议消息过大".into());
        }
        let frame: Frame = serde_json::from_str(text).map_err(|_| "设备协议消息格式无效")?;
        if frame.kind == "bind" && frame.peer.is_empty() && frame.message == "targetId" {
            if !valid_id(&frame.client) || !self.client.is_empty() {
                return Err("设备会话标识无效".into());
            }
            self.client = frame.client;
            return Ok(vec![DeviceEvent::Assigned]);
        }
        let reverse_report = frame.kind == "msg"
            && !self.peer.is_empty()
            && frame.client == self.peer
            && frame.peer == self.client;
        if (frame.client != self.client && !reverse_report) || self.client.is_empty() {
            return Err("收到其他设备会话的消息".into());
        }
        if frame.kind == "heartbeat" {
            return Ok(vec![]);
        }
        if frame.kind == "bind" && frame.message == "200" {
            if !valid_id(&frame.peer) || (!self.peer.is_empty() && self.peer != frame.peer) {
                return Err("APP 配对信息无效".into());
            }
            self.peer = frame.peer;
            return Ok(vec![DeviceEvent::Paired]);
        }
        if frame.kind == "error" || (frame.kind == "bind" && frame.message != "200") {
            return Ok(vec![DeviceEvent::Error(
                match frame.message.as_str() {
                    "400" => "配对已被占用",
                    "401" => "配对已失效，请重新扫码",
                    "402" => "APP 配对已失效",
                    "404" => "APP 已离线",
                    _ => "设备协议请求失败",
                }
                .into(),
            )]);
        }
        if self.peer.is_empty() || (frame.peer != self.peer && !reverse_report) {
            return Err("收到其他 APP 的消息".into());
        }
        if frame.kind == "break" {
            return Ok(vec![DeviceEvent::Disconnected]);
        }
        if let Some(values) = frame.message.strip_prefix("strength-") {
            let values: Vec<u16> = values
                .split('+')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .map_err(|_| "APP 强度回传无效")?;
            if values.len() != 4 || values.iter().any(|v| *v > 200) {
                return Err("APP 强度或上限超出范围".into());
            }
            return Ok(vec![DeviceEvent::Feedback {
                strengths: [values[0], values[1]],
                limits: [values[2], values[3]],
            }]);
        }
        Ok(vec![])
    }
    pub fn pairing_url(&self, base: &str) -> Result<String, String> {
        if self.client.is_empty() {
            return Err("尚未取得配对信息".into());
        }
        let mut url = super::settings::validate_url(base, DeviceMode::App3)?;
        url.set_path(&format!("/{}", self.client));
        Ok(format!(
            "https://www.dungeon-lab.com/app-download.php#DGLAB-SOCKET#{url}"
        ))
    }
    pub fn encode(&self, command: &DeviceCommand) -> Result<Vec<String>, String> {
        if self.peer.is_empty() {
            return Err("APP 尚未配对".into());
        }
        let messages = match command {
            DeviceCommand::SetStrength { channel, value } if *channel < 2 && *value <= 200 => {
                vec![format!("strength-{}+2+{value}", channel + 1)]
            }
            DeviceCommand::Clear { channel } if *channel < 2 => {
                vec![format!("clear-{}", channel + 1)]
            }
            DeviceCommand::Pulse {
                channel, frames, ..
            } if *channel < 2 && frames_valid(frames) => vec![format!(
                "pulse-{}:{}",
                if *channel == 0 { "A" } else { "B" },
                serde_json::to_string(frames).unwrap()
            )],
            DeviceCommand::ResetAll => vec![
                "clear-1".into(),
                "clear-2".into(),
                "strength-1+2+0".into(),
                "strength-2+2+0".into(),
            ],
            _ => return Err("设备指令参数无效".into()),
        };
        messages.into_iter().map(|message| {
            let text=json!({"type":"msg","clientId":self.client,"targetId":self.peer,"message":message}).to_string();
            if text.chars().count()>1950 {Err("设备指令超过协议长度限制".into())}else{Ok(text)}
        }).collect()
    }
}
