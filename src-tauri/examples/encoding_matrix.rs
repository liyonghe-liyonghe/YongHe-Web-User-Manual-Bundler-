//! 语言-编码矩阵测试：由 MB_MATRIX_INPUT_DIR 指定的目录下 12 个 `<语言>-<编码>` 文件夹
//! × 11 种文档格式（pdf/doc/docx/rtf/wps/odt/odp/ppt/pptx/md/txt）。
//! ① 逐文件转换断言：语言标记必须出现在输出中（编码解析正确）、PDF 必须走 pdfium 渲染、
//!    演示格式必须分页、除空文件外不得出现降级占位。
//! ② 全量合并打包（build_bundle）断言产物。
//! 任一断言失败退出码非 0。

use manualbundler::bundler::{self, is_page_ext, load_page_content};
use manualbundler::model::*;
use std::path::{Path, PathBuf};

fn input_dir() -> String {
    std::env::var("MB_MATRIX_INPUT_DIR").unwrap_or_else(|_| "_test_out/encoding_pages".to_string())
}
fn out_dir() -> String {
    std::env::var("MB_MATRIX_OUTPUT_DIR").unwrap_or_else(|_| "_test_out".to_string())
}

/// 语言代码 → 每个文件都必须出现的标记
fn lang_marker(folder: &str) -> Option<&'static str> {
    if folder.starts_with("zh-CN-") {
        Some("简体中文样式矩阵")
    } else if folder.starts_with("zh-TW-") {
        Some("繁體中文樣式矩陣")
    } else if folder.starts_with("ja-JP-") {
        Some("日本語スタイルマトリクス")
    } else if folder.starts_with("ko-KR-") {
        Some("한국어 스타일 매트릭스")
    } else if folder.starts_with("en-US-") {
        Some("English Style Matrix")
    } else if folder.starts_with("ru-RU-") {
        Some("Русская матрица стилей")
    } else {
        None
    }
}

/// 格式级额外标记
fn extra_markers(name_lower: &str) -> Vec<&'static str> {
    if name_lower.ends_with(".pdf") {
        // PDF 走 pdfium 整页渲染，正文在图片内、无文本可断言；
        // 校验渲染成功标记 + 页码标记
        vec!["data:image/png;base64,", "第 1 页"]
    } else if name_lower.ends_with(".txt") {
        vec!["doc-plain"]
    } else if name_lower.ends_with(".md") {
        vec!["<h1>"]
    } else if name_lower.ends_with(".docx") || name_lower.ends_with(".odt") {
        vec!["<h1>", "doc-table"]
    } else if name_lower.ends_with(".pptx") || name_lower.ends_with(".odp") {
        vec!["第 1 页"]
    } else {
        vec![] // doc/wps/ppt 旧版二进制：仅断言语言标记（无分页标记）
    }
}

fn page_zone(html: &str) -> &str {
    let s = html.find("const PAGES =").map(|i| i + "const PAGES =".len()).unwrap_or(0);
    let e = html.find("const FILEMAP =").unwrap_or(html.len());
    &html[s..e]
}

