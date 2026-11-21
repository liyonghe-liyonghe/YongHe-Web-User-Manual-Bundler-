use crate::error::AppError;
use crate::model::*;
use crate::bundler;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

fn emit_log<R: tauri::Runtime>(app: &AppHandle<R>, level: &str, source: &str, message: impl Into<String>) {
    let _ = app.emit("mb-log", serde_json::json!({
        "level": level,
        "source": source,
        "message": message.into(),
    }));
}

/// 自动多线程：rayon 按可用核心数自动建池，这里仅取当前线程数用于日志展示
fn thread_pool_label() -> String {
    format!("{} 线程", rayon::current_num_threads())
}

#[tauri::command]
pub fn scan_folder<R: tauri::Runtime>(app: AppHandle<R>, path: String) -> Result<ScanResult, AppError> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        emit_log(&app, "ERROR", "扫描", format!("路径不存在: {}", path));
        return Err(AppError::ScanError(format!("路径不存在: {}", path)));
    }

    let mut pages: Vec<PageInfo> = vec![];
    let is_file = p.is_file();
    let (root_display, root_for_rel) = if is_file {
        // 文件场景：以父目录为"根"以便生成合理的相对路径
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        (p.file_name().and_then(|s| s.to_str()).unwrap_or("root").to_string(), parent.to_path_buf())
    } else {
        (p.file_name().and_then(|s| s.to_str()).unwrap_or("root").to_string(), p.clone())
    };

    let mut root = DirNode {
        name: root_display.clone(),
        path: path.clone(),
        is_dir: !is_file,
        children: vec![],
    };

    if is_file {
        // 单文件：仅接受可打包的页面类型（HTML/MHTML/图片/文档）
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !bundler::is_page_ext(ext) {
            let msg = format!(
                "不支持的文件类型: {}（支持 .html/.htm/.mht/.mhtml、常见图片及文档格式）",
                path
            );
            emit_log(&app, "ERROR", "扫描", msg.clone());
            return Err(AppError::ScanError(msg));
        }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string();
        let rel = p.strip_prefix(&root_for_rel).ok()
            .and_then(|r| r.to_str().map(|s| s.replace('\\', "/")))
            .unwrap_or_else(|| name.clone());
        let title = page_title(&p, &name);
        emit_log(&app, "INFO", "扫描", format!("扫描单文件: {}", rel));
        pages.push(PageInfo { path: rel.clone(), title, input_dir_index: 0 });
        root.children.push(DirNode { name, path: rel, is_dir: false, children: vec![] });
    } else {
        emit_log(&app, "INFO", "扫描", format!("开始扫描目录: {}（自动多线程: {}）", path, thread_pool_label()));
        // 先顺序走目录树收集页面（含占位标题），记录绝对路径供并行标题提取
        let mut page_paths: Vec<(usize, PathBuf, String)> = vec![];
        scan_recursive(&p, &p, 0, &mut pages, &mut root.children, &mut page_paths);
        // 标题提取可能涉及 MHTML 解析 / 文档整页转换，耗时 → 自动多线程并行
        let titles: Vec<String> = page_paths
            .par_iter()
            .map(|(_, abs, name)| page_title(abs, name))
            .collect();
        for ((idx, _, _), title) in page_paths.iter().zip(titles) {
            pages[*idx].title = title;
        }
    }

    emit_log(&app, "INFO", "扫描", format!("扫描完成: {} 页", pages.len()));
    Ok(ScanResult { pages, dir_tree: vec![root] })
}

