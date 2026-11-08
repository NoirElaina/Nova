//! Windows 受限令牌沙箱验证：令牌 + ACL 授权后的 workspace-write 语义。
//!
//! 运行：`cargo run -p nova-core --example win_sandbox_probe`

#[cfg(windows)]
fn main() {
    use nova_core::agent::sandbox::windows;
    use std::path::Path;

    let cmd = "C:\\Windows\\System32\\cmd.exe";
    let data_dir = std::env::temp_dir().join("nova-sandbox-probe");
    let workspace = std::env::temp_dir().join("nova-sandbox-ws");
    if let Err(error) = std::fs::create_dir_all(&workspace) {
        println!("准备测试工作区失败: {error}");
        return;
    }

    println!("[1] 当前用户 SID");
    match windows::current_user_sid() {
        Ok(sid) => println!("    {sid}"),
        Err(error) => println!("    失败: {error}"),
    }

    println!("[2] 为工作区分配合成 SID");
    let cap_sid = match windows::workspace_write_sid(&data_dir, &workspace) {
        Ok(sid) => {
            println!("    {sid}");
            sid
        }
        Err(error) => {
            println!("    失败: {error}");
            return;
        }
    };

    println!("[3] 给工作区授予该 SID 完全控制");
    if let Err(error) = windows::grant_workspace_write(&workspace, &cap_sid) {
        println!("    失败: {error}");
        return;
    }
    println!("    ok  ws={}", workspace.display());

    println!("[4] 创建写限制令牌");
    let token = match windows::create_write_restricted_token(&[cap_sid.clone()]) {
        Ok(token) => {
            println!("    ok");
            token
        }
        Err(error) => {
            println!("    失败: {error}");
            return;
        }
    };

    println!("[5] 写工作区（应成功）");
    let ws_file = workspace.join("probe.txt");
    let _ = std::fs::remove_file(&ws_file);
    let _ = windows::spawn_restricted(
        token,
        cmd,
        &["/C", &format!("echo probe>\"{}\"", ws_file.display())],
        Some(&workspace),
        5000,
    );
    println!("    exists={}（应为 true）", ws_file.exists());

    println!("[6] 写家目录（应失败）");
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let home_file = format!("{}\\nova_sandbox_probe.txt", home);
    let _ = std::fs::remove_file(&home_file);
    let _ = windows::spawn_restricted(
        token,
        cmd,
        &["/C", &format!("echo probe>\"{home_file}\"")],
        Some(&workspace),
        5000,
    );
    println!(
        "    exists={}（应为 false）",
        Path::new(&home_file).exists()
    );

    let _ = std::fs::remove_file(&ws_file);
    let _ = std::fs::remove_file(&home_file);
}

#[cfg(not(windows))]
fn main() {
    println!("仅 Windows 可用");
}
