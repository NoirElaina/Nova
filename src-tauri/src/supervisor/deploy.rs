use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

/// 获取全局标准化的 Nova CLI 安装目录
/// Windows:   %LOCALAPPDATA%\Nova\bin\nova.exe
/// Unix/macOS: ~/.nova/bin/nova
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

/// 自动在用户机器上部署 CLI：
/// 1. 若目标路径尚无 CLI，从桌面端自带资源或构建目录中释放/复制过去
/// 2. 自动将全局 bin 目录追加到用户系统 PATH 环境变量
pub fn auto_deploy_cli(bundled_cli_source: Option<&Path>) -> Result<PathBuf, String> {
    let global_dir = get_global_cli_dir()?;
    let target_exe = get_global_cli_exe_path()?;

    // 1. 创建全局目录
    if !global_dir.exists() {
        fs::create_dir_all(&global_dir)
            .map_err(|e| format!("创建全局 CLI 目录失败: {}", e))?;
    }

    // 2. 如果目标位置缺少可执行文件，从自带资源中复制部署
    if !target_exe.exists() {
        if let Some(source) = bundled_cli_source {
            if source.exists() {
                info!(from = %source.display(), to = %target_exe.display(), "正在释放并部署 Nova CLI 到全局目录...");
                fs::copy(source, &target_exe)
                    .map_err(|e| format!("复制释放 CLI 文件失败: {}", e))?;
                
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&target_exe, fs::Permissions::from_mode(0o755));
                }
                info!("Nova CLI 已成功释放部署至全局路径");
            } else {
                return Err(format!("未找到内置 CLI 源文件: {}", source.display()));
            }
        } else {
            return Err("未指定内置 CLI 来源，无法完成首次释放部署".to_string());
        }
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