fn scan_recursive(
    root: &Path,
    current: &Path,
    idx: usize,
    pages: &mut Vec<PageInfo>,
    children: &mut Vec<DirNode>,
    page_paths: &mut Vec<(usize, PathBuf, String)>,
) {
    let Ok(entries) = std::fs::read_dir(current) else { return };
    for ent in entries.flatten() {
        let p = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            let mut sub_children = vec![];
            scan_recursive(root, &p, idx, pages, &mut sub_children, page_paths);
            children.push(DirNode { name, path: p.to_string_lossy().to_string(), is_dir: true, children: sub_children });
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if bundler::is_page_ext(ext) {
                let rel = p.strip_prefix(root).ok()
                    .and_then(|r| r.to_str().map(|s| s.replace('\\', "/")))
                    .unwrap_or_else(|| name.clone());
                let page_idx = pages.len();
                pages.push(PageInfo { path: rel.clone(), title: String::new(), input_dir_index: idx });
                page_paths.push((page_idx, p, name.clone()));
                children.push(DirNode { name, path: rel, is_dir: false, children: vec![] });
            }
        }
    }
}

fn extract_title(p: &Path) -> Option<String> {
    let content = std::fs::read_to_string(p).ok()?;
    extract_title_from_html(&content)
}

fn extract_title_from_html(content: &str) -> Option<String> {
    if let Some(start) = content.find("<title") {
        let after = &content[start..];
        if let Some(close) = after.find('>') {
            let rest = &after[close+1..];
            if let Some(end) = rest.find("</title") {
                let t = rest[..end].trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

/// 按文件类型提取页面标题：图片/文档 → 文件名；MHTML → 解析后取 <title>；HTML → <title>
fn page_title(p: &Path, display_name: &str) -> String {
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    if bundler::is_image_ext(ext) || bundler::is_doc_ext(ext) {
        return p.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| display_name.to_string());
    }
    if bundler::is_mhtml_ext(ext) {
        if let Ok(html) = bundler::load_page_content(p, display_name) {
            if let Some(t) = crate::mhtml::extract_html_title(&html) {
                return t;
            }
        }
        return display_name.to_string();
    }
    extract_title(p).unwrap_or_else(|| display_name.to_string())
}

#[tauri::command]
pub fn preview_bundle<R: tauri::Runtime>(app: AppHandle<R>, config: BundlerConfig) -> Result<PreviewResult, AppError> {
    emit_log(&app, "INFO", "打包", format!("开始预览生成...（自动多线程: {}）", thread_pool_label()));
    let result = std::panic::catch_unwind(|| bundler::build_bundle(&config))
        .map_err(|_| AppError::BundleError("打包过程 panic".into()))??;
    // 资源内嵌失败等告警 → 日志面板
    for w in bundler::take_warnings() {
        emit_log(&app, "WARN", "打包", w);
    }
    emit_log(&app, "INFO", "打包", format!("预览生成完成: {} 页, {} 资源, {} bytes",
        result.stats.page_count, result.stats.embedded_resource_count, result.stats.final_bytes));
    Ok(result)
}

#[tauri::command]
pub fn write_bundle<R: tauri::Runtime>(app: AppHandle<R>, config: BundlerConfig, save_path: String) -> Result<WriteResult, AppError> {
    emit_log(&app, "INFO", "打包", format!("开始生成单文件到: {}（自动多线程: {}）", save_path, thread_pool_label()));
    let preview = std::panic::catch_unwind(|| bundler::build_bundle(&config))
        .map_err(|_| AppError::BundleError("打包过程 panic".into()))??;
    for w in bundler::take_warnings() {
        emit_log(&app, "WARN", "打包", w);
    }

    let save = PathBuf::from(&save_path);
    if let Some(parent) = save.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::BundleError(format!("创建输出目录失败: {}", e)))?;
        }
    }
    std::fs::write(&save, &preview.html).map_err(|e| AppError::BundleError(format!("写入输出文件失败 {}: {}", save_path, e)))?;

    let bytes = std::fs::metadata(&save).map(|m| m.len() as usize).unwrap_or(preview.stats.final_bytes);
    emit_log(&app, "INFO", "打包", format!("单文件生成完成: {} ({} bytes)", save_path, bytes));
    Ok(WriteResult { path: save_path, bytes, stats: preview.stats })
}

