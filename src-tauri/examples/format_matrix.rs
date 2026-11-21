//! 全格式合并测试：对 MB_MATRIX_INPUT_DIR 指定目录中全部支持格式测试文件
//! ① 逐文件转换矩阵（load_page_content 直测）
//! ② 全量合并打包（build_bundle）并断言产物
//! 任一断言失败退出码非 0。

use manualbundler::bundler::{self, is_page_ext, load_page_content};
use manualbundler::model::*;
use std::path::{Path, PathBuf};

fn input_dir() -> String {
    std::env::var("MB_MATRIX_INPUT_DIR").unwrap_or_else(|_| "_test_out/pages".to_string())
}
fn out_dir() -> String {
    std::env::var("MB_MATRIX_OUTPUT_DIR").unwrap_or_else(|_| "_test_out".to_string())
}

/// 每个测试文件的期望标记（大小写不敏感匹配文件名前缀）
fn expected_markers(file_name: &str) -> Vec<&'static str> {
    let f = file_name.to_ascii_lowercase();
    // v0.8.3 极限版：全格式样式矩阵
    if f.starts_with("test_file_1234567890-html") { return vec!["HTML 格式测试页"]; }
    if f.starts_with("test_file_1234567890-htm.") { return vec!["HTM 格式测试页"]; }
    if f.starts_with("test_file_1234567890-mht.") { return vec!["MHTML", "1234567890"]; }
    if f.starts_with("test_file_1234567890-mhtml") { return vec!["1234567890"]; }
    if f.starts_with("test_file_1234567890-md") { return vec!["全格式样式矩阵", "极限版", "<table"]; }
    if f.starts_with("test_file_1234567890-pdf") { return vec!["data:image/png;base64,"]; }
    if f.starts_with("test_file_1234567890-docx") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-doc.") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-odt") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-odp") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-ppt.") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-pptx2") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-rtf") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_file_1234567890-wps") { return vec!["全格式样式矩阵", "极限版"]; }
    if f.starts_with("test_document_1234567890-pptx") { return vec![]; } // 通用：非空+无降级
    if f == "test_txt.txt" { return vec!["doc-plain"]; }
    if f == "test_txt_-_copy.txt" { return vec!["文件内容为空"]; } // 空文件 → 预期降级占位
    if f.starts_with("transparent") { return vec!["data:image/png;base64,"]; }
    if f.starts_with("cexsbptcjq1ha") { return vec!["data:image/jpeg;base64,"]; }
    if f.ends_with(".gif") { return vec!["data:image/gif;base64,"]; }
    if f.ends_with(".webp") { return vec!["data:image/webp;base64,"]; }
    if f.ends_with(".bmp") { return vec!["data:image/bmp;base64,"]; }
    if f.ends_with(".svg") { return vec!["data:image/svg+xml;base64,"]; }
    if f.ends_with(".ico") { return vec!["data:image/x-icon;base64,"]; }
    if f.ends_with(".avif") { return vec!["data:image/avif;base64,"]; }
    vec![]
}

fn page_zone(html: &str) -> &str {
    let s = html.find("const PAGES =").map(|i| i + "const PAGES =".len()).unwrap_or(0);
    let e = html.find("const FILEMAP =").unwrap_or(html.len());
    &html[s..e]
}

