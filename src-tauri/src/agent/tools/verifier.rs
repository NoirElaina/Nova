use std::path::Path;

/// 代码修改后的即时语法与规范校验器（Self-Healing Verifier）
/// 在文件被修改后，自动执行即时静态验证，若有语法错误即刻拦截并回灌给模型。
pub struct CodeVerifier;

impl CodeVerifier {
    /// 对修改的文件进行即时语法有效性检查
    pub async fn verify_file_content(path: &Path, content: &str) -> Result<(), String> {
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        
        match extension {
            "rs" => {
                // 利用 syn 解析 Rust 源码语法树
                if let Err(err) = syn::parse_file(content) {
                    return Err(format!(
                        "Rust 语法解析错误 (文件: {}): {}",
                        path.display(),
                        err
                    ));
                }
            }
            "json" => {
                if let Err(err) = serde_json::from_str::<serde_json::Value>(content) {
                    return Err(format!(
                        "JSON 语法格式错误 (文件: {}): {}",
                        path.display(),
                        err
                    ));
                }
            }
            "toml" => {
                if let Err(err) = toml::from_str::<toml::Value>(content) {
                    return Err(format!(
                        "TOML 格式配置错误 (文件: {}): {}",
                        path.display(),
                        err
                    ));
                }
            }
            _ => {
                // 其他类型文件暂不做静态语法树拦截
            }
        }

        Ok(())
    }
}
