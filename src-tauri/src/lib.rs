//! ManualBundler 核心库 —— 暴露打包引擎的所有模块。
//! main.rs (Tauri binary) 和 examples/test_bundle.rs (CLI 测试) 共用此 lib。

pub mod error;
pub mod model;
pub mod bundler;
pub mod commands;
pub mod mhtml;
pub mod docs;
