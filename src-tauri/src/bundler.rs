use crate::error::AppError;
use crate::model::*;
use rayon::prelude::*;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::collections::HashMap;

/// 路径稳定哈希 → pg_ 前缀 id
pub fn page_id_for(rel_path: &str) -> String {
    let hash = md5::compute(rel_path.as_bytes());
    let b: Vec<u8> = hash.iter().take(4).cloned().collect();
    let hex = b.iter().map(|x| format!("{:02x}", x)).collect::<String>();
    format!("pg_{}", hex)
}

// ==================== 输入文件类型判定 ====================

pub fn is_html_ext(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
}

pub fn is_mhtml_ext(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("mht") || ext.eq_ignore_ascii_case("mhtml")
}

pub fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "avif"
    )
}

pub fn is_doc_ext(ext: &str) -> bool {
    crate::docs::is_doc_ext(ext)
}

/// 可作为手册页面的文件类型（HTML / MHTML / 常见图片 / 常见文档）
pub fn is_page_ext(ext: &str) -> bool {
    is_html_ext(ext) || is_mhtml_ext(ext) || is_image_ext(ext) || is_doc_ext(ext)
}

/// 读取页面文件并统一转为可打包的 HTML 文本：
/// - html/htm  → 原文（资源内嵌由 embed_resources_simple 处理）
/// - mht/mhtml → 解析 MIME 结构，资源转 data URI
/// - 图片      → 包装为居中展示该图片的极简页面
/// - 文档      → 文本提取转 HTML（docs 模块）
pub fn load_page_content(path: &Path, rel: &str) -> Result<String, AppError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let bytes = std::fs::read(path)
        .map_err(|e| AppError::BundleError(format!("读取页面失败 {}: {}", rel, e)))?;
    if is_mhtml_ext(&ext) {
        crate::mhtml::convert_mhtml_to_html(&bytes)
    } else if is_image_ext(&ext) {
        Ok(image_page_html(&bytes, &ext, rel))
    } else if crate::docs::is_doc_ext(&ext) {
        Ok(crate::docs::convert_doc_to_html(&bytes, &ext, rel))
    } else {
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// 图片 → 单页 HTML（深色背景居中展示）
fn image_page_html(bytes: &[u8], ext: &str, rel: &str) -> String {
    let mime = guess_mime_by_ext(ext);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
    let name = Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("图片")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head><meta charset="utf-8"><title>{name}</title>
<style>
html,body{{margin:0;height:100%;background:#14161c;display:flex;align-items:center;justify-content:center;}}
img{{max-width:94%;max-height:94%;object-fit:contain;}}
</style></head>
<body><img alt="{name}" src="data:{mime};base64,{b64}"></body>
</html>"#
    )
}

pub fn build_bundle(config: &BundlerConfig) -> Result<PreviewResult, AppError> {
    // 清空上一轮的缺失资源记录（防止早退路径遗留脏数据）
    let _ = take_missing_resources();
    // 1. 收集输入目录页面（支持目录或文件）
    let mut all_pages: Vec<(PathBuf, String)> = vec![];
    for input in &config.input_dirs {
        let p = PathBuf::from(input);
        if !p.exists() {
            return Err(AppError::ScanError(format!("输入路径不存在: {}", input)));
        }
        if p.is_file() {
            // 单文件：直接加入，相对路径用文件名（或从父目录算）
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string();
            let ext_ok = p
                .extension()
                .and_then(|e| e.to_str())
                .map(is_page_ext)
                .unwrap_or(false);
            if ext_ok {
                let parent = p.parent().unwrap_or_else(|| Path::new("."));
                let rel = p.strip_prefix(parent).ok()
                    .and_then(|r| r.to_str().map(|s| s.replace('\\', "/")))
                    .unwrap_or_else(|| name.clone());
                all_pages.push((p.clone(), rel));
            }
        } else {
            collect_html_pages(&p, &p, &mut all_pages);
        }
    }
    if all_pages.is_empty() {
        return Err(AppError::ScanError(
            "未找到任何可打包的页面（支持 .html/.htm/.mht/.mhtml、常见图片及文档格式）".into(),
        ));
    }

    // 2. 只打包导航引用到的页面（导航为空时回退：打包全部扫描页面）
    let referenced = collect_nav_files(&config.nav);
    if !referenced.is_empty() {
        all_pages.retain(|(_, rel)| referenced.contains(rel));
        if all_pages.is_empty() {
            return Err(AppError::ScanError("导航引用的页面均未在输入目录中找到".into()));
        }
    }

    // 3. 并行处理每页
    let page_results: Vec<_> = all_pages.par_iter().map(|(abs_path, rel)| {
        let content = load_page_content(abs_path, rel)?;
        let page_id = page_id_for(rel);
        let (embedded_html, res_count) = embed_resources_simple(&content, abs_path, config)?;
        let final_html = inject_shim(&embedded_html, config);
        Ok::<_, AppError>((page_id, rel.clone(), final_html, res_count))
    }).collect::<Result<Vec<_>, _>>()?;

    let mut pages_map: HashMap<String, String> = HashMap::new();
    let mut filemap: HashMap<String, String> = HashMap::new();
    let mut total_res = 0usize;
    for (pid, rel, html, rc) in page_results {
        filemap.insert(rel.clone(), pid.clone());
        pages_map.insert(pid, html);
        total_res += rc;
    }

    // 4. 构建导航 JSON
    let nav_json = build_nav_json(&config.nav, &filemap);
    let shell_style_char = match config.shell_style {
        ShellStyle::A => 'A', ShellStyle::B => 'B', ShellStyle::C => 'C', ShellStyle::D => 'D',
    };
    let breadcrumb = config.breadcrumb_prefix.trim_end_matches('/').trim();
    let title = &config.title;
    // 初始页面 = 导航中第一个有效页面（避免 HashMap 随机取到无关页）
    let init_file = first_nav_file(&config.nav, &filemap)
        .unwrap_or_else(|| filemap.keys().next().cloned().unwrap_or_default());

    // 5. 组装外壳（辅助文字语言：显式选择 > 界面语言 > 简体中文）
    let aux = aux_texts(&resolve_aux_lang(config));
    let shell_html = build_shell_html(title, breadcrumb, shell_style_char, &nav_json, &pages_map, &filemap, &init_file, config.options.collapsible_sidebar, &aux);

    // 6. 输出外壳（保留原始换行，避免内联脚本被压平）
    let final_html = shell_html;

    let stats = Stats {
        page_count: pages_map.len(),
        embedded_resource_count: total_res,
        final_bytes: final_html.len(),
    };

    Ok(PreviewResult {
        html: final_html,
        stats,
        missing_resources: take_missing_resources(),
    })
}

fn collect_html_pages(root: &Path, current: &Path, out: &mut Vec<(PathBuf, String)>) {
    if let Ok(entries) = std::fs::read_dir(current) {
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_dir() {
                collect_html_pages(root, &p, out);
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if is_page_ext(ext) {
                    let rel = p.strip_prefix(root).ok()
                        .and_then(|r| r.to_str().map(|s| s.replace('\\', "/")))
                        .unwrap_or_else(|| p.to_string_lossy().to_string());
                    out.push((p, rel));
                }
            }
        }
    }
}

/// 简化版资源内嵌：正则替换
fn embed_resources_simple(html: &str, page_path: &Path, config: &BundlerConfig) -> Result<(String, usize), AppError> {
    let base_dir = page_path.parent().unwrap_or_else(|| Path::new("."));
    let mut result = html.to_string();
    let mut count = 0usize;

    // ===== 1. 匹配 <img src="..."> 双引号版本 =====
    let re_img_dq = regex_static(r#"(<img\b[^>]*?\bsrc\s*=\s*")([^"]+)"#);
    result = re_img_dq.replace_all(&result, |caps: &regex::Captures| {
        let prefix = &caps[1]; // 含前缀 + 引号
        let url = &caps[2];
        if should_embed(url) {
            match load_as_data_uri(url, base_dir) {
                Ok(data_uri) => { count += 1; format!("{}{}\"", prefix, data_uri) }
                Err(e) => { LogWarn::new(format!("嵌入图片失败 {}: {}", url, e)); format!("{}{}\"", prefix, url) }
            }
        } else {
            format!("{}{}\"", prefix, url)
        }
    }).to_string();

    // ===== 2. 匹配 <img src='...'> 单引号版本 =====
    let re_img_sq = regex_static(r#"(<img\b[^>]*?\bsrc\s*=\s*')([^']+)'"#);
    result = re_img_sq.replace_all(&result, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let url = &caps[2];
        if should_embed(url) {
            match load_as_data_uri(url, base_dir) {
                Ok(data_uri) => { count += 1; format!("{}{}'", prefix, data_uri) }
                Err(e) => { LogWarn::new(format!("嵌入图片失败 {}: {}", url, e)); format!("{}{}'", prefix, url) }
            }
        } else {
            format!("{}{}'", prefix, url)
        }
    }).to_string();

    // ===== 3. 匹配 <link rel="stylesheet" href="..."> 双引号版本 =====
    // 先用更简单直接的 approach：匹配 link stylesheet 提取所有属性，重写
    let re_link_any = regex_static(r#"<link\b([^>]*?)rel\s*=\s*"stylesheet"([^>]*)>"#);
    result = re_link_any.replace_all(&result, |caps: &regex::Captures| {
        let before = &caps[1]; // rel="stylesheet" 之前的部分
        let after = &caps[2];  // rel="stylesheet" 之后的部分（含可能的 href）
        // 尝试从 after 中找 href="..." 或 href='...'
        let (href_val, _rest_after) = extract_href(after);
        if let Some(href) = href_val {
            if should_embed(&href) {
                match load_local_text(&href, base_dir) {
                    Ok(content) => { count += 1; format!("<style>{}</style>", content) }
                    Err(e) => {
                        LogWarn::new(format!("内嵌样式表失败 {}: {}", href, e));
                        format!("<link{}rel=\"stylesheet\"{}>", before, after)
                    }
                }
            } else {
                format!("<link{}rel=\"stylesheet\"{}>", before, after)
            }
        } else {
            // href 在 before 里？也尝试找
            let (href_val2, rest_before) = extract_href(before);
            if let Some(href) = href_val2 {
                if should_embed(&href) {
                    match load_local_text(&href, base_dir) {
                        Ok(content) => { count += 1; format!("<style>{}</style>", content) }
                        Err(e) => {
                            LogWarn::new(format!("内嵌样式表失败 {}: {}", href, e));
                            format!("<link{}rel=\"stylesheet\"{}>", rest_before, after)
                        }
                    }
                } else {
                    format!("<link{}rel=\"stylesheet\"{}>", rest_before, after)
                }
            } else {
                // 保持原样
                format!("<link{}rel=\"stylesheet\"{}>", before, after)
            }
        }
    }).to_string();

    // ===== 4. 匹配 <script src="...">...</script> 双引号版本 =====
    let re_script_tag = regex_static(r#"<script\b([^>]*)src\s*=\s*"([^"]+)"([^>]*)>([^<]*)</script>"#);
    result = re_script_tag.replace_all(&result, |caps: &regex::Captures| {
        let before = &caps[1]; // src 之前的属性
        let href = &caps[2];
        let after = &caps[3];  // src 之后的属性
        if should_embed(href) {
            match load_local_text(href, base_dir) {
                Ok(content) => { count += 1; format!("<script>{}</script>", content) }
                Err(e) => {
                    LogWarn::new(format!("内嵌脚本失败 {}: {}", href, e));
                    format!("<script{}src=\"{}\"{}></script>", before, href, after)
                }
            }
        } else {
            format!("<script{}src=\"{}\"{}></script>", before, href, after)
        }
    }).to_string();

    // ===== 5. 匹配 <script src='...'> 单引号版本 =====
    let re_script_sq = regex_static(r#"<script\b([^>]*)src\s*=\s*'([^']+)'([^>]*)>([^<]*)</script>"#);
    result = re_script_sq.replace_all(&result, |caps: &regex::Captures| {
        let before = &caps[1];
        let href = &caps[2];
        let after = &caps[3];
        if should_embed(href) {
            match load_local_text(href, base_dir) {
                Ok(content) => { count += 1; format!("<script>{}</script>", content) }
                Err(e) => {
                    LogWarn::new(format!("内嵌脚本失败 {}: {}", href, e));
                    format!("<script{}src='{}'{}></script>", before, href, after)
                }
            }
        } else {
            format!("<script{}src='{}'{}></script>", before, href, after)
        }
    }).to_string();

    Ok((result, count))
}

/// 从字符串中提取 href="..." 或 href='...' 的值，返回 (Option<href_value>, 剩余部分)
fn extract_href(s: &str) -> (Option<String>, String) {
    // 先找双引号版本
    if let Some(pos) = s.find("href=\"") {
        let start = pos + 6;
        if let Some(end_pos) = s[start..].find('"') {
            let val = s[start..start + end_pos].to_string();
            let before_href = s[..pos].to_string();
            let after_href = s[start + end_pos + 1..].to_string();
            let rest = format!("{}{}", before_href, after_href);
            return (Some(val), rest);
        }
    }
    // 再找单引号版本
    if let Some(pos) = s.find("href='") {
        let start = pos + 6;
        if let Some(end_pos) = s[start..].find('\'') {
            let val = s[start..start + end_pos].to_string();
            let before_href = s[..pos].to_string();
            let after_href = s[start + end_pos + 1..].to_string();
            let rest = format!("{}{}", before_href, after_href);
            return (Some(val), rest);
        }
    }
    (None, s.to_string())
}

fn should_embed(url: &str) -> bool {
    if url.is_empty() { return false; }
    // http(s) 外链一律抓取内嵌（原「保留原外链」选项已移除，固定为默认内嵌行为）
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("//") {
        return true;
    }
    // file:/// 绝对路径引用（浏览器另存/生成器输出常见）→ 解析为本地路径后内嵌
    if url.starts_with("file://") { return true; }
    if url.starts_with("data:") { return false; }
    if url.contains(':') && !url.starts_with('/') { return false; }
    true
}

// ==================== 资源读取（本地 / file:/// / 远程 http(s)） ====================

/// 远程资源大小上限（64MB），防止异常超大文件拖垮内存
const REMOTE_MAX_BYTES: usize = 64 * 1024 * 1024;

static HTTP_AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();

fn http_agent() -> &'static ureq::Agent {
    HTTP_AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("ManualBundler/", env!("CARGO_PKG_VERSION")))
            .build()
    })
}

/// 抓取远程 http(s) 资源，返回 (字节流, Content-Type)
fn fetch_remote(url: &str) -> Result<(Vec<u8>, Option<String>), AppError> {
    let resp = http_agent()
        .get(url)
        .call()
        .map_err(|e| AppError::BundleError(format!("远程资源获取失败 {}: {}", url, e)))?;
    let ct_raw = resp.content_type(); // ureq 2: 返回 &str，缺失时为空串
    let ct = if ct_raw.is_empty() { None } else { Some(ct_raw.to_string()) };
    let mut reader = resp.into_reader().take((REMOTE_MAX_BYTES + 1) as u64);
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .map_err(|e| AppError::BundleError(format!("远程资源读取失败 {}: {}", url, e)))?;
    if buf.len() > REMOTE_MAX_BYTES {
        return Err(AppError::BundleError(format!("远程资源超过 64MB 上限: {}", url)));
    }
    Ok((buf, ct))
}

/// file:// → 本地路径（Windows 形如 file:///<盘符>:/x.png；POSIX 形如 file:///home/x）
fn file_url_path(rest: &str) -> PathBuf {
    if let Some(r) = rest.strip_prefix('/') {
        // "/<盘符>:/..." → Windows 盘符路径 "<盘符>:/..."；POSIX 绝对路径保持原样
        if r.len() >= 2 && r.as_bytes()[1] == b':' {
            PathBuf::from(r)
        } else {
            PathBuf::from(rest)
        }
    } else {
        PathBuf::from(rest)
    }
}

// 缺失的本地资源引用收集：build 期间写入，随 PreviewResult 返回给前端（供"资源补齐"流程）
static MISSING_RESOURCES: std::sync::OnceLock<std::sync::Mutex<Vec<crate::model::MissingResource>>> =
    std::sync::OnceLock::new();

fn push_missing_resource(url: &str, attempted: &Path) {
    let store = MISSING_RESOURCES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    if let Ok(mut v) = store.lock() {
        let key = attempted.to_string_lossy().to_string();
        if !v.iter().any(|m| m.attempted == key) {
            v.push(crate::model::MissingResource {
                url: url.to_string(),
                attempted: key,
            });
        }
    }
}

/// 取走本次打包发现的全部缺失本地资源（取后清空）
pub fn take_missing_resources() -> Vec<crate::model::MissingResource> {
    let store = MISSING_RESOURCES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut v = store.lock().unwrap();
    std::mem::take(&mut *v)
}

/// 统一资源读取核心：
/// - file:/// → 本地绝对路径
/// - http(s):// 或 // 协议相对 → 远程抓取（ureq + rustls）
/// - 其余 → 相对页面目录 / 本地绝对路径
/// 返回 (字节流, 远程响应 Content-Type)
fn load_resource_bytes(url: &str, base: &Path) -> Result<(Vec<u8>, Option<String>), AppError> {
    let clean = url.split('?').next().unwrap_or(url).split('#').next().unwrap_or(url);
    if let Some(rest) = clean.strip_prefix("file://") {
        let p = file_url_path(rest);
        let bytes = std::fs::read(&p).map_err(|e| {
            push_missing_resource(url, &p);
            AppError::BundleError(format!("资源不存在 src=\"{}\" → {}: {}", url, p.display(), e))
        })?;
        return Ok((bytes, None));
    }
    if clean.starts_with("http://") || clean.starts_with("https://") || clean.starts_with("//") {
        let full = if clean.starts_with("//") { format!("https:{}", clean) } else { clean.to_string() };
        return fetch_remote(&full);
    }
    let p = if Path::new(clean).is_absolute() { PathBuf::from(clean) } else { base.join(clean) };
    let bytes = std::fs::read(&p).map_err(|e| {
        push_missing_resource(url, &p);
        AppError::BundleError(format!("资源不存在 src=\"{}\" → {}: {}", url, p.display(), e))
    })?;
    Ok((bytes, None))
}

/// Content-Type 优先，其次按扩展名猜测
fn normalize_mime(content_type: Option<&str>, url: &str) -> String {
    if let Some(ct) = content_type {
        let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if !ct.is_empty() && ct != "application/octet-stream" {
            return ct;
        }
    }
    let clean = url.split('?').next().unwrap_or(url).split('#').next().unwrap_or(url);
    let ext = Path::new(clean).extension().and_then(|e| e.to_str()).unwrap_or("");
    guess_mime_by_ext(ext).to_string()
}

fn load_as_data_uri(url: &str, base: &Path) -> Result<String, AppError> {
    let (bytes, ct) = load_resource_bytes(url, base)?;
    let mime = normalize_mime(ct.as_deref(), url);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

fn load_local_text(url: &str, base: &Path) -> Result<String, AppError> {
    let (bytes, _) = load_resource_bytes(url, base)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn guess_mime_by_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" | "cur" => "image/x-icon",
        "avif" => "image/avif",
        "css" => "text/css",
        "js" => "application/javascript",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn inject_shim(html: &str, _config: &BundlerConfig) -> String {
    let shim = r#"
<script>
(function(){
  function interceptClick(e){
    var a = e.target.closest && e.target.closest('a');
    if(!a) return;
    var href = a.getAttribute('href') || '';
    var target = a.getAttribute('target') || '';
    if(target === '_blank' || target === '_top' || href.startsWith('#') || href.startsWith('http') || href.startsWith('mailto')) return;
    if(/\.html?$/i.test(href)){
      e.preventDefault();
      e.stopPropagation();
      if(window.parent && window.parent.loadPageFromIframe){
        window.parent.loadPageFromIframe(href);
      }
    }
  }
  document.addEventListener('click', interceptClick, true);
})();
</script>"#;
    if let Some(idx) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + shim.len());
        result.push_str(&html[..idx]);
        result.push_str(shim);
        result.push_str(&html[idx..]);
        result
    } else {
        format!("{}</body></html>", html)
    }
}

fn build_nav_json(nav: &[NavNode], filemap: &HashMap<String, String>) -> String {
    let groups: Vec<_> = nav.iter()
        .filter(|n| subtree_has_page(n))
        .map(|n| nav_to_group(n, filemap, ""))
        .collect();
    let json = serde_json::to_string(&groups).unwrap_or_else(|_| "[]".into());
    // 与 json_string_literal 同理：防止节点名中含 </script> 提前终止外壳脚本块
    json.replace("</", "<\\/")
}

/// 收集导航中引用的所有页面相对路径（统一为 / 分隔）
fn collect_nav_files(nav: &[NavNode]) -> std::collections::HashSet<String> {
    fn walk(node: &NavNode, out: &mut std::collections::HashSet<String>) {
        if let NodeKind::Page = node.kind {
            if let Some(f) = &node.file {
                out.insert(f.replace('\\', "/"));
            }
        }
        for c in &node.children {
            walk(c, out);
        }
    }
    let mut out = std::collections::HashSet::new();
    nav.iter().for_each(|n| walk(n, &mut out));
    out
}

/// 导航中第一个可解析到 filemap 的页面文件（深度优先）
fn first_nav_file(nav: &[NavNode], filemap: &HashMap<String, String>) -> Option<String> {
    fn walk(node: &NavNode, filemap: &HashMap<String, String>) -> Option<String> {
        if let NodeKind::Page = node.kind {
            if let Some(f) = &node.file {
                let norm = f.replace('\\', "/");
                if filemap.contains_key(&norm) {
                    return Some(norm);
                }
            }
        }
        for c in &node.children {
            if let Some(f) = walk(c, filemap) {
                return Some(f);
            }
        }
        None
    }
    nav.iter().find_map(|n| walk(n, filemap))
}

fn subtree_has_page(node: &NavNode) -> bool {
    if let NodeKind::Page = node.kind {
        if node.file.is_some() {
            return true;
        }
    }
    node.children.iter().any(subtree_has_page)
}

/// 组节点：直接子页面 → items，子分组 → groups（递归保留层级）
fn nav_to_group(node: &NavNode, filemap: &HashMap<String, String>, prefix: &str) -> serde_json::Value {
    let self_prefix = if prefix.is_empty() {
        node.name.clone()
    } else {
        format!("{} / {}", prefix, node.name)
    };

    let mut items = vec![];
    if let NodeKind::Page = &node.kind {
        if let Some(f) = &node.file {
            let norm = f.replace('\\', "/");
            items.push(page_item(node, &norm, filemap, prefix));
        }
    }
    let mut groups = vec![];
    for c in &node.children {
        match &c.kind {
            NodeKind::Page => {
                if let Some(f) = &c.file {
                    let norm = f.replace('\\', "/");
                    items.push(page_item(c, &norm, filemap, &self_prefix));
                }
            }
            _ => {
                if subtree_has_page(c) {
                    groups.push(nav_to_group(c, filemap, &self_prefix));
                }
            }
        }
    }

    serde_json::json!({
        "id": short_hash(&node.name),
        "title": node.name,
        "type": "main",
        "items": items,
        "groups": groups,
    })
}

fn page_item(node: &NavNode, norm_file: &str, filemap: &HashMap<String, String>, prefix: &str) -> serde_json::Value {
    let crumb = if prefix.is_empty() {
        node.name.clone()
    } else {
        format!("{} / {}", prefix, node.name)
    };
    serde_json::json!({
        "id": short_hash(&node.name),
        "name": node.name,
        "file": norm_file,
        "pageId": filemap.get(norm_file).cloned().unwrap_or_default(),
        "crumb": crumb,
    })
}

fn short_hash(s: &str) -> String {
    let h = md5::compute(s.as_bytes());
    let b: Vec<u8> = h.iter().take(2).cloned().collect();
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

// ==================== 输出手册辅助文字多语言（0.8.5） ====================

/// 输出手册外壳的辅助文字：左侧展开竖条（rail）、收起/展开导航提示、
/// 侧栏副标题与内容加载占位。语言由中栏「辅助文字语言」选项决定。
struct AuxTexts {
    lang: &'static str,
    collapse: &'static str, // 收起导航
    expand: &'static str,   // 展开导航
    manual: &'static str,   // 侧栏「用户手册」副标题
    loading: &'static str,  // 内容加载中占位
}

const AUX_ZH_CN: AuxTexts = AuxTexts {
    lang: "zh-CN", collapse: "收起导航", expand: "展开导航", manual: "用户手册", loading: "加载中...",
};
const AUX_ZH_TW: AuxTexts = AuxTexts {
    lang: "zh-TW", collapse: "收合導航", expand: "展開導航", manual: "使用者手冊", loading: "載入中...",
};
const AUX_ZH_HK: AuxTexts = AuxTexts {
    lang: "zh-HK", collapse: "收合導航", expand: "展開導航", manual: "用戶手冊", loading: "載入中...",
};

fn aux_texts(lang: &str) -> AuxTexts {
    match lang {
        "zh-TW" => AUX_ZH_TW,
        "zh-HK" => AUX_ZH_HK,
        "en" => AuxTexts { lang: "en", collapse: "Collapse navigation", expand: "Expand navigation", manual: "User Manual", loading: "Loading..." },
        "fr" => AuxTexts { lang: "fr", collapse: "Réduire la navigation", expand: "Déplier la navigation", manual: "Manuel de l'utilisateur", loading: "Chargement..." },
        "ru" => AuxTexts { lang: "ru", collapse: "Свернуть навигацию", expand: "Развернуть навигацию", manual: "Руководство пользователя", loading: "Загрузка..." },
        "es" => AuxTexts { lang: "es", collapse: "Contraer navegación", expand: "Desplegar navegación", manual: "Manual de usuario", loading: "Cargando..." },
        "pt" => AuxTexts { lang: "pt", collapse: "Recolher navegação", expand: "Expandir navegação", manual: "Manual do utilizador", loading: "A carregar..." },
        "ar" => AuxTexts { lang: "ar", collapse: "طي التنقل", expand: "توسيع التنقل", manual: "دليل المستخدم", loading: "جارٍ التحميل..." },
        "ja" => AuxTexts { lang: "ja", collapse: "ナビを折りたたむ", expand: "ナビを展開", manual: "ユーザーマニュアル", loading: "読み込み中..." },
        "ko" => AuxTexts { lang: "ko", collapse: "탐색 접기", expand: "탐색 펼치기", manual: "사용자 매뉴얼", loading: "불러오는 중..." },
        "el" => AuxTexts { lang: "el", collapse: "Σύμπτυξη πλοήγησης", expand: "Ανάπτυξη πλοήγησης", manual: "Εγχειρίδιο χρήστη", loading: "Φόρτωση..." },
        "de" => AuxTexts { lang: "de", collapse: "Navigation einklappen", expand: "Navigation ausklappen", manual: "Benutzerhandbuch", loading: "Wird geladen..." },
        _ => AUX_ZH_CN,
    }
}

/// 解析辅助文字语言：显式选择优先，其次跟随界面语言（config.locale），默认简体中文
fn resolve_aux_lang(config: &BundlerConfig) -> String {
    let explicit = config.options.aux_lang.as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "auto");
    explicit
        .or_else(|| config.locale.clone().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "zh-CN".to_string())
}

/// HTML 属性/文本转义（辅助文字均来自内置文案，此处兜底防御）
fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn build_shell_html(
    title: &str,
    breadcrumb: &str,
    scheme: char,
    nav_json: &str,
    pages_map: &HashMap<String, String>,
    filemap: &HashMap<String, String>,
    init_file: &str,
    collapsible: bool,
    aux: &AuxTexts,
) -> String {
    let shell_css = include_str!("../templates/shell.css");

    let pages_json_str = {
        let mut entries: Vec<(&String, &String)> = pages_map.iter().collect();
        entries.sort_by_key(|(k, _)| *k);
        let parts: Vec<String> = entries.iter().map(|(k, v)| {
            format!("  {}: {}", serde_json::Value::String((*k).clone()), json_string_literal(v))
        }).collect();
        format!("const PAGES = {{\n{}\n}};", parts.join(",\n"))
    };

    let filemap_json_str = {
        let mut m = serde_json::Map::new();
        for (k, v) in filemap {
            m.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
        format!("const FILEMAP = {};", serde_json::Value::Object(m))
    };

    let init_file_json = serde_json::Value::String(init_file.to_string());
    let scheme_str = format!("{}", scheme.to_lowercase());
    let scheme_json = serde_json::Value::String(scheme_str);

    let template = include_str!("../templates/shell.html");

    template
        .replace("{{TITLE}}", title)
        .replace("{{SHELL_CSS}}", shell_css)
        .replace("{{BREADCRUMB}}", breadcrumb)
        .replace("{{NAV_JSON}}", nav_json)
        .replace("{{PAGES_JSON}}", &pages_json_str)
        .replace("{{FILEMAP_JSON}}", &filemap_json_str)
        .replace("{{INIT_FILE_JSON}}", &init_file_json.to_string())
        .replace("{{SCHEME_JSON}}", &scheme_json.to_string())
        .replace("{{SIDEBAR_COLLAPSIBLE_JSON}}", if collapsible { "true" } else { "false" })
        // 辅助文字（左侧展开竖条等）按所选语言注入
        .replace("{{AUX_LANG}}", aux.lang)
        .replace("{{AUX_COLLAPSE}}", &html_esc(aux.collapse))
        .replace("{{AUX_EXPAND}}", &html_esc(aux.expand))
        .replace("{{AUX_MANUAL}}", &html_esc(aux.manual))
        .replace("{{AUX_LOADING}}", &html_esc(aux.loading))
}

fn json_string_literal(s: &str) -> String {
    let json = serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    // 关键修复：转义 "</" → "<\/"。
    // serde_json 不转义 "/"，内嵌页面 HTML（含注入 shim 的 </script>）会在
    // 外壳 <script> 块内形成字面 "</script>"，浏览器解析 HTML 时会提前终止
    // 脚本块 → 外壳 JS 全部失效（导航空白、iframe 空白、JSON 以纯文本显示）。
    // "\/" 是 JSON 与 JS 字符串的合法转义，解码后还原为 "/"，页面内容不受影响。
    json.replace("</", "<\\/")
}

// ============= 静态正则缓存 =============
static RE_CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<&'static str, regex::Regex>>> = std::sync::OnceLock::new();
fn regex_static(pattern: &'static str) -> regex::Regex {
    let cache = RE_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap();
    if let Some(re) = map.get(pattern) { return re.clone(); }
    let re = regex::Regex::new(pattern).unwrap();
    map.insert(pattern, re.clone());
    re
}

// 资源内嵌失败告警收集：build 期间写入，commands 层 take_warnings() 后上报前端日志
static WARNINGS: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();

struct LogWarn;
impl LogWarn {
    fn new(msg: String) -> Self {
        let store = WARNINGS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
        if let Ok(mut v) = store.lock() {
            if !v.contains(&msg) { v.push(msg); }
        }
        LogWarn
    }
}

/// 取走本次打包产生的全部告警（取后清空）
pub fn take_warnings() -> Vec<String> {
    let store = WARNINGS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut v = store.lock().unwrap();
    std::mem::take(&mut *v)
}

// ==================== 集成测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BundlerConfig, NavNode, NodeKind};

    /// 1x1 PNG 样本
    const PNG_1PX: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00,
        0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
        0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn page_config(input: String, file: &str) -> BundlerConfig {
        BundlerConfig {
            input_dirs: vec![input],
            nav: vec![NavNode {
                kind: NodeKind::Page,
                name: "页面".into(),
                file: Some(file.into()),
                children: vec![],
            }],
            ..Default::default()
        }
    }

    fn temp_dir() -> PathBuf {
        // 每次调用生成唯一目录：此前全部测试共用 mb_test_{pid}，并行测试的
        // remove_dir_all 收尾会删掉其他测试正在读写的文件（file:// 内嵌测试偶发挂）。
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("mb_test_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 图片作为输入页：完整打包后应产出内嵌 data URI 的图片展示页
    #[test]
    fn test_image_page_bundle() {
        let dir = temp_dir();
        let png_path = dir.join("screenshot.png");
        // 1x1 PNG
        let png_bytes: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
            0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00,
            0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
            0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
            0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&png_path, &png_bytes).unwrap();

        let config = BundlerConfig {
            input_dirs: vec![png_path.to_string_lossy().replace('\\', "/")],
            nav: vec![NavNode {
                kind: NodeKind::Page,
                name: "截图".into(),
                file: Some("screenshot.png".into()),
                children: vec![],
            }],
            ..Default::default()
        };
        let result = build_bundle(&config).unwrap();
        assert_eq!(result.stats.page_count, 1);
        assert!(result.html.contains("data:image/png;base64,"), "image not inlined");
        // 页面 HTML 以 JSON 字符串内嵌（"</" 被转义为 "<\/"），只断言 title 起始片段
        assert!(result.html.contains("<title>screenshot"), "image page title missing");
        assert!(result.html.contains("const PAGES ="), "shell missing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MHTML 作为输入页：完整打包后资源应为 data URI
    #[test]
    fn test_mhtml_page_bundle() {
        let mht = crate::mhtml::tests::SAMPLE;
        let dir = temp_dir();
        let mht_path = dir.join("saved.mht");
        std::fs::write(&mht_path, mht).unwrap();

        let config = BundlerConfig {
            input_dirs: vec![mht_path.to_string_lossy().replace('\\', "/")],
            nav: vec![NavNode {
                kind: NodeKind::Page,
                name: "存档页".into(),
                file: Some("saved.mht".into()),
                children: vec![],
            }],
            ..Default::default()
        };
        let result = build_bundle(&config).unwrap();
        assert_eq!(result.stats.page_count, 1);
        assert!(result.html.contains("data:image/png;base64,"), "mhtml image not inlined");
        assert!(!result.html.contains("https://example.com/img.png\""), "raw url remains");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Markdown 文档作为输入页：完整打包后正文渲染为 HTML
    #[test]
    fn test_md_page_bundle() {
        let dir = temp_dir();
        let md_path = dir.join("readme.md");
        std::fs::write(&md_path, "# 安装\n\n第一步：解压。\n").unwrap();

        let config = BundlerConfig {
            input_dirs: vec![md_path.to_string_lossy().replace('\\', "/")],
            nav: vec![NavNode {
                kind: NodeKind::Page,
                name: "说明".into(),
                file: Some("readme.md".into()),
                children: vec![],
            }],
            ..Default::default()
        };
        let result = build_bundle(&config).unwrap();
        assert_eq!(result.stats.page_count, 1);
        // 页面 HTML 以 JSON 字符串内嵌（"</" 被转义为 "<\/"），只断言 h1 起始片段
        assert!(result.html.contains("<h1>安装"), "md not rendered: {}", result.html);
        assert!(result.html.contains("第一步：解压。"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// HTML 页面引用同目录（中文名）图片 → 应成功内嵌 data URI
    #[test]
    fn test_local_relative_ref_embed() {
        let dir = temp_dir();
        let png_path = dir.join("截图 测试-20261119.png");
        std::fs::write(&png_path, PNG_1PX).unwrap();
        let html_path = dir.join("页面.html");
        std::fs::write(&html_path, "<html><body><img src=\"截图 测试-20261119.png\" alt=\"图\"></body></html>").unwrap();

        let config = page_config(html_path.to_string_lossy().replace('\\', "/"), "页面.html");
        let result = build_bundle(&config).unwrap();
        assert_eq!(result.stats.embedded_resource_count, 1, "relative ref not embedded");
        assert!(result.html.contains("data:image/png;base64,"), "relative ref not inlined");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// HTML 页面以 file:/// 绝对路径引用图片 → 应解析为本地路径并内嵌
    #[test]
    fn test_file_url_ref_embed() {
        let dir = temp_dir();
        let png_path = dir.join("shot_file_url.png");
        std::fs::write(&png_path, PNG_1PX).unwrap();
        let file_url = format!("file:///{}", png_path.to_string_lossy().replace('\\', "/"));
        let html_path = dir.join("page.html");
        std::fs::write(&html_path, format!("<html><body><img src=\"{}\"></body></html>", file_url)).unwrap();

        let config = page_config(html_path.to_string_lossy().replace('\\', "/"), "page.html");
        let result = build_bundle(&config).unwrap();
        assert_eq!(result.stats.embedded_resource_count, 1, "file:// ref not embedded");
        assert!(result.html.contains("data:image/png;base64,"), "file:// ref not inlined");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 扩展名判定
    #[test]
    fn test_ext_detection() {
        assert!(is_page_ext("html"));
        assert!(is_page_ext("HTM"));
        assert!(is_page_ext("mht"));
        assert!(is_page_ext("MHTML"));
        assert!(is_page_ext("png"));
        assert!(is_page_ext("JPG"));
        assert!(is_page_ext("svg"));
        assert!(is_page_ext("bmp"));
        // 文档格式
        assert!(is_page_ext("txt"));
        assert!(is_page_ext("md"));
        assert!(is_page_ext("pdf"));
        assert!(is_page_ext("docx"));
        assert!(is_page_ext("ppt"));
        assert!(!is_page_ext("exe"));
        assert!(!is_page_ext("bin"));
        assert!(is_image_ext("jpeg"));
        assert!(!is_image_ext("html"));
        assert!(is_doc_ext("rtf"));
        assert!(is_doc_ext("wps"));
        assert!(!is_doc_ext("png"));
    }

    /// 0.8.5：辅助文字语言注入（左栏展开竖条等提示随选项转换）
    #[test]
    fn test_aux_lang_shell_injection() {
        let dir = temp_dir();
        let html_path = dir.join("page.html");
        std::fs::write(&html_path, "<html><title>P</title><body>p</body></html>").unwrap();

        let mk = |locale: Option<&str>, aux: Option<&str>| BundlerConfig {
            input_dirs: vec![html_path.to_string_lossy().replace('\\', "/")],
            locale: locale.map(String::from),
            options: crate::model::Options { aux_lang: aux.map(String::from), ..Default::default() },
            ..Default::default()
        };

        // 默认（跟随 zh-CN）：简体辅助文字
        let out = build_bundle(&mk(Some("zh-CN"), None)).unwrap().html;
        assert!(out.contains("展开导航"), "default aux text missing");
        assert!(out.contains("<html lang=\"zh-CN\">"));

        // 界面语言 en：辅助文字自动跟随
        let out = build_bundle(&mk(Some("en"), None)).unwrap().html;
        assert!(out.contains("Expand navigation"), "follow-locale aux missing");
        assert!(out.contains("<html lang=\"en\">"));

        // 显式选择 ja 覆盖界面语言
        let out = build_bundle(&mk(Some("en"), Some("ja"))).unwrap().html;
        assert!(out.contains("ナビを展開"), "explicit aux missing");
        assert!(out.contains("<html lang=\"ja\">"));

        // 显式 auto = 跟随界面语言
        let out = build_bundle(&mk(Some("ru"), Some("auto"))).unwrap().html;
        assert!(out.contains("Развернуть навигацию"), "auto aux missing");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
