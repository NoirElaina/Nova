use std::fs;
use std::path::Path;

fn sync_bundled_cli() {
    #[cfg(target_os = "windows")]
    let exe_name = "nova.exe";
    #[cfg(not(target_os = "windows"))]
    let exe_name = "nova";

    let release_path = Path::new("../target/release").join(exe_name);
    let target_dir = Path::new("bin");
    let target_path = target_dir.join(exe_name);

    if release_path.exists() {
        let should_copy = if !target_path.exists() {
            true
        } else {
            let src_meta = fs::metadata(&release_path).ok();
            let dst_meta = fs::metadata(&target_path).ok();
            match (src_meta, dst_meta) {
                (Some(s), Some(d)) => {
                    s.len() != d.len()
                        || s.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                            > d.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                }
                _ => true,
            }
        };

        if should_copy {
            let _ = fs::create_dir_all(target_dir);
            if let Err(e) = fs::copy(&release_path, &target_path) {
                println!("cargo:warning=无法从 ../target/release 自动同步 CLI: {}", e);
            } else {
                println!(
                    "cargo:warning=已自动同步最新的 CLI 编译产物到 src-tauri/bin/{}",
                    exe_name
                );
            }
        }
    }
}

fn main() {
    sync_bundled_cli();
    // 显式声明对本仓库外产物的依赖：release 版 CLI 变化时必须重跑本脚本，
    // 否则 src-tauri/bin/nova.exe 会一直停留在旧版本，dev 模式会 spawn 到过期的内核。
    println!("cargo:rerun-if-changed=../target/release/nova.exe");
    println!("cargo:rerun-if-changed=../target/release/nova");
    tauri_build::build();
}
