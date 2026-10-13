use gmod_dglab_common::{
    protocol::{decode_message, Message, MAX_MESSAGE_BYTES},
    session::SessionGate,
};
use std::{
    io,
    net::TcpStream,
    time::{Duration, Instant},
};
use tungstenite::{accept_with_config, protocol::WebSocketConfig};

pub fn receive(stream: TcpStream, token: String) -> io::Result<usize> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let mut ws = accept_with_config(stream, Some(config))
        .map_err(|_| io::Error::other("WebSocket 握手失败"))?;
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(500)))?;
    let started = Instant::now();
    let mut gate = SessionGate::new(token);
    let mut authenticated = false;
    let mut count = 0;
    loop {
        let now = started.elapsed().as_millis() as u64;
        if (authenticated && !gate.is_alive(now)) || (!authenticated && now >= 3000) {
            return Err(io::Error::other("认证或心跳超时"));
        }
        let frame = match ws.read() {
            Ok(frame) => frame,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(tungstenite::Error::ConnectionClosed) => return Ok(count),
            Err(_) => return Err(io::Error::other("游戏连接断开或消息无效")),
        };
        let text = match frame {
            tungstenite::Message::Text(text) => text,
            tungstenite::Message::Close(_) => {
                let _ = ws.flush();
                return Ok(count);
            }
            tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => {
                let _ = ws.flush();
                continue;
            }
            _ => return Err(io::Error::other("仅接受 JSON 文本消息")),
        };
        let message =
            decode_message(text.as_bytes()).map_err(|_| io::Error::other("协议消息校验失败"))?;
        gate.accept(&message, started.elapsed().as_millis() as u64)
            .map_err(|_| io::Error::other("会话、凭据、序号或消息时效无效"))?;
        count += 1;
        match message {
            Message::Hello { module_id, .. } => {
                authenticated = true;
                println!("游戏已认证：{module_id}");
                ws.send(tungstenite::Message::Text(
                    r#"{"type":"hello_ok","version":1}"#.into(),
                ))
                .map_err(|_| io::Error::other("认证回复失败"))?;
            }
            Message::Heartbeat { seq, .. } => println!("游戏心跳：{seq}"),
            Message::Event { data, .. } => println!("游戏事件：{data:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn run(token: &str) -> (Result<usize, String>, bool) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            receive(s, "a".repeat(64)).map_err(|e| e.to_string())
        });
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://{address}/"), stream).unwrap();
        let hello = Message::Hello {
            version: 1,
            session_id: "test".into(),
            token: token.into(),
            module_id: "diagnostic-only".into(),
        };
        ws.send(tungstenite::Message::Text(
            serde_json::to_string(&hello).unwrap().into(),
        ))
        .unwrap();
        let accepted = ws
            .read()
            .is_ok_and(|m| m.to_text().is_ok_and(|s| s.contains("hello_ok")));
        if accepted {
            ws.send(tungstenite::Message::Text(
                r#"{"type":"heartbeat","version":1,"session_id":"test","seq":1}"#.into(),
            ))
            .unwrap();
            ws.close(None).unwrap();
        }
        (worker.join().unwrap(), accepted)
    }
    #[test]
    fn actual_websocket_authentication_and_heartbeat() {
        let (result, accepted) = run(&"a".repeat(64));
        assert!(accepted);
        assert_eq!(result.unwrap(), 2);
    }
    #[test]
    fn actual_websocket_rejects_wrong_credential() {
        let (result, accepted) = run(&"b".repeat(64));
        assert!(!accepted);
        assert!(result.is_err());
    }
}