#[tauri::command]
pub fn save_project<R: tauri::Runtime>(app: AppHandle<R>, config: BundlerConfig, path: String) -> Result<(), AppError> {
    emit_log(&app, "INFO", "项目IO", format!("保存项目: {}", path));
    let json = serde_json::to_string_pretty(&config)?;
    std::fs::write(&path, &json)?;
    emit_log(&app, "INFO", "项目IO", "项目保存完成");
    Ok(())
}

#[tauri::command]
pub fn load_project<R: tauri::Runtime>(app: AppHandle<R>, path: String) -> Result<BundlerConfig, AppError> {
    emit_log(&app, "INFO", "项目IO", format!("加载项目: {}", path));
    let content = std::fs::read_to_string(&path)?;
    let config: BundlerConfig = serde_json::from_str(&content)?;
    emit_log(&app, "INFO", "项目IO", "项目加载完成");
    Ok(config)
}

#[tauri::command]
pub fn read_text_file<R: tauri::Runtime>(app: AppHandle<R>, path: String) -> Result<String, AppError> {
    emit_log(&app, "INFO", "文件IO", format!("读取: {}", path));
    let content = std::fs::read_to_string(&path)?;
    Ok(content)
}

#[tauri::command]
pub fn write_text_file<R: tauri::Runtime>(app: AppHandle<R>, path: String, content: String) -> Result<(), AppError> {
    emit_log(&app, "INFO", "文件IO", format!("写入: {}", path));
    let save = PathBuf::from(&path);
    if let Some(parent) = save.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok();
        }
    }
    std::fs::write(&path, &content)?;
    Ok(())
}

// ==================== 打开输出文件夹 ====================

