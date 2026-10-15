use super::{
    frames_valid, valid_id, DeviceChoice, DeviceCommand, DeviceEvent, DeviceMode, DeviceState,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
#[derive(Clone)]
struct Remote {
    id: String,
    key: String,
    name: String,
    props: Value,
    state: Value,
}
struct Pending {
    deadline: u64,
    stop: bool,
    devices: bool,
}
#[derive(Default)]
pub struct V4Session {
    client: String,
    peer: String,
    devices: Vec<Remote>,
    selected: Option<String>,
    next_device: u64,
    next_req: u64,
    pending: HashMap<String, Pending>,
    stop_pending: HashSet<String>,
    stop_sent: bool,
}
fn slot_id(value: &Value) -> Result<String, String> {
    let id = value
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
        .ok_or("设备标识无效")?;
    Ok(id.into())
}
fn merge(target: &mut Value, patch: &Value, depth: usize) -> Result<(), String> {
    if depth > 8 {
        return Err("设备状态嵌套过深".into());
    }
    if let Some(object) = patch.as_object() {
        if !target.is_object() {
            *target = json!({});
        }
        let output = target.as_object_mut().unwrap();
        for (key, value) in object {
            merge(
                output.entry(key.clone()).or_insert(Value::Null),
                value,
                depth + 1,
            )?;
        }
    } else {
        *target = patch.clone();
    }
    Ok(())
}
fn number(value: &Value) -> Result<Option<u16>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_u64()
        .filter(|n| *n <= 200)
        .ok_or("APP 强度或上限超出范围")?;
    Ok(Some(number as u16))
}
impl V4Session {
    fn replace_devices(&mut self, devices: &[Value]) -> Result<(), String> {
        if devices.len() > 32 {
            return Err("设备数量超出支持范围".into());
        }
        let previous = std::mem::take(&mut self.devices);
        for value in devices {
            self.update(value, true)?;
            if let Some(current) = self.devices.last_mut() {
                if let Some(old) = previous.iter().find(|old| old.id == current.id) {
                    current.key = old.key.clone();
                }
            }
        }
        if !self
            .devices
            .iter()
            .any(|device| Some(&device.key) == self.selected.as_ref())
        {
            self.selected = if self.devices.len() == 1 {
                Some(self.devices[0].key.clone())
            } else {
                None
            };
        }
        Ok(())
    }
    pub fn pairing_url(&self, base: &str) -> Result<String, String> {
        if self.client.is_empty() {
            return Err("尚未取得配对信息".into());
        }
        let mut url = super::settings::validate_url(base, DeviceMode::App4)?;
        url.query_pairs_mut().append_pair("tid", &self.client);
        let mut qr = url::Url::parse("https://dungeon-lab.cn/s/").unwrap();
        qr.query_pairs_mut()
            .append_pair("v", "1")
            .append_pair("action", "socket")
            .append_pair("url", url.as_str());
        Ok(qr.into())
    }
    pub fn select_device(&mut self, key: &str) -> Result<(), String> {
        if !self.devices.iter().any(|device| device.key == key) {
            return Err("所选设备已离线，请重新选择".into());
        }
        self.selected = Some(key.into());
        Ok(())
    }
    pub fn device_state(&self) -> DeviceState {
        let choices = self
            .devices
            .iter()
            .map(|device| DeviceChoice {
                key: device.key.clone(),
                name: device.name.clone(),
            })
            .collect();
        let mut output = DeviceState {
            choices,
            selected_key: self.selected.clone(),
            ..Default::default()
        };
        if let Some(device) = self
            .devices
            .iter()
            .find(|device| Some(&device.key) == self.selected.as_ref())
        {
            output.strengths = [
                number(&device.props["intensityA"]).ok().flatten(),
                number(&device.props["intensityB"]).ok().flatten(),
            ];
            output.limits = [
                number(&device.state["channelA"]["intensityMax"])
                    .ok()
                    .flatten(),
                number(&device.state["channelB"]["intensityMax"])
                    .ok()
                    .flatten(),
            ];
            for (i, channel) in ["channelA", "channelB"].iter().enumerate() {
                if device.state[channel]["isMuted"] == true
                    || device.props[if i == 0 {
                        "channelAStatus"
                    } else {
                        "channelBStatus"
                    }]
                    .as_u64()
                    .is_some_and(|status| status == 3 || status == 4)
                {
                    output.limits[i] = Some(0);
                }
            }
            output.ready = device.state["hasDevice"] == true
                && device.props["connectState"] != "disconnected"
                && output.strengths.iter().all(Option::is_some)
                && output.limits.iter().all(Option::is_some);
        }
        output
    }
    fn update(&mut self, value: &Value, full: bool) -> Result<(), String> {
        let id = slot_id(&value["slotId"])?;
        if full {
            self.devices.retain(|device| device.id != id);
            if value["type"] != "COYOTE_030" {
                return Ok(());
            }
            self.next_device += 1;
            let name = value["name"]
                .as_str()
                .filter(|name| name.len() <= 256)
                .unwrap_or("郊狼 3.0")
                .into();
            self.devices.push(Remote {
                id: id.clone(),
                key: format!("device-{}", self.next_device),
                name,
                props: json!({}),
                state: json!({}),
            });
        }
        if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
            if value.get("props").is_some() {
                merge(&mut device.props, &value["props"], 0)?;
            }
            if value.get("slotState").is_some() {
                merge(&mut device.state, &value["slotState"], 0)?;
            }
            if device.props.to_string().len() + device.state.to_string().len() > 16384 {
                return Err("设备状态过大".into());
            }
            for item in [
                &device.props["intensityA"],
                &device.props["intensityB"],
                &device.state["channelA"]["intensityMax"],
                &device.state["channelB"]["intensityMax"],
            ] {
                number(item)?;
            }
        }
        if self.devices.len() > 32 {
            return Err("设备数量超出支持范围".into());
        }
        Ok(())
    }
    pub fn receive(&mut self, text: &str) -> Result<Vec<DeviceEvent>, String> {
        if text.len() > 65536 {
            return Err("设备协议消息过大".into());
        }
        let frame: Value = serde_json::from_str(text).map_err(|_| "设备协议消息格式无效")?;
        match frame["type"].as_str() {
            Some("hello") => {
                let client = frame["clientId"]
                    .as_str()
                    .filter(|id| valid_id(id))
                    .ok_or("设备会话标识无效")?;
                if !self.client.is_empty() {
                    return Err("设备会话重复初始化".into());
                }
                self.client = client.into();
                return Ok(vec![DeviceEvent::Assigned]);
            }
            Some("client_attached") => {
                let peer = frame["clientId"]
                    .as_str()
                    .filter(|id| valid_id(id))
                    .ok_or("APP 标识无效")?;
                if self.peer.is_empty() {
                    self.peer = peer.into();
                    return Ok(vec![DeviceEvent::Paired]);
                }
                return Ok(vec![]);
            }
            Some("client_disconnected") if frame["clientId"] == self.peer => {
                return Ok(vec![DeviceEvent::Disconnected])
            }
            Some("heartbeat" | "pong") => return Ok(vec![]),
            Some("error" | "idle_timeout") => {
                return Ok(vec![DeviceEvent::Error(
                    "APP 连接已失效，请重新连接并扫码".into(),
                )])
            }
            Some("message") => {}
            _ => return Ok(vec![]),
        }
        if self.peer.is_empty() || frame["clientId"] != self.peer {
            return Ok(vec![]);
        }
        let data = &frame["data"];
        if data["t"] == "resp" {
            let id = data["reqId"].as_str().ok_or("APP 回复标识无效")?;
            if let Some(pending) = self.pending.remove(id) {
                if data.get("error").is_some() {
                    return Err("APP 拒绝设备指令，输出已停用".into());
                }
                if pending.stop {
                    self.stop_pending.remove(id);
                }
                if pending.devices {
                    let devices = data["result"]["devices"]
                        .as_array()
                        .ok_or("APP 设备列表回复无效")?;
                    self.replace_devices(devices)?;
                    return Ok(vec![DeviceEvent::State(self.device_state())]);
                }
                return Ok(vec![DeviceEvent::RpcComplete {
                    stop_complete: self.stop_complete(),
                }]);
            }
            return Ok(vec![]);
        }
        if data["t"] != "ev" {
            return Ok(vec![]);
        }
        match data["ev"].as_str() {
            Some("devices.snapshot") => {
                let devices = data["devices"].as_array().ok_or("设备列表无效")?;
                self.replace_devices(devices)?;
            }
            Some("devices.patch") => {
                if let Some(removed) = data["removed"].as_array() {
                    for id in removed {
                        let id = slot_id(id)?;
                        self.devices.retain(|device| device.id != id);
                    }
                }
                if let Some(added) = data["added"].as_array() {
                    if added.len() > 32 {
                        return Err("设备数量超出支持范围".into());
                    }
                    for device in added {
                        self.update(device, true)?;
                    }
                }
            }
            Some("slots.patch") => {
                let slots = data["slots"].as_array().ok_or("设备更新无效")?;
                if slots.len() > 32 {
                    return Err("设备更新数量超出支持范围".into());
                }
                for slot in slots {
                    self.update(slot, false)?;
                }
            }
            _ => return Ok(vec![]),
        }
        if !self
            .devices
            .iter()
            .any(|device| Some(&device.key) == self.selected.as_ref())
        {
            self.selected = if self.devices.len() == 1 {
                Some(self.devices[0].key.clone())
            } else {
                None
            };
        }
        Ok(vec![DeviceEvent::State(self.device_state())])
    }
    pub fn stop_complete(&self) -> bool {
        self.stop_sent && self.stop_pending.is_empty()
    }
    pub fn check_timeouts(&self, now: u64) -> Result<(), String> {
        if self.pending.values().any(|pending| now >= pending.deadline) {
            Err("APP 指令回复超时，输出已停用".into())
        } else {
            Ok(())
        }
    }
    fn request(
        &mut self,
        method: &str,
        data: Option<Value>,
        now: u64,
        duration: u64,
        stop: bool,
    ) -> Result<String, String> {
        if self.peer.is_empty() {
            return Err("APP 尚未配对".into());
        }
        if self.pending.len() >= 32 {
            return Err("等待 APP 确认的指令过多，输出已停用".into());
        }
        self.next_req += 1;
        let req_id = format!("{}-{}", self.client, self.next_req);
        let mut payload = json!({"t":"req","reqId":req_id,"m":method});
        if let Some(data) = data {
            payload["data"] = data;
        }
        let text = json!({"type":"message","clientId":self.peer,"data":payload}).to_string();
        if text.len() > 65536 {
            return Err("设备指令过大".into());
        }
        self.pending.insert(
            req_id.clone(),
            Pending {
                deadline: now + duration + 3000,
                stop,
                devices: method == "devices.get",
            },
        );
        if stop {
            self.stop_pending.insert(req_id);
        }
        Ok(text)
    }
    pub fn encode(&mut self, command: &DeviceCommand, now: u64) -> Result<Vec<String>, String> {
        if matches!(command, DeviceCommand::RequestDevices) {
            return Ok(vec![self.request("devices.get", None, now, 0, false)?]);
        }
        let slot = self
            .devices
            .iter()
            .find(|device| Some(&device.key) == self.selected.as_ref())
            .map(|device| device.id.clone())
            .ok_or("请先选择郊狼 3.0 设备")?;
        let mut messages = vec![];
        if matches!(command, DeviceCommand::ResetAll) {
            self.stop_sent = true;
            self.stop_pending.clear();
            // 清理不等待普通任务回复，释放在途容量；迟到回复会被忽略。
            self.pending.clear();
            messages.push(self.request(
                "device.op.clear",
                Some(json!({"s":slot})),
                now,
                0,
                true,
            )?);
            for channel in 0..2 {
                messages.push(self.request(
                    "device.op",
                    Some(json!({"s":slot,"t":7,"c":channel,"p":2,"v":0})),
                    now,
                    0,
                    true,
                )?);
            }
            return Ok(messages);
        }
        let (method, data, duration) = match command {
            DeviceCommand::SetStrength { channel, value } if *channel < 2 && *value <= 200 => (
                "device.op",
                if *value == 0 {
                    json!({"s":slot,"t":7,"c":channel,"p":2,"v":0})
                } else {
                    json!({"s":slot,"t":4,"c":channel,"p":1,"d":2000,"im":true,"v":value})
                },
                if *value == 0 { 0 } else { 2000 },
            ),
            DeviceCommand::Pulse {
                channel,
                frames,
                duration_ms,
            } if *channel < 2 && frames_valid(frames) && *duration_ms <= 2000 => (
                "device.op",
                json!({"s":slot,"t":0,"c":channel,"p":1,"d":duration_ms,"im":true,"v":frames,"ver":3}),
                *duration_ms,
            ),
            DeviceCommand::Clear { channel } if *channel < 2 => {
                ("device.op.clear", json!({"s":slot,"c":channel}), 0)
            }
            _ => return Err("设备指令参数无效".into()),
        };
        self.stop_sent = false;
        messages.push(self.request(method, Some(data), now, duration, false)?);
        Ok(messages)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v4_filters_devices_and_merges_channel_limits() {
        let mut session = V4Session::default();
        session
            .receive(r#"{"type":"hello","clientId":"panel"}"#)
            .unwrap();
        session
            .receive(r#"{"type":"client_attached","clientId":"app"}"#)
            .unwrap();
        session.receive(r#"{"type":"message","clientId":"app","data":{"t":"ev","ev":"devices.snapshot","devices":[{"slotId":"toy","name":"其他","type":"OVC_1"},{"slotId":"coyote","name":"郊狼","type":"COYOTE_030","props":{"intensityA":11,"intensityB":7,"connectState":"connected"},"slotState":{"hasDevice":true,"channelA":{"intensityMax":100},"channelB":{"intensityMax":35}}}]}}"#).unwrap();
        let state = session.device_state();
        assert_eq!(state.choices.len(), 1);
        assert_eq!(state.limits, [Some(100), Some(35)]);
        session.receive(r#"{"type":"message","clientId":"app","data":{"t":"ev","ev":"slots.patch","slots":[{"slotId":"coyote","slotState":{"channelA":{"intensityMax":20}}}]}}"#).unwrap();
        assert_eq!(session.device_state().limits, [Some(20), Some(35)]);
        session.receive(r#"{"type":"message","clientId":"app","data":{"t":"ev","ev":"slots.patch","slots":[{"slotId":"coyote","slotState":{"hasDevice":false}}]}}"#).unwrap();
        assert!(!session.device_state().ready);
    }
    #[test]
    fn v4_late_response_and_long_task_do_not_block_reset() {
        let mut session = V4Session::default();
        session
            .receive(r#"{"type":"hello","clientId":"panel"}"#)
            .unwrap();
        session
            .receive(r#"{"type":"client_attached","clientId":"app"}"#)
            .unwrap();
        session.receive(r#"{"type":"message","clientId":"app","data":{"t":"ev","ev":"devices.snapshot","devices":[{"slotId":"coyote","name":"郊狼","type":"COYOTE_030","props":{"intensityA":0,"intensityB":0},"slotState":{"hasDevice":true,"channelA":{"intensityMax":100},"channelB":{"intensityMax":100}}}]}}"#).unwrap();
        let commands = session
            .encode(
                &DeviceCommand::SetStrength {
                    channel: 0,
                    value: 30,
                },
                0,
            )
            .unwrap();
        let command: serde_json::Value = serde_json::from_str(&commands[0]).unwrap();
        assert_eq!(command["data"]["data"]["t"], 4);
        assert_eq!(command["data"]["data"]["d"], 2000);
        assert_eq!(
            session.encode(&DeviceCommand::ResetAll, 20).unwrap().len(),
            3
        );
        assert!(session.receive(r#"{"type":"message","clientId":"app","data":{"t":"resp","reqId":"old","error":"invalid_operate"}}"#).unwrap().is_empty());
    }
}