fn main() {
    // 收集 12 个语言-编码文件夹
    let mut folders: Vec<PathBuf> = std::fs::read_dir(input_dir()).unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && lang_marker(&p.file_name().unwrap().to_string_lossy()).is_some())
        .collect();
    folders.sort();

    let total_expect: usize = folders.iter().map(|f| {
        std::fs::read_dir(f).unwrap().filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file()
                && p.extension().and_then(|e| e.to_str()).map(is_page_ext).unwrap_or(false))
            .count()
    }).sum();

    println!("=== 阶段一：逐文件转换矩阵（{} 文件夹 / {} 个文件）===",
        folders.len(), total_expect);

    let mut fails: Vec<String> = vec![];
    let mut all_files: Vec<(String, PathBuf, String)> = vec![]; // (folder, path, name)

    for folder in &folders {
        let folder_name = folder.file_name().unwrap().to_string_lossy().to_string();
        let marker = lang_marker(&folder_name).unwrap();
        let mut files: Vec<PathBuf> = std::fs::read_dir(folder).unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file()
                && p.extension().and_then(|e| e.to_str()).map(is_page_ext).unwrap_or(false))
            .collect();
        files.sort();
        for p in &files {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let rel = format!("{}/{}", folder_name, name);
            let bytes_before = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let lower = name.to_ascii_lowercase();
            let result = load_page_content(p, &rel);
            match result {
                Ok(html) => {
                    let dump_dir = Path::new(&out_dir()).join("encoding_pages").join(&folder_name);
                    std::fs::create_dir_all(&dump_dir).unwrap();
                    std::fs::write(dump_dir.join(&name), &html).unwrap();
                    let mut ok = true;
                    let mut why: Vec<String> = vec![];
                    // PDF 渲染为整页图片，语言标记在图片内不可文本断言
                    if !lower.ends_with(".pdf") && !html.contains(marker) {
                        ok = false;
                        why.push(format!("缺语言标记「{}」", marker));
                    }
                    for m in extra_markers(&lower) {
                        if !html.contains(m) {
                            ok = false;
                            why.push(format!("缺标记「{}」", m));
                        }
                    }
                    if html.len() < 50 { ok = false; why.push("页面内容过短".into()); }
                    if html.contains("无法提取") { ok = false; why.push("意外降级占位".into()); }
                    let status = if ok { "PASS" } else { "FAIL" };
                    println!("  [{}] {:12}/{:32} {:>8}B → {}B {}",
                        status, folder_name, name, bytes_before, html.len(),
                        if why.is_empty() { String::new() } else { format!("({})", why.join("; ")) });
                    if !ok { fails.push(rel.clone()); }
                    all_files.push((folder_name.clone(), p.clone(), name.clone()));
                }
                Err(e) => {
                    println!("  [FAIL] {:12}/{:32} 转换错误: {}", folder_name, name, e);
                    fails.push(rel.clone());
                }
            }
        }
    }

    println!("\n=== 阶段二：全量合并打包 ===");
    // nav：每个文件夹一个分组
    let mut nav: Vec<NavNode> = vec![];
    for (folder_name, _, _) in &all_files {
        if nav.iter().any(|n| n.name == *folder_name) { continue; }
        let children = all_files.iter()
            .filter(|(f, _, _)| f == folder_name)
            .map(|(_, _, name)| NavNode::page(name.clone(), format!("{}/{}", folder_name, name)))
            .collect();
        nav.push(NavNode { kind: NodeKind::Text, name: folder_name.clone(), file: None, children });
    }
    let config = BundlerConfig {
        input_dirs: vec![input_dir()],
        title: "语言-编码矩阵合并测试".into(),
        breadcrumb_prefix: "矩阵测试 /".into(),
        nav,
        shell_style: ShellStyle::C,
        options: Options::default(),
        output_path: None,
        locale: None,
    };
    let result = match bundler::build_bundle(&config) {
        Ok(r) => r,
        Err(e) => { println!("[FATAL] 打包失败: {}", e); std::process::exit(1); }
    };
    println!("  页面数: {}（预期 {}）", result.stats.page_count, total_expect);
    println!("  最终体积: {:.2} MB", result.stats.final_bytes as f64 / 1024.0 / 1024.0);

    let html = &result.html;
    let zone = page_zone(html);
    let page_count = zone.matches("\"pg_").count();
    let raw_script = zone.matches("</script>").count();
    let doc_errors = zone.matches("无法提取").count();

    let out = Path::new(&out_dir()).join("语言编码矩阵合并测试-20261121.html");
    std::fs::write(&out, html).unwrap();
    println!("  产物已写入: {}", out.to_string_lossy());

    let mut bundle_fails: Vec<String> = vec![];
    macro_rules! check { ($name:expr, $cond:expr) => {
        println!("  {} {}", if $cond { "✓" } else { "✗" }, $name);
        if !$cond { bundle_fails.push($name.to_string()); }
    };}
    check!("页面数与文件数一致", result.stats.page_count == total_expect);
    check!("PAGES 区含全部页面", page_count >= total_expect);
    check!("PAGES 内无裸露 </script>", raw_script == 0);
    check!("全部文件无降级占位", doc_errors == 0);
    // 语言标记在合并产物中仍可寻
    let mut missing: Vec<String> = vec![];
    for (folder_name, _, _name) in &all_files {
        let marker = lang_marker(folder_name).unwrap();
        if !html.contains(marker) && !missing.contains(&marker.to_string()) {
            missing.push(marker.to_string());
        }
    }
    if missing.is_empty() {
        println!("  ✓ 合并产物语言标记校验通过");
    } else {
        for m in &missing {
            bundle_fails.push(format!("合并产物缺标记「{}」", m));
            println!("  ✗ 合并产物缺标记「{}」", m);
        }
    }

    println!("\n=== 结果 ===");
    if fails.is_empty() && bundle_fails.is_empty() {
        println!("ALL PASS（{} 个文件全部通过）", total_expect);
    } else {
        println!("FAILED: 逐文件 {:?}；合并 {:?}", fails, bundle_fails);
        std::process::exit(1);
    }
}
