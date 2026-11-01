//! 内核配置：启动期一次性解析并校验的路径与运行参数。
//!
//! 路径在构造点即为已解析状态，访问器一律不可失败——
//! 任何"路径不存在"类错误都在宿主启动时暴露，而不是散落在业务调用点。

use std::path::PathBuf;

/// Tauri 应用标识（历史数据目录契约，必须保持不变以共享设置/会话/密钥）。
const APP_IDENTIFIER: &str = "com.tauri-app.nova";

#[derive(Debug, Clone)]
pub struct CoreConfig {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    download_dir: PathBuf,
    resource_dir: Option<PathBuf>,
}

impl CoreConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            cache_dir: data_dir.clone(),
            download_dir: data_dir.clone(),
            data_dir,
            resource_dir: None,
        }
    }

    /// 无桌面宿主（CLI / 守护进程）：按系统约定目录解析，与桌面端数据同源。
    pub fn standalone() -> Result<Self, String> {
        let data = dirs::data_dir()
            .ok_or_else(|| "无法解析系统数据目录".to_string())?
            .join(APP_IDENTIFIER);
        let mut config = Self::new(data);
        if let Some(cache) = dirs::cache_dir() {
            config.cache_dir = cache.join(APP_IDENTIFIER);
        }
        if let Some(download) = dirs::download_dir() {
            config.download_dir = download;
        }
        Ok(config)
    }

    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_dir = dir;
        self
    }

    pub fn with_download_dir(mut self, dir: PathBuf) -> Self {
        self.download_dir = dir;
        self
    }

    pub fn with_resource_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.resource_dir = dir;
        self
    }

    /// 应用数据目录（设置/会话数据库/技能/密钥）。
    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    /// 缓存目录（临时导出等）；缺省回退数据目录。
    pub fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    /// 用户下载目录（会话导出落盘）；缺省回退数据目录。
    pub fn download_dir(&self) -> PathBuf {
        self.download_dir.clone()
    }

    /// 打包资源目录（仅桌面安装包形态存在）。
    pub fn resource_dir(&self) -> Option<PathBuf> {
        self.resource_dir.clone()
    }
}
