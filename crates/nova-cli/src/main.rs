mod headless;
mod host;
mod serve;
mod tui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nova", version, about = "Nova Agent 内核宿主：交互终端 / 无头执行 / 守护进程")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// 无头一次性执行（真实内核回合，流式输出）。
    #[arg(short = 'p', long)]
    prompt: Option<String>,

    /// 复用指定会话 ID（配合 -p）。
    #[arg(long)]
    conversation: Option<String>,

    /// 无头模式自动放行权限请求（缺省一律拒绝，安全默认）。
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动后台内核服务。
    ///
    /// 缺省以 stdio JSONL 提供服务（由桌面壳 spawn 并持有管道，不监听端口）；
    /// 加 --listen 切换为 WebSocket 远程模式。
    Serve {
        /// 启用 WebSocket 监听（远程 / 多客户端模式）
        #[arg(long)]
        listen: bool,

        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        #[arg(short, long, default_value_t = 3080)]
        port: u16,
    },
}

/// 守护进程的日志同时写 stderr 与文件。
///
/// stdio 模式下 stdout 是 JSONL 协议流，任何日志插进去都会破坏帧边界。
/// 而桌面壳以 GUI 方式拉起本进程时 stderr 没有控制台，日志会整段丢失——
/// 所以必须落文件，否则排查只能靠猜。
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let mut guard: Option<tracing_appender::non_blocking::WorkerGuard> = None;

    let file_layer = kernel_log_path().and_then(|path| {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        let (non_blocking, worker) = tracing_appender::non_blocking(file);
        guard = Some(worker);
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking),
        )
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .init();

    guard
}

/// 内核日志路径：与应用数据目录同源。
fn kernel_log_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".local").join("share"));

    base.map(|dir| {
        dir.join("com.tauri-app.nova")
            .join("logs")
            .join("kernel.log")
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { listen, host, port }) => {
            let _log_guard = init_tracing();
            if listen {
                serve::run(&host, port).await
            } else {
                serve::run_stdio().await
            }
        }
        None => {
            if let Some(prompt) = cli.prompt {
                headless::run(headless::HeadlessOptions {
                    prompt,
                    conversation_id: cli.conversation,
                    yes: cli.yes,
                })
                .await
            } else {
                let app = tui::TuiApp::new().await?;
                app.run().await
            }
        }
    }
}
