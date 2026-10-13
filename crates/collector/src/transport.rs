use crate::{sampling, STOP};
use gmod_dglab_common::{
    protocol::{Message, VERSION},
    runtime,
};
use std::{
    io,
    net::TcpStream,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub fn connect_and_heartbeat() -> io::Result<()> {
    sampling::EVENTS.clear();
    let generation = sampling::GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let config = runtime::load_config()?;
    let stream = TcpStream::connect_timeout(&config.address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let (mut socket, _) = tungstenite::client(format!("ws://{}/", config.address), stream)
        .map_err(|_| io::Error::other("本机 WebSocket 连接失败"))?;
    let hello = Message::Hello {
        version: VERSION,
        session_id: runtime::random_token()?,
        token: config.token,
        module_id: "windows-x64-player-sampling-preview".into(),
        process_id: Some(std::process::id()),
    };
    let session = match &hello {
        Message::Hello { session_id, .. } => session_id.clone(),
        _ => unreachable!(),
    };
    socket
        .send(tungstenite::Message::Text(
            serde_json::to_string(&hello)
                .map_err(io::Error::other)?
                .into(),
        ))
        .map_err(|_| io::Error::other("握手发送失败"))?;
    let response = socket
        .read()
        .map_err(|_| io::Error::other("未收到认证回复"))?;
    let accepted = response
        .to_text()
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .is_some_and(|v| v["type"] == "hello_ok" && v["version"] == 1);
    if !accepted {
        return Err(io::Error::other("诊断接收器拒绝连接"));
    }
    let mut seq = 1u64;
    let mut last_beat = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut last_report = Instant::now();
    while !STOP.load(Ordering::Acquire) {
        if last_beat.elapsed() >= Duration::from_secs(1) {
            let beat = Message::Heartbeat {
                version: VERSION,
                session_id: session.clone(),
                seq,
            };
            socket
                .send(tungstenite::Message::Text(
                    serde_json::to_string(&beat)
                        .map_err(io::Error::other)?
                        .into(),
                ))
                .map_err(|_| io::Error::other("心跳发送失败"))?;
            seq = seq
                .checked_add(1)
                .ok_or_else(|| io::Error::other("会话序号耗尽"))?;
            last_beat = Instant::now();
        }
        while let Some(event) = sampling::EVENTS.pop_generation(Instant::now(), generation) {
            let message = Message::Event {
                version: VERSION,
                session_id: session.clone(),
                seq,
                age_ms: event.at.elapsed().as_millis().min(u64::MAX as u128) as u64,
                data: event.data,
            };
            socket
                .send(tungstenite::Message::Text(
                    serde_json::to_string(&message)
                        .map_err(io::Error::other)?
                        .into(),
                ))
                .map_err(|_| io::Error::other("事件发送失败"))?;
            seq = seq
                .checked_add(1)
                .ok_or_else(|| io::Error::other("会话序号耗尽"))?;
        }
        if last_report.elapsed() >= Duration::from_secs(15) {
            crate::log(&sampling::statistics());
            last_report = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = socket.close(None);
    Ok(())
}
