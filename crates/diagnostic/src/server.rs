use gmod_dglab_common::protocol::Message;
use std::{io, net::TcpStream, sync::atomic::AtomicBool};
pub fn receive(stream: TcpStream, token: String) -> io::Result<usize> {
    gmod_dglab_common::receiver::receive(stream, token, &AtomicBool::new(false), |message| {
        match message {
            Message::Hello { module_id, .. } => {
                println!("游戏连接成功，身份验证已通过：{module_id}");
            }
            Message::Heartbeat { seq, .. } => println!("游戏心跳：{seq}"),
            Message::Event { data, .. } => {
                use gmod_dglab_common::tracker::GameEvent;
                match data {
                    GameEvent::PlayerState {
                        sample: Some(player),
                        max_health,
                        thread_id,
                    } => println!(
                        "玩家状态：血量 {}，最大血量 {:?}，生命状态 {}，护甲 {:?}，线程 {}",
                        player.health, max_health, player.life_state, player.armor, thread_id
                    ),
                    GameEvent::PlayerState { sample: None, .. } => {
                        println!("玩家状态：暂无有效本地玩家")
                    }
                    GameEvent::HealthDecreased {
                        before,
                        after,
                        death_priority,
                    } => println!(
                        "血量下降：{before} → {after}{}",
                        if death_priority {
                            "（致命伤害，不再额外触发受伤联动）"
                        } else {
                            ""
                        }
                    ),
                    GameEvent::ArmorDecreased { before, after } => {
                        println!("护甲减少：{before} → {after}")
                    }
                    GameEvent::Died => println!("玩家已死亡"),
                    GameEvent::Respawned => println!("玩家已重生"),
                }
            }
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Duration;
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
            process_id: None,
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
