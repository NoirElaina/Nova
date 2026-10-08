//! 本机终端与 Shell 自动探测服务。
//!
//! 支持自动探测 Windows / macOS / Linux 下已安装的多种终端，
//! 供用户终端 PTY 与智能体 Shell 会话统一使用，并在系统设置中提供可视化选择。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    /// 唯一标识，如 "pwsh", "powershell", "git-bash", "cmd", "wsl", "zsh", "bash", "fish"
    pub id: String,
    /// 显示名称，如 "PowerShell 7", "Windows PowerShell", "Git Bash"
    pub name: String,
    /// 可执行文件绝对路径
    pub path: String,
    /// 本机是否存在该终端
    pub is_available: bool,
    /// 是否为推荐默认项
    pub is_recommended: bool,
    /// 终端类型分类："powershell" | "cmd" | "bash" | "other"
    pub shell_type: String,
}

/// 检查候选路径列表，返回首个存在的有效文件路径
fn find_first_existing(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// 在系统 PATH 中查找指定可执行程序
fn find_in_path(executable_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let full = dir.join(executable_name);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

/// 获取本机所有支持探测的终端列表
pub fn list_available_terminals() -> Vec<TerminalInfo> {
    let mut list = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();

        // 1. PowerShell 7 (pwsh) - 推荐
        let pwsh_candidates = vec![
            PathBuf::from(format!("{}\\PowerShell\\7\\pwsh.exe", program_files)),
            PathBuf::from(format!("{}\\PowerShell\\6\\pwsh.exe", program_files)),
            PathBuf::from(format!("{}\\Microsoft\\PowerShell\\7\\pwsh.exe", local_app_data)),
        ];
        let pwsh_path = find_first_existing(&pwsh_candidates)
            .or_else(|| find_in_path("pwsh.exe"));
        let pwsh_available = pwsh_path.is_some();
        list.push(TerminalInfo {
            id: "pwsh".to_string(),
            name: "PowerShell 7".to_string(),
            path: pwsh_path.map(|p| p.display().to_string()).unwrap_or_else(|| format!("{}\\PowerShell\\7\\pwsh.exe", program_files)),
            is_available: pwsh_available,
            is_recommended: pwsh_available,
            shell_type: "powershell".to_string(),
        });

        // 2. Windows PowerShell (powershell.exe) - Windows 自带
        let win_ps_path = PathBuf::from(format!("{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe", system_root));
        let win_ps_available = win_ps_path.is_file();
        list.push(TerminalInfo {
            id: "powershell".to_string(),
            name: "Windows PowerShell".to_string(),
            path: win_ps_path.display().to_string(),
            is_available: win_ps_available,
            is_recommended: !pwsh_available && win_ps_available,
            shell_type: "powershell".to_string(),
        });

        // 3. Git Bash
        let git_bash_candidates = vec![
            PathBuf::from(format!("{}\\Git\\bin\\bash.exe", program_files)),
            PathBuf::from(format!("{}\\Git\\usr\\bin\\bash.exe", program_files)),
            PathBuf::from(format!("{}\\Programs\\Git\\bin\\bash.exe", local_app_data)),
        ];
        let git_bash_path = find_first_existing(&git_bash_candidates)
            .or_else(|| find_in_path("bash.exe"));
        let git_bash_available = git_bash_path.is_some();
        list.push(TerminalInfo {
            id: "git-bash".to_string(),
            name: "Git Bash".to_string(),
            path: git_bash_path.map(|p| p.display().to_string()).unwrap_or_else(|| format!("{}\\Git\\bin\\bash.exe", program_files)),
            is_available: git_bash_available,
            is_recommended: false,
            shell_type: "bash".to_string(),
        });

        // 4. Command Prompt (cmd.exe)
        let cmd_path = PathBuf::from(format!("{}\\System32\\cmd.exe", system_root));
        let cmd_available = cmd_path.is_file();
        list.push(TerminalInfo {
            id: "cmd".to_string(),
            name: "Command Prompt (cmd.exe)".to_string(),
            path: cmd_path.display().to_string(),
            is_available: cmd_available,
            is_recommended: false,
            shell_type: "cmd".to_string(),
        });

        // 5. WSL (wsl.exe)
        let wsl_path = PathBuf::from(format!("{}\\System32\\wsl.exe", system_root));
        let wsl_available = wsl_path.is_file();
        if wsl_available {
            list.push(TerminalInfo {
                id: "wsl".to_string(),
                name: "WSL (Windows Subsystem for Linux)".to_string(),
                path: wsl_path.display().to_string(),
                is_available: true,
                is_recommended: false,
                shell_type: "bash".to_string(),
            });
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let user_shell = std::env::var("SHELL").ok();

        // 1. Zsh
        let zsh_path = find_first_existing(&[
            PathBuf::from("/bin/zsh"),
            PathBuf::from("/usr/bin/zsh"),
            PathBuf::from("/opt/homebrew/bin/zsh"),
            PathBuf::from("/usr/local/bin/zsh"),
        ]).or_else(|| find_in_path("zsh"));
        let zsh_available = zsh_path.is_some();
        let zsh_is_default = user_shell.as_deref().map(|s| s.ends_with("zsh")).unwrap_or(true);
        list.push(TerminalInfo {
            id: "zsh".to_string(),
            name: "Zsh".to_string(),
            path: zsh_path.map(|p| p.display().to_string()).unwrap_or_else(|| "/bin/zsh".to_string()),
            is_available: zsh_available,
            is_recommended: zsh_is_default && zsh_available,
            shell_type: "bash".to_string(),
        });

        // 2. Bash
        let bash_path = find_first_existing(&[
            PathBuf::from("/bin/bash"),
            PathBuf::from("/usr/bin/bash"),
            PathBuf::from("/opt/homebrew/bin/bash"),
            PathBuf::from("/usr/local/bin/bash"),
        ]).or_else(|| find_in_path("bash"));
        let bash_available = bash_path.is_some();
        let bash_is_default = user_shell.as_deref().map(|s| s.ends_with("bash")).unwrap_or(false);
        list.push(TerminalInfo {
            id: "bash".to_string(),
            name: "Bash".to_string(),
            path: bash_path.map(|p| p.display().to_string()).unwrap_or_else(|| "/bin/bash".to_string()),
            is_available: bash_available,
            is_recommended: bash_is_default || (!zsh_available && bash_available),
            shell_type: "bash".to_string(),
        });

        // 3. Fish
        let fish_path = find_first_existing(&[
            PathBuf::from("/usr/bin/fish"),
            PathBuf::from("/usr/local/bin/fish"),
            PathBuf::from("/opt/homebrew/bin/fish"),
        ]).or_else(|| find_in_path("fish"));
        if let Some(path) = fish_path {
            list.push(TerminalInfo {
                id: "fish".to_string(),
                name: "Fish".to_string(),
                path: path.display().to_string(),
                is_available: true,
                is_recommended: user_shell.as_deref().map(|s| s.ends_with("fish")).unwrap_or(false),
                shell_type: "other".to_string(),
            });
        }
    }

    list
}

/// 解析用户选择或自动推荐的活跃终端配置
pub fn resolve_active_terminal(configured_id_or_path: Option<&str>) -> TerminalInfo {
    let available = list_available_terminals();

    // 1. 如果用户显式配置了 id 或 path
    if let Some(target) = configured_id_or_path.map(str::trim).filter(|s| !s.is_empty()) {
        // 先按 ID 匹配已探测可用项
        if let Some(found) = available.iter().find(|t| t.is_available && t.id.eq_ignore_ascii_case(target)) {
            return found.clone();
        }
        // 再按 Path 匹配已探测可用项
        if let Some(found) = available.iter().find(|t| t.is_available && t.path.eq_ignore_ascii_case(target)) {
            return found.clone();
        }
        // 如果是自定义完整路径且真实存在
        let custom_path = Path::new(target);
        if custom_path.is_file() {
            let filename = custom_path.file_name().and_then(|n| n.to_str()).unwrap_or("custom");
            let shell_type = if filename.contains("pwsh") || filename.contains("powershell") {
                "powershell"
            } else if filename.contains("cmd") {
                "cmd"
            } else {
                "bash"
            };
            return TerminalInfo {
                id: "custom".to_string(),
                name: format!("Custom ({})", filename),
                path: target.to_string(),
                is_available: true,
                is_recommended: false,
                shell_type: shell_type.to_string(),
            };
        }
    }

    // 2. 回退到推荐项
    if let Some(rec) = available.iter().find(|t| t.is_available && t.is_recommended) {
        return rec.clone();
    }

    // 3. 回退到任意首个可用项
    if let Some(first_avail) = available.iter().find(|t| t.is_available) {
        return first_avail.clone();
    }

    // 4. 极端兜底（本机没有任何标准终端）
    #[cfg(target_os = "windows")]
    {
        TerminalInfo {
            id: "cmd".to_string(),
            name: "Command Prompt".to_string(),
            path: "cmd.exe".to_string(),
            is_available: true,
            is_recommended: true,
            shell_type: "cmd".to_string(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        TerminalInfo {
            id: "sh".to_string(),
            name: "sh".to_string(),
            path: "/bin/sh".to_string(),
            is_available: true,
            is_recommended: true,
            shell_type: "bash".to_string(),
        }
    }
}

static CONFIGURED_TERMINAL: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// 设置当前用户配置的终端偏好 (id 或 path)
pub fn set_configured_terminal(terminal: Option<String>) {
    if let Ok(mut guard) = CONFIGURED_TERMINAL.write() {
        *guard = terminal;
    }
}

/// 获取当前用户配置的终端偏好
pub fn get_configured_terminal() -> Option<String> {
    CONFIGURED_TERMINAL.read().ok().and_then(|g| g.clone())
}

/// 获取当前全局生效的活跃终端
pub fn get_current_active_terminal() -> TerminalInfo {
    let configured = get_configured_terminal();
    resolve_active_terminal(configured.as_deref())
}

/// 专门为 PowerShell 任务执行（如 shell_sessions agent 交互）解析可用的 PowerShell 可执行文件
#[cfg(target_os = "windows")]
pub fn resolve_powershell_path() -> PathBuf {
    let active = get_current_active_terminal();
    if active.shell_type == "powershell" && active.is_available {
        return PathBuf::from(active.path);
    }

    let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());

    let candidates = vec![
        PathBuf::from(format!("{}\\PowerShell\\7\\pwsh.exe", program_files)),
        PathBuf::from(format!("{}\\PowerShell\\6\\pwsh.exe", program_files)),
        PathBuf::from(format!("{}\\Microsoft\\PowerShell\\7\\pwsh.exe", local_app_data)),
    ];
    if let Some(pwsh) = find_first_existing(&candidates).or_else(|| find_in_path("pwsh.exe")) {
        return pwsh;
    }

    let win_ps = PathBuf::from(format!("{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe", system_root));
    if win_ps.is_file() {
        return win_ps;
    }

    PathBuf::from("powershell.exe")
}

#[cfg(not(target_os = "windows"))]
pub fn resolve_powershell_path() -> PathBuf {
    PathBuf::from("sh")
}
