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

/// 守护进程的日志一律写 stderr。
///
/// stdio 模式下 stdout 是 JSONL 协议流，任何一行日志插进去都会破坏帧边界，
/// 对端只会表现为「偶发丢消息」，排查成本极高。
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { listen, host, port }) => {
            init_tracing();
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
