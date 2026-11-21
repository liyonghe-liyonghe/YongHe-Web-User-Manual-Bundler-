use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Text,
    Page,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavNode {
    #[serde(rename = "type", alias = "kind", default = "default_kind")]
    pub kind: NodeKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NavNode>,
}

fn default_kind() -> NodeKind { NodeKind::Text }

impl NavNode {
    pub fn text(name: impl Into<String>) -> Self {
        Self { kind: NodeKind::Text, name: name.into(), file: None, children: vec![] }
    }
    pub fn page(name: impl Into<String>, file: impl Into<String>) -> Self {
        Self { kind: NodeKind::Page, name: name.into(), file: Some(file.into()), children: vec![] }
    }
    pub fn walk_pages(&self, out: &mut Vec<(String, String)>) {
        if let NodeKind::Page = &self.kind {
            if let Some(f) = &self.file {
                out.push((f.clone(), self.name.clone()));
            }
        }
        for c in &self.children {
            c.walk_pages(out);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ShellStyle { A, B, C, D }

impl Default for ShellStyle { fn default() -> Self { ShellStyle::C } }

fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    /// 合并内嵌 CSS（0.8.0 起默认勾选，旧 .mproj 缺省时亦为 true）
    #[serde(default = "default_true")]
    pub merge_css: bool,
    /// 生成的手册外壳左侧导航栏可收起（默认开启，旧 .mproj 缺省时亦为 true）
    #[serde(default = "default_true")]
    pub collapsible_sidebar: bool,
    /// 输出手册辅助文字（左侧展开竖条、收起/展开导航提示等）语言。
    /// None 或 "auto" = 跟随界面语言（config.locale）；旧 .mproj 缺省时亦为 None。
    #[serde(default)]
    pub aux_lang: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            merge_css: true,
            collapsible_sidebar: true,
            aux_lang: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundlerConfig {
    #[serde(default)]
    pub input_dirs: Vec<String>,
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default = "default_breadcrumb")]
    pub breadcrumb_prefix: String,
    #[serde(default)]
    pub nav: Vec<NavNode>,
    #[serde(default)]
    pub shell_style: ShellStyle,
    #[serde(default)]
    pub options: Options,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
}

fn default_title() -> String { "泳和批量改名".into() }
fn default_breadcrumb() -> String { "泳和批量改名 / 用户手册 /".into() }

impl Default for BundlerConfig {
    fn default() -> Self {
        Self {
            input_dirs: vec![],
            title: default_title(),
            breadcrumb_prefix: default_breadcrumb(),
            nav: vec![],
            shell_style: ShellStyle::C,
            options: Options::default(),
            output_path: None,
            locale: None,
        }
    }
}

// --- 扫描结果 ---
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub pages: Vec<PageInfo>,
    pub dir_tree: Vec<DirNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageInfo {
    pub path: String,       // 相对输入目录
    pub title: String,
    pub input_dir_index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<DirNode>,
}

// --- 预览/打包结果 ---
#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub page_count: usize,
    pub embedded_resource_count: usize,
    pub final_bytes: usize,
}

/// 缺失的本地资源引用（页面 src/href 原始 URL + 解析时的期望绝对路径）
#[derive(Debug, Clone, Serialize)]
pub struct MissingResource {
    pub url: String,
    pub attempted: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewResult {
    pub html: String,
    pub stats: Stats,
    #[serde(default)]
    pub missing_resources: Vec<MissingResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub path: String,
    pub bytes: usize,
    pub stats: Stats,
}
