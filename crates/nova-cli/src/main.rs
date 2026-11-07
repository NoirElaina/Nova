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
    /// 启动后台守护服务（桌面端与远程客户端经 WebSocket JSON-RPC 接入）。
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        #[arg(short, long, default_value_t = 3080)]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { host, port }) => {
            tracing_subscriber::fmt::init();
            serve::run(&host, port).await
        }
        None => {
            if let Some(prompt) = cli.prompt {
                let core = host::build_core()?;
                host::init_kernel(&core).await;
                headless::run(
                    core,
                    headless::HeadlessOptions {
                        prompt,
                        conversation_id: cli.conversation,
                        yes: cli.yes,
                    },
                )
                .await
            } else {
                let app = tui::TuiApp::new().await?;
                app.run().await
            }
        }
    }
}