fn main() {
    let dir = PathBuf::from(input_dir());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| p.extension().and_then(|e| e.to_str()).map(is_page_ext).unwrap_or(false))
        .collect();
    files.sort();

    println!("=== 阶段一：逐文件转换矩阵（{} 个支持格式页面）===", files.len());
    let mut fails: Vec<String> = vec![];
    for p in &files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let rel = name.clone();
        let bytes_before = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let result = load_page_content(p, &rel);
        match result {
            Ok(html) => {
                // 留存单页产物供人工核查
                let dump_dir = Path::new(&out_dir()).join("pages");
                std::fs::create_dir_all(&dump_dir).unwrap();
                std::fs::write(dump_dir.join(&name), &html).unwrap();
                let markers = expected_markers(&name);
                let mut ok = true;
                let mut why: Vec<String> = vec![];
                for m in &markers {
                    if !html.contains(m) {
                        ok = false;
                        why.push(format!("缺标记「{}」", m));
                    }
                }
                if html.len() < 50 { ok = false; why.push("页面内容过短".into()); }
                // 非预期降级检测（空 txt 除外）。注意：.doc-error 是模板 CSS 类，
                // 不能作为降级判据；真实降级正文以「无法提取」开头。
                let degrade = html.contains("无法提取");
                let expect_degrade = name.eq_ignore_ascii_case("Test_txt_-_Copy.txt");
                if degrade && !expect_degrade { ok = false; why.push("意外出现降级占位".into()); }
                if !degrade && expect_degrade { ok = false; why.push("空文件未按预期降级".into()); }
                let status = if ok { "PASS" } else { "FAIL" };
                println!("  [{}] {:45} {:>9}B → {}B {}", status, name, bytes_before, html.len(),
                    if why.is_empty() { String::new() } else { format!("({})", why.join("; ")) });
                if !ok { fails.push(name.clone()); }
            }
            Err(e) => {
                println!("  [FAIL] {:45} 转换错误: {}", name, e);
                fails.push(name.clone());
            }
        }
    }

    println!("\n=== 阶段二：全量合并打包 ===");
    let nav: Vec<NavNode> = files.iter()
        .map(|p| NavNode::page(p.file_name().unwrap().to_string_lossy().to_string(),
                               p.file_name().unwrap().to_string_lossy().to_string()))
        .collect();
    let config = BundlerConfig {
        input_dirs: vec![input_dir()],
        title: "全格式合并测试".into(),
        breadcrumb_prefix: "合并测试 /".into(),
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
    println!("  页面数: {}（预期 {}）", result.stats.page_count, files.len());
    println!("  资源内嵌数: {}", result.stats.embedded_resource_count);
    println!("  最终体积: {:.2} MB", result.stats.final_bytes as f64 / 1024.0 / 1024.0);

    let html = &result.html;
    let zone = page_zone(html);
    let page_count = zone.matches("\"pg_").count();
    let raw_script = zone.matches("</script>").count();
    let doc_errors = zone.matches("无法提取").count();

    std::fs::create_dir_all(out_dir()).unwrap();
    let out = Path::new(&out_dir()).join("全格式合并测试-20261120.html");
    std::fs::write(&out, html).unwrap();
    println!("  产物已写入: {}", out.to_string_lossy());

    let mut bundle_fails: Vec<String> = vec![];
    macro_rules! check { ($name:expr, $cond:expr) => {
        println!("  {} {}", if $cond { "✓" } else { "✗" }, $name);
        if !$cond { bundle_fails.push($name.to_string()); }
    };}
    check!("页面数与支持格式文件数一致", result.stats.page_count == files.len());
    check!("md 相对引用图片已内嵌（资源内嵌数 > 0）", result.stats.embedded_resource_count > 0);
    check!("PAGES 区含全部页面", page_count >= files.len());
    check!("PAGES 内无裸露 </script>", raw_script == 0);
    check!("空 txt 预期降级占位恰好 1 处", doc_errors == 1);
    check!("不支持格式（dps/et/xlsx）未被打包",
        !html.contains(".dps") && !html.contains(".et\"") && !html.contains(".xlsx"));
    // 每文件标记在最终产物中仍可寻（页面内容为 JSON 字符串，转义不影响中文与 data: 前缀）
    for p in &files {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        for m in expected_markers(&name) {
            if !html.contains(m) {
                bundle_fails.push(format!("合并产物缺「{}」: {}", name, m));
                println!("  ✗ 合并产物缺「{}」: {}", name, m);
            }
        }
    }
    if bundle_fails.is_empty() { println!("  ✓ 合并产物全标记校验通过"); }

    println!("\n=== 结果 ===");
    if fails.is_empty() && bundle_fails.is_empty() {
        println!("ALL PASS（{} 页面全部通过）", files.len());
    } else {
        println!("FAILED: 逐文件 {:?}；合并 {:?}", fails, bundle_fails);
        std::process::exit(1);
    }
}
