//! CLI 宿主：内核构建与启动初始化（无桌面、无 Mock，全部真实内核能力）。

use nova_core::{Core, CoreConfig};

/// 构建独立内核（优先使用 `NOVA_DATA_DIR`，否则与桌面端同源）。
pub fn build_core() -> anyhow::Result<Core> {
    let config = match std::env::var_os("NOVA_DATA_DIR").filter(|value| !value.is_empty()) {
        Some(dir) => {
            let root = std::path::PathBuf::from(dir);
            CoreConfig::new(root.clone())
                .with_cache_dir(root.join("cache"))
                .with_download_dir(root.join("downloads"))
        }
        None => CoreConfig::standalone().map_err(anyhow::Error::msg)?,
    };
    Ok(Core::new(config, 4096))
}

/// 启动初始化：主密钥、模型目录、设置预载。
/// 任何一步失败都只降级（打印警告），不阻断 CLI 启动。
pub async fn init_kernel(core: &Core) {
    if let Err(error) = nova_core::services::settings_secrets::init_master_key(core) {
        eprintln!("[init] 主密钥初始化失败: {}", error);
    }
    nova_core::agent::utils::model_context::init(core);
    let _ = nova_core::services::settings::load_settings(core);
    if let Err(error) = nova_core::services::workspace::default_workspace_root(core) {
        eprintln!("[init] 默认工作区准备失败: {}", error);
    }
}
