use crate::{
    protocol::{decode_message, Message, MAX_MESSAGE_BYTES},
    session::SessionGate,
};
use std::{
    io,
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub fn receive(
    stream: TcpStream,
    token: String,
    stop: &AtomicBool,
    on_message: impl FnMut(Message),
) -> io::Result<usize> {
    receive_checked(stream, token, stop, |_| Ok(()), on_message)
}
pub fn receive_checked(
    stream: TcpStream,
    token: String,
    stop: &AtomicBool,
    mut validate_hello: impl FnMut(&Message) -> io::Result<()>,
    mut on_message: impl FnMut(Message),
) -> io::Result<usize> {
    // 监听器为非阻塞模式时，接受的 socket 不能假定是阻塞模式。
    // 本接收循环依赖读超时，因此在握手前显式设置，避免时序相关失败。
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let mut ws = tungstenite::accept_with_config(stream, Some(config))
        .map_err(|_| io::Error::other("WebSocket 握手失败"))?;
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(500)))?;
    let started = Instant::now();
    let mut gate = SessionGate::new(token);
    let mut authenticated = false;
    let mut count = 0;
    while !stop.load(Ordering::Acquire) {
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
        if matches!(message, Message::Hello { .. }) {
            validate_hello(&message)?;
            ws.send(tungstenite::Message::Text(
                r#"{"type":"hello_ok","version":1}"#.into(),
            ))
            .map_err(|_| io::Error::other("认证回复失败"))?;
            authenticated = true;
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        count += 1;
        on_message(message);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{TcpListener, TcpStream},
        sync::atomic::AtomicBool,
    };
    #[test]
    fn actual_socket_stop_and_validated_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut seen = 0;
            let count = receive(stream, "a".repeat(64), &worker_stop, |_| {
                seen += 1;
                ready_tx.send(()).unwrap();
            })
            .unwrap();
            (count, seen)
        });
        let (mut ws, _) =
            tungstenite::client(format!("ws://{addr}"), TcpStream::connect(addr).unwrap()).unwrap();
        ws.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"test","token":"{}","module_id":"test"}}"#, "a".repeat(64)).into())).unwrap();
        assert!(ws.read().unwrap().to_text().unwrap().contains("hello_ok"));
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        stop.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(worker.join().unwrap(), (1, 1));
    }
    #[test]
    fn actual_socket_rejects_duplicate_and_expired_events_before_callback() {
        for (age, duplicate, expected) in [(1001, false, 1), (0, true, 2)] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let worker = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut seen = 0;
                let result = receive(stream, "a".repeat(64), &AtomicBool::new(false), |_| {
                    seen += 1
                });
                (result.is_err(), seen)
            });
            let stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let (mut ws, _) = tungstenite::client(format!("ws://{addr}"), stream).unwrap();
            ws.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"test","token":"{}","module_id":"test"}}"#,"a".repeat(64)).into())).unwrap();
            ws.read().unwrap();
            let event = format!(
                r#"{{"type":"event","version":1,"session_id":"test","seq":1,"age_ms":{age},"data":{{"kind":"died"}}}}"#
            );
            ws.send(tungstenite::Message::Text(event.clone().into()))
                .unwrap();
            if duplicate {
                ws.send(tungstenite::Message::Text(event.into())).unwrap();
            }
            let _ = ws.read();
            assert_eq!(worker.join().unwrap(), (true, expected));
        }
    }
    #[test]
    fn nonblocking_accepted_socket_supports_delayed_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nonblocking(true).unwrap();
            ready_tx.send(()).unwrap();
            receive(stream, "a".repeat(64), &AtomicBool::new(false), |_| {})
        });
        let stream = TcpStream::connect(addr).unwrap();
        ready_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let (mut ws, _) = tungstenite::client(format!("ws://{addr}"), stream).unwrap();
        ws.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"test","token":"{}","module_id":"test"}}"#,"a".repeat(64)).into())).unwrap();
        ws.read().unwrap();
        ws.close(None).unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), 1);
    }

    #[test]
    fn checked_receiver_rejects_other_pid_before_ack() {
        for pid in [123, 456] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let worker = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                receive_checked(
                    stream,
                    "a".repeat(64),
                    &AtomicBool::new(false),
                    |m| match m {
                        Message::Hello {
                            process_id: Some(123),
                            ..
                        } => Ok(()),
                        _ => Err(io::Error::other("PID mismatch")),
                    },
                    |_| {},
                )
            });
            let stream = TcpStream::connect(addr).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let (mut ws, _) = tungstenite::client(format!("ws://{addr}"), stream).unwrap();
            ws.send(tungstenite::Message::Text(format!(r#"{{"type":"hello","version":1,"session_id":"s","token":"{}","module_id":"test","process_id":{pid}}}"#,"a".repeat(64)).into())).unwrap();
            let ack = ws.read();
            if pid == 123 {
                assert!(ack.unwrap().to_text().unwrap().contains("hello_ok"));
                ws.close(None).unwrap();
                assert!(worker.join().unwrap().is_ok());
            } else {
                assert!(ack.is_err());
                assert!(worker.join().unwrap().is_err());
            }
        }
    }
}