/// 在系统文件管理器中打开输出文件所在文件夹：
/// 文件存在时选中该文件（Windows explorer /select），否则打开其父目录。
#[tauri::command]
pub fn open_output_folder<R: tauri::Runtime>(app: AppHandle<R>, path: String) -> Result<(), AppError> {
    let p = PathBuf::from(&path);
    emit_log(&app, "INFO", "输出", format!("打开输出文件夹: {}", path));

    #[cfg(target_os = "windows")]
    {
        // 规范化为绝对路径（explorer /select 需要反斜杠形式）
        let abs = std::fs::canonicalize(&p).unwrap_or(p.clone());
        let abs_str = abs.to_string_lossy().trim_start_matches(r"\\?\").replace('/', "\\");
        if abs.is_file() {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", abs_str))
                .spawn()
                .map_err(|e| AppError::BundleError(format!("打开文件夹失败: {}", e)))?;
            return Ok(());
        }
        let dir = if abs.is_dir() { abs } else {
            return Err(AppError::ScanError(format!("输出文件尚未生成: {}", path)));
        };
        std::process::Command::new("explorer")
            .arg(dir.to_string_lossy().to_string())
            .spawn()
            .map_err(|e| AppError::BundleError(format!("打开文件夹失败: {}", e)))?;
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let dir = if p.is_dir() {
            p
        } else if p.is_file() {
            p.parent().map(|d| d.to_path_buf()).unwrap_or(p)
        } else {
            return Err(AppError::ScanError(format!("输出文件尚未生成: {}", path)));
        };
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(cmd)
            .arg(&dir)
            .spawn()
            .map_err(|e| AppError::BundleError(format!("打开文件夹失败: {}", e)))?;
        Ok(())
    }
}

// ==================== OS 拖入导入 ====================
// WebView2 已移除 DOM 侧 File.path，拖入的文件无法直接取绝对路径；
// 前端通过 webkitGetAsEntry 读出文件内容（base64）后调用本命令落盘，
// 再走常规目录扫描流程。

#[derive(Debug, serde::Deserialize)]
pub struct DroppedFile {
    #[serde(rename = "relPath")]
    pub rel_path: String,
    #[serde(rename = "dataBase64")]
    pub data_base64: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ImportResult {
    pub dir: String,
    pub saved: Vec<String>,
    pub skipped: usize,
}

#[tauri::command]
pub fn import_dropped_files<R: tauri::Runtime>(app: AppHandle<R>, files: Vec<DroppedFile>) -> Result<ImportResult, AppError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let root = std::env::temp_dir()
        .join("manualbundler_import")
        .join(format!("import_{}", ts));
    emit_log(&app, "INFO", "拖入导入", format!("接收拖入文件 {} 个", files.len()));
    let (saved, skipped) = import_dropped_files_impl(&root, &files)?;
    let has_page = saved.iter().any(|p| {
        p.rsplit('.').next().map(|e| bundler::is_page_ext(e)).unwrap_or(false)
    });
    if !has_page {
        return Err(AppError::ScanError(
            "拖入内容中没有可导入的页面文件（支持 .html/.htm/.mht/.mhtml、常见图片及文档格式）".into(),
        ));
    }
    emit_log(&app, "INFO", "拖入导入", format!("落盘完成: {} 个文件 → {}", saved.len(), root.to_string_lossy()));
    Ok(ImportResult {
        dir: root.to_string_lossy().into_owned(),
        saved,
        skipped,
    })
}

/// 拖入文件落盘核心逻辑：
/// 保存全部安全文件（页面 + 页面引用的图片/CSS/JS 等资源），仅拒绝路径穿越；
/// 返回 (已保存相对路径, 跳过数)。全量落盘以保证页面外链资源内嵌不缺失。
/// base64 解码 + 写盘为 CPU/IO 密集 → 自动多线程并行处理。
fn import_dropped_files_impl(root: &Path, files: &[DroppedFile]) -> Result<(Vec<String>, usize), AppError> {
    use base64::Engine;
    let eng = &base64::engine::general_purpose::STANDARD;
    let results: Vec<Result<Option<String>, AppError>> = files
        .par_iter()
        .map(|f| {
            let rel_norm = f.rel_path.replace('\\', "/");
            // 安全：拒绝 .. / 绝对路径 / 空段穿越
            let segs: Vec<&str> = rel_norm.split('/').collect();
            let safe = !rel_norm.is_empty()
                && !segs.iter().any(|s| s.is_empty() || *s == "." || *s == "..")
                && !rel_norm.starts_with('/');
            if !safe {
                return Ok(None);
            }
            let data = eng.decode(f.data_base64.as_bytes())
                .map_err(|e| AppError::BundleError(format!("拖入文件解码失败 {}: {}", rel_norm, e)))?;
            let dest = root.join(&rel_norm);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &data)?;
            Ok(Some(rel_norm))
        })
        .collect();
    // par_iter().collect() 保持输入顺序，saved 顺序与原串行实现一致
    let mut saved = vec![];
    let mut skipped = 0usize;
    for r in results {
        match r {
            Ok(Some(rel)) => saved.push(rel),
            Ok(None) => skipped += 1,
            Err(e) => return Err(e),
        }
    }
    Ok((saved, skipped))
}

// ==================== 缺失引用资源补齐 ====================
// 场景：OS 拖入单个 HTML 时，其引用的同目录图片/CSS/JS 不在拖入内容中
// （WebView2 DOM drop 拿不到源文件绝对路径）。由用户指定这些文件所在的
// 原始文件夹，按相对路径/文件名匹配复制到期望位置，重新打包即可内嵌。

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MissingResourceInput {
    pub url: String,
    pub attempted: String,
}

#[tauri::command]
pub fn repair_missing_resources<R: tauri::Runtime>(
    app: AppHandle<R>,
    folder: String,
    missing: Vec<MissingResourceInput>,
) -> Result<usize, AppError> {
    emit_log(&app, "INFO", "资源补齐", format!("尝试补齐 {} 个缺失资源 ← {}", missing.len(), folder));
    let repaired = repair_missing_resources_impl(&folder, &missing)?;
    emit_log(&app, "INFO", "资源补齐", format!("补齐完成: {}/{} 个", repaired, missing.len()));
    Ok(repaired)
}

