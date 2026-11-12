use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// 获取全局标准化的 Nova CLI 安装目录
/// Windows:   %LOCALAPPDATA%\Nova\bin\nova.exe
/// Linux/Unix: ~/.nova/bin/nova
pub fn get_global_cli_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let local_appdata = env::var("LOCALAPPDATA")
            .map_err(|_| "无法获取 LOCALAPPDATA 环境变量".to_string())?;
        Ok(PathBuf::from(local_appdata).join("Nova").join("bin"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let home = env::var("HOME").map_err(|_| "无法获取 HOME 环境变量".to_string())?;
        Ok(PathBuf::from(home).join(".nova").join("bin"))
    }
}

pub fn get_global_cli_exe_path() -> Result<PathBuf, String> {
    let dir = get_global_cli_dir()?;
    #[cfg(target_os = "windows")]
    {
        Ok(dir.join("nova.exe"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(dir.join("nova"))
    }
}

/// 弹出 Windows 原生确认对话框（置顶置前显示）
#[cfg(target_os = "windows")]
fn show_confirm_dialog(title: &str, message: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let wide_title: Vec<u16> = OsStr::new(title).encode_wide().chain(Some(0)).collect();
    let wide_msg: Vec<u16> = OsStr::new(message).encode_wide().chain(Some(0)).collect();

    extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            lpText: *const u16,
            lpCaption: *const u16,
            uType: u32,
        ) -> i32;
    }

    const MB_YESNO: u32 = 0x00000004;
    const MB_ICONQUESTION: u32 = 0x00000020;
    const MB_TOPMOST: u32 = 0x00040000;
    const IDYES: i32 = 6;

    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide_msg.as_ptr(),
            wide_title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_TOPMOST,
        )
    };

    result == IDYES
}

/// 检查指定可执行文件当前是否被运行中的进程独占锁定
#[cfg(target_os = "windows")]
fn is_file_locked(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    match fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(e) => {
            let code = e.raw_os_error().unwrap_or(0);
            code == 32
                || code == 33
                || code == 5
                || e.kind() == std::io::ErrorKind::PermissionDenied
        }
    }
}

/// 强制静默终止运行中的旧版 nova.exe 进程
#[cfg(target_os = "windows")]
fn kill_running_cli_processes() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "nova.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    std::thread::sleep(std::time::Duration::from_millis(250));
}

/// 安全复制部署 CLI：直接覆盖写入；若目标文件被已有运行中进程锁定，采用 Windows 重命名回退无缝替换
fn safe_deploy_copy(source: &Path, target: &Path) -> Result<(), String> {
    if let Err(e) = fs::copy(source, target) {
        #[cfg(target_os = "windows")]
        {
            let old_backup = target.with_extension("exe.old");
            let _ = fs::remove_file(&old_backup);
            if fs::rename(target, &old_backup).is_ok() {
                if let Err(copy_err) = fs::copy(source, target) {
                    return Err(format!("重命名旧文件后复制新 CLI 失败: {}", copy_err));
                }
                let _ = fs::remove_file(&old_backup);
                return Ok(());
            }
        }
        return Err(format!("复制释放 CLI 文件失败: {}", e));
    }
    Ok(())
}

/// 自动在用户机器上部署 CLI：
/// 1. 桌面端启动时，直接强制无脑覆盖复制到全局目录 (%LOCALAPPDATA%\Nova\bin\nova.exe)，无需多余比对
/// 2. 自动将全局 bin 目录追加到用户系统 PATH 环境变量
pub fn auto_deploy_cli(bundled_cli_source: Option<&Path>) -> Result<PathBuf, String> {
    let global_dir = get_global_cli_dir()?;
    let target_exe = get_global_cli_exe_path()?;

    // 1. 创建全局目录
    if !global_dir.exists() {
        fs::create_dir_all(&global_dir)
            .map_err(|e| format!("创建全局 CLI 目录失败: {}", e))?;
    }

    // 2. 只要桌面端带有内置 CLI 源文件，每次启动直接强制覆写更新，确保与桌面端 100% 同步
    if let Some(source) = bundled_cli_source {
        if source.exists() {
            #[cfg(target_os = "windows")]
            if is_file_locked(&target_exe) {
                let confirmed = show_confirm_dialog(
                    "Nova 更新安装提示",
                    "检测到旧版 Nova 正在后台或终端中运行。\n\n是否立即退出并杀死旧进程，然后完成新版本覆盖安装？",
                );
                if confirmed {
                    info!("用户已确认关闭旧版 Nova 进程，正在终止旧进程并覆盖安装新版...");
                    kill_running_cli_processes();
                } else {
                    info!("用户取消了关闭旧版进程，保留当前运行中的版本");
                    return Ok(target_exe);
                }
            }

            info!(from = %source.display(), to = %target_exe.display(), "正在释放并强制同步部署 Nova CLI 到全局目录...");
            safe_deploy_copy(source, &target_exe)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target_exe, fs::Permissions::from_mode(0o755));
            }
            info!("Nova CLI 已成功部署/更新至全局路径");
        } else if !target_exe.exists() {
            return Err(format!("未找到内置 CLI 源文件: {}", source.display()));
        }
    } else if !target_exe.exists() {
        return Err("未指定内置 CLI 来源，无法完成首次释放部署".to_string());
    }

    // 3. 将全局 bin 目录注入到用户的环境变量 PATH 中
    register_dir_to_path(&global_dir)?;

    Ok(target_exe)
}

/// 将目录无侵入追加到系统用户 PATH 中
fn register_dir_to_path(dir: &Path) -> Result<(), String> {
    let dir_str = dir.to_string_lossy().to_string();

    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let env_key = hkcu
            .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
            .map_err(|e| format!("无法打开用户 Environment 注册表: {}", e))?;

        let current_path: String = env_key.get_value("Path").unwrap_or_default();
        let paths: Vec<&str> = current_path.split(';').map(|s| s.trim()).collect();

        // 检查是否已包含
        if paths.iter().any(|p| p.eq_ignore_ascii_case(&dir_str)) {
            info!(path = %dir_str, "Nova CLI 路径已在当前用户 PATH 中");
            return Ok(());
        }

        let new_path = if current_path.is_empty() {
            dir_str.clone()
        } else {
            format!("{};{}", current_path, dir_str)
        };

        env_key
            .set_value("Path", &new_path)
            .map_err(|e| format!("写入用户 PATH 失败: {}", e))?;

        info!(path = %dir_str, "已成功将 Nova 全局目录写入 Windows 用户环境变量 PATH！");
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Unix 上除 ~/.nova/bin 外，还在 ~/.local/bin 创建软链接
        if let Ok(home) = env::var("HOME") {
            let local_bin = PathBuf::from(home).join(".local").join("bin");
            if local_bin.exists() {
                let link_dest = local_bin.join("nova");
                let target_exe = get_global_cli_exe_path()?;
                if !link_dest.exists() {
                    use std::os::unix::fs::symlink;
                    let _ = symlink(&target_exe, &link_dest);
                    info!(path = %link_dest.display(), "已在 ~/.local/bin 创建 nova 软链接");
                }
            }
        }
    }

    Ok(())
}
