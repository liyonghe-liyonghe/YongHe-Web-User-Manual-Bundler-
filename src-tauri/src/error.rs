use thiserror::Error;
use serde::Serialize;

#[derive(Error, Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    #[error("扫描错误: {0}")]
    ScanError(String),

    #[error("打包错误: {0}")]
    BundleError(String),

    #[error("项目IO错误: {0}")]
    ProjectIOError(String),

    #[error("无效配置: {0}")]
    InvalidConfig(String),
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::BundleError(format!("IO 错误: {}", e))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::ProjectIOError(format!("JSON 错误: {}", e))
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::BundleError(s)
    }
}
