use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tracing::info;

static DAEMON_PROCESS: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
static IS_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn get_daemon_holder() -> &'static Mutex<Option<Child>> {
    DAEMON_PROCESS.get_or_init(|| Mutex::new(None))
}

/// 在后台启动 nova serve 守护进程服务
pub fn spawn_daemon(cli_exe_path: &Path, port: u16) -> Result<(), String> {
    if !cli_exe_path.exists() {
        return Err(format!("找不到 CLI 目标可执行文件: {}", cli_exe_path.display()));
    }

    let mut holder = get_daemon_holder().lock().map_err(|e| e.to_string())?;
    if holder.is_some() {
        info!("Nova Daemon 已经在运行中");
        return Ok(());
    }

    info!(exe = %cli_exe_path.display(), port = port, "正在以后台守护进程模式拉起 Nova CLI...");

    let mut cmd = Command::new(cli_exe_path);
    cmd.arg("serve").arg("--port").arg(port.to_string());

    // 在 Windows 下强制静默运行，彻底隐藏控制台黑框窗口 (CREATE_NO_WINDOW = 0x08000000)
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 Nova Daemon 失败: {}", e))?;

    info!(pid = child.id(), "Nova Daemon 守护进程已成功拉起");
    *holder = Some(child);

    Ok(())
}

/// 桌面端退出时同步杀掉后台守护进程，确保绝不留下孤儿僵尸进程
pub fn terminate_daemon() {
    if IS_SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }

    let mut holder = match get_daemon_holder().lock() {
        Ok(h) => h,
        Err(_) => return,
    };

    if let Some(mut child) = holder.take() {
        info!(pid = child.id(), "桌面宿主退出，正在优雅终止后台 Nova Daemon 进程...");
        let _ = child.kill();
        let _ = child.wait();
        info!("Nova Daemon 已退出");
    }
}
