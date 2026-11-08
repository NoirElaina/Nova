//! Windows 受限令牌沙箱验证：令牌能建、进程能起、写操作被收窄。
//!
//! 运行：`cargo run -p nova-core --example win_sandbox_probe`

#[cfg(windows)]
fn main() {
    use nova_core::agent::sandbox::windows;
    use std::path::Path;

    println!("[1] 当前用户 SID");
    match windows::current_user_sid() {
        Ok(sid) => println!("    {sid}"),
        Err(error) => println!("    失败: {error}"),
    }

    println!("[2] 为工作区分配合成 SID");
    let data_dir = std::env::temp_dir().join("nova-sandbox-probe");
    let workspace = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
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

    println!("[3] 创建写限制令牌");
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

    println!("[4] 用受限令牌启动 whoami");
    match windows::spawn_restricted(
        token,
        "C:\\Windows\\System32\\whoami.exe",
        &[],
        None,
        5000,
    ) {
        Ok(code) => println!("    exit={code}（能起就说明令牌+CreateProcessAsUserW 通过）"),
        Err(error) => println!("    失败: {error}"),
    }

    println!("[5] 用受限令牌尝试写家目录");
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let probe = format!("{}\\_nova_sandbox_probe.txt", home);
    match windows::spawn_restricted(
        token,
        "C:\\Windows\\System32\\cmd.exe",
        &["/C", &format!("echo probe>\"{probe}\"")],
        None,
        5000,
    ) {
        Ok(code) => println!("    cmd exit={code}"),
        Err(error) => println!("    失败: {error}"),
    }

    println!(
        "[6] 家目录探测文件存在 = {}（应为 false）",
        Path::new(&probe).exists()
    );
    let _ = std::fs::remove_file(&probe);
}

#[cfg(not(windows))]
fn main() {
    println!("仅 Windows 可用");
}