/// 补齐核心逻辑：对每个缺失引用，在所选文件夹中按相对路径 / 文件名匹配后复制到期望路径
pub fn repair_missing_resources_impl(folder: &str, missing: &[MissingResourceInput]) -> Result<usize, AppError> {
    let root = PathBuf::from(folder);
    if !root.is_dir() {
        return Err(AppError::ScanError(format!("路径不存在或不是目录: {}", folder)));
    }
    let mut still = missing.to_vec();
    Ok(repair_from_root(&root, &mut still))
}

/// 判断是否跳过该目录（系统/隐藏/生成物目录）
fn is_skip_dir_name(name: &str) -> bool {
    name.starts_with('.')
        || name.eq_ignore_ascii_case("$Recycle.Bin")
        || name.eq_ignore_ascii_case("System Volume Information")
        || name.eq_ignore_ascii_case("Windows")
        || name.eq_ignore_ascii_case("AppData")
        || name.eq_ignore_ascii_case("Application Data")
        || name.eq_ignore_ascii_case("node_modules")
}

/// 递归索引目录内文件（文件名 → 路径列表），带深度/数量限制，避免扫爆大目录
fn walk_index_limited(root: &Path, max_depth: usize, index: &mut std::collections::HashMap<String, Vec<PathBuf>>) {
    const MAX_DIRS: usize = 20000;
    const MAX_FILES: usize = 200_000;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        if visited >= MAX_DIRS || index.len() >= MAX_FILES { return; }
        visited += 1;
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for ent in entries.flatten() {
            let path = ent.path();
            let name = ent.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if is_skip_dir_name(&name) { continue; }
                if depth + 1 < max_depth { stack.push((path, depth + 1)); }
            } else if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                index.entry(fname.to_string()).or_default().push(path);
            }
        }
    }
}

/// 在单个根目录内尝试补齐 still 列表（原地移除已解决项），返回实际复制数
fn repair_from_root(root: &Path, still: &mut Vec<MissingResourceInput>) -> usize {
    let mut index: std::collections::HashMap<String, Vec<PathBuf>> = std::collections::HashMap::new();
    walk_index_limited(root, 8, &mut index);
    let mut copied = 0usize;
    still.retain(|m| {
        let dest = PathBuf::from(&m.attempted);
        if dest.exists() { return false; } // 已存在（此前已补齐）→ 视为已解决

        let clean = m.url.split('?').next().unwrap_or(&m.url)
            .split('#').next().unwrap_or(&m.url).to_string();
        let mut found: Option<PathBuf> = None;
        // 候选1：相对 URL 直接拼接到根目录
        let is_remote = clean.starts_with("http://") || clean.starts_with("https://")
            || clean.starts_with("file://") || clean.starts_with("data:");
        if !is_remote && !clean.is_empty() {
            let cand = root.join(clean.replace('\\', "/"));
            if cand.is_file() { found = Some(cand); }
        }
        // 候选2：按文件名在根目录内递归搜索
        if found.is_none() {
            let base = Path::new(&clean).file_name().and_then(|s| s.to_str()).map(|s| s.to_string())
                .or_else(|| Path::new(&m.attempted).file_name().and_then(|s| s.to_str()).map(|s| s.to_string()));
            if let Some(name) = base {
                if let Some(list) = index.get(&name) {
                    if let Some(p) = list.first() { found = Some(p.clone()); }
                }
            }
        }
        if let Some(src) = found {
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&src, &dest).is_ok() {
                copied += 1;
                return false; // 已解决
            }
        }
        true // 仍未解决
    });
    copied
}

