#[cfg(not(all(windows, target_arch = "x86_64")))]
compile_error!("诊断程序仅支持 Windows x64");

fn main() {
    if let Err(error) = run() {
        eprintln!("诊断接收器失败：{error}");
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
        return Err(std::io::Error::other("仅允许本机有效端口"));
    }
    let listener = std::net::TcpListener::bind(address)?;
    let token = runtime::random_token()?;
    runtime::save_config(&RuntimeConfig {
        address,
        token: token.clone(),
    })?;
    println!("诊断接收器已启动：{address}；此版本不连接郊狼。");
    println!("请使用外部注入器加载诊断 DLL，凭据已使用 Windows 当前用户加密保存。");
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
