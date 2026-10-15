#[cfg(not(all(windows, target_arch = "x86_64")))]
compile_error!("诊断程序仅支持 Windows x64");

fn main() {
    if let Err(error) = run() {
        eprintln!("诊断接收程序发生错误：{error}");
        std::process::exit(1);
    }
}
fn run() -> std::io::Result<()> {
    use gmod_dglab_common::runtime::{self, RuntimeConfig};
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:47831".into())
        .parse::<std::net::SocketAddr>()
        .map_err(|_| std::io::Error::other("地址格式应为 127.0.0.1:端口"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(std::io::Error::other("仅支持本机回环地址，端口范围为 1～65535"));
    }
    let listener = std::net::TcpListener::bind(address)?;
    let token = runtime::random_token()?;
    runtime::save_config(&RuntimeConfig {
        address,
        token: token.clone(),
    })?;
    println!("诊断接收程序已启动，监听地址：{address}。诊断模式仅接收游戏数据，不控制设备。");
    println!("请使用外部注入器将采样 DLL 加载到 GMod。连接凭据已加密保存，仅当前 Windows 用户可读取。");
    for stream in listener.incoming() {
        match stream
            .and_then(|stream| gmod_dglab_diagnostic::server::receive(stream, token.clone()))
        {
            Ok(count) => println!("连接结束，共接收 {count} 条有效消息。"),
            Err(error) => eprintln!("连接结束：{error}"),
        }
    }
    Ok(())
}
