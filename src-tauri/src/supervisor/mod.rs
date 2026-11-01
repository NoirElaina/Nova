pub mod deploy;
pub mod process;

use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tracing::{info, warn};

/// 初始化桌面伴生守护子系统：
/// 1. 自动寻找桌面端安装包内置携带的 CLI 二进制
/// 2. 自动将其释放部署到系统固定全局目录 (%LOCALAPPDATA%\Nova\bin 或 ~/.nova/bin)
/// 3. 自动将全局目录写入用户 PATH，供终端任何地方随时使用
/// 4. 以后台服务模式拉起已部署好的全局 CLI 守护进程
pub fn init_desktop_supervisor(app: &AppHandle) {
    let mut candidates = Vec::new();

    // 1. 优先从 Tauri 安装包内置的 Resource 资源目录寻找（生产安装包形态）
    if let Ok(resource_dir) = app.path().resource_dir() {
        #[cfg(target_os = "windows")]
        candidates.push(resource_dir.join("bin").join("nova.exe"));
        #[cfg(not(target_os = "windows"))]
        candidates.push(resource_dir.join("bin").join("nova"));
    }

    // 2. 从当前应用同级目录寻找
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            #[cfg(target_os = "windows")]
            candidates.push(parent.join("nova.exe"));
            #[cfg(not(target_os = "windows"))]
            candidates.push(parent.join("nova"));
        }
    }

    // 3. 开发构建环境回退路径 (target/debug 或 target/release)
    #[cfg(target_os = "windows")]
    {
        candidates.push(PathBuf::from("target/release/nova.exe"));
        candidates.push(PathBuf::from("target/debug/nova.exe"));
    }
    #[cfg(not(target_os = "windows"))]
    {
        candidates.push(PathBuf::from("target/release/nova"));
        candidates.push(PathBuf::from("target/debug/nova"));
    }

    let bundled_source = candidates.into_iter().find(|p| p.exists());

    // 执行全局自释放部署与 PATH 注入
    match deploy::auto_deploy_cli(bundled_source.as_deref()) {
        Ok(deployed_exe) => {
            info!(path = %deployed_exe.display(), "Nova CLI 已就绪于全局路径，正在拉起后台服务...");
            if let Err(e) = process::spawn_daemon(&deployed_exe, 3080) {
                warn!(error = %e, "启动 Nova Daemon 守护进程失败");
            }
        }
        Err(e) => {
            warn!(error = %e, "自动部署 Nova CLI 失败，请检查安装文件完整性");
        }
    }
}

/// 桌面应用退出钩子：清理后台进程
pub fn on_desktop_exit() {
    process::terminate_daemon();
}