// ==================== 缺失引用资源自动补齐（无弹窗） ====================
// 检测到缺失资源时按优先级自动搜索多个候选根目录：
//   ① 导入目录 / 输入目录 / 历史补齐目录（前端传入，页面"同级"及记忆位置）
//   ② 系统常用目录：桌面 / 文档 / 下载 / 图片（首次导入也能无感命中）
// 全部未命中时前端再降级为手动选择文件夹弹窗。

#[derive(Debug, serde::Serialize)]
pub struct AutoRepairResult {
    pub repaired: usize,
    pub still_missing: Vec<MissingResourceInput>,
    /// 命中资源的来源根目录（供前端记忆，下次优先搜索）
    pub repaired_from: Vec<String>,
}

#[tauri::command]
pub fn auto_repair_missing_resources<R: tauri::Runtime>(
    app: AppHandle<R>,
    missing: Vec<MissingResourceInput>,
    extra_dirs: Vec<String>,
) -> Result<AutoRepairResult, AppError> {
    let mut roots: Vec<PathBuf> = vec![];
    for d in &extra_dirs {
        let p = PathBuf::from(d);
        if p.is_dir() && !roots.contains(&p) { roots.push(p); }
    }
    for d in [dirs::desktop_dir(), dirs::document_dir(), dirs::download_dir(), dirs::picture_dir()]
        .into_iter().flatten()
    {
        if !roots.contains(&d) { roots.push(d); }
    }
    emit_log(&app, "INFO", "资源补齐", format!("自动补齐: {} 个缺失，候选根目录 {} 个", missing.len(), roots.len()));

    let mut still = missing.clone();
    let mut repaired_total = 0usize;
    let mut repaired_from: Vec<String> = vec![];
    for root in &roots {
        if still.is_empty() { break; }
        let n = repair_from_root(root, &mut still);
        if n > 0 {
            repaired_total += n;
            repaired_from.push(root.to_string_lossy().into_owned());
            emit_log(&app, "INFO", "资源补齐", format!("从 {} 自动复制 {} 个资源", root.display(), n));
        }
    }
    emit_log(&app, "INFO", "资源补齐", format!("自动补齐完成: {}/{} 个", repaired_total, missing.len()));
    Ok(AutoRepairResult { repaired: repaired_total, still_missing: still, repaired_from })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn df(rel: &str, data: &[u8]) -> DroppedFile {
        DroppedFile {
            rel_path: rel.into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        }
    }

    #[test]
    fn test_import_dropped_impl() {
        let root = std::env::temp_dir().join(format!("mb_import_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let files = vec![
            df("page1.html", b"<html><title>P1</title></html>"),
            df("sub/dir/shot.png", b"PNGDATA"),
            df("notes.txt", b"plain text page"), // txt 也是页面类型 → 保存
            df("assets/style.css", b"body{}"),   // 页面引用的资源 → 全量落盘保存
            df("assets/app.js", b"var a=1;"),    // 同上
            df("../evil.html", b"traversal"),    // 路径穿越 → 跳过
        ];
        let (saved, skipped) = import_dropped_files_impl(&root, &files).unwrap();
        assert_eq!(
            saved,
            vec![
                "page1.html".to_string(),
                "sub/dir/shot.png".to_string(),
                "notes.txt".to_string(),
                "assets/style.css".to_string(),
                "assets/app.js".to_string(),
            ]
        );
        assert_eq!(skipped, 1);
        let html = std::fs::read_to_string(root.join("page1.html")).unwrap();
        assert_eq!(html, "<html><title>P1</title></html>");
        let png = std::fs::read(root.join("sub/dir/shot.png")).unwrap();
        assert_eq!(png, b"PNGDATA");
        let txt = std::fs::read_to_string(root.join("notes.txt")).unwrap();
        assert_eq!(txt, "plain text page");
        let css = std::fs::read_to_string(root.join("assets/style.css")).unwrap();
        assert_eq!(css, "body{}");
        assert!(!root.parent().unwrap().join("evil.html").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_import_bad_base64() {
        let root = std::env::temp_dir().join(format!("mb_import_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let files = vec![DroppedFile {
            rel_path: "bad.html".into(),
            data_base64: "!!!not-base64!!!".into(),
        }];
        assert!(import_dropped_files_impl(&root, &files).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_repair_missing_resources_impl() {
        let src = std::env::temp_dir().join(format!("mb_repair_src_{}", std::process::id()));
        let dst = std::env::temp_dir().join(format!("mb_repair_dst_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(&src).unwrap();

        // 源文件夹：相对路径文件 + 仅文件名可匹配的子目录文件
        std::fs::write(src.join("截图.png"), b"PNG").unwrap();
        std::fs::create_dir_all(src.join("deep/sub")).unwrap();
        std::fs::write(src.join("deep/sub/logo.png"), b"LOGO").unwrap();

        let missing = vec![
            MissingResourceInput { url: "截图.png".into(), attempted: dst.join("截图.png").to_string_lossy().to_string() },
            // 期望路径在 dst/img/logo.png，但源里只有 deep/sub/logo.png → 按文件名匹配
            MissingResourceInput { url: "img/logo.png".into(), attempted: dst.join("img/logo.png").to_string_lossy().to_string() },
            // 远程 URL → 跳过
            MissingResourceInput { url: "https://a.com/x.png".into(), attempted: dst.join("remote.png").to_string_lossy().to_string() },
        ];
        let repaired = repair_missing_resources_impl(src.to_string_lossy().as_ref(), &missing).unwrap();
        assert_eq!(repaired, 2);
        assert_eq!(std::fs::read(dst.join("截图.png")).unwrap(), b"PNG");
        assert_eq!(std::fs::read(dst.join("img/logo.png")).unwrap(), b"LOGO");
        assert!(!dst.join("remote.png").exists());

        // 幂等：再次补齐 → 全部已存在，repaired = 0
        let again = repair_missing_resources_impl(src.to_string_lossy().as_ref(), &missing).unwrap();
        assert_eq!(again, 0);

        // 目录不存在 → 报错
        assert!(repair_missing_resources_impl(src.join("no_such_dir").to_string_lossy().as_ref(), &missing).is_err());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }

    #[test]
    fn test_repair_from_root_auto_search() {
        let src = std::env::temp_dir().join(format!("mb_auto_src_{}", std::process::id()));
        let dst = std::env::temp_dir().join(format!("mb_auto_dst_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
        // 源：下一级子目录（含应跳过的 .git）+ 页面同级目录
        std::fs::create_dir_all(src.join(".git")).unwrap();
        std::fs::write(src.join(".git/hidden.png"), b"HIDDEN").unwrap();
        std::fs::create_dir_all(src.join("deep/sub")).unwrap();
        std::fs::write(src.join("deep/sub/logo.png"), b"LOGO").unwrap();
        std::fs::write(src.join("top.png"), b"TOP").unwrap();

        let mut still = vec![
            // .git 内的同名文件必须被跳过，deep/sub 里的命中
            MissingResourceInput { url: "logo.png".into(), attempted: dst.join("a/logo.png").to_string_lossy().to_string() },
            MissingResourceInput { url: "top.png".into(), attempted: dst.join("top.png").to_string_lossy().to_string() },
            MissingResourceInput { url: "nope.png".into(), attempted: dst.join("nope.png").to_string_lossy().to_string() },
        ];
        let copied = repair_from_root(&src, &mut still);
        assert_eq!(copied, 2);
        assert_eq!(still.len(), 1);
        assert_eq!(still[0].url, "nope.png");
        assert_eq!(std::fs::read(dst.join("a/logo.png")).unwrap(), b"LOGO");
        // .git 内的文件未被误用
        assert!(!std::fs::read(dst.join("a/logo.png")).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }
}
