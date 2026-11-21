//! 验收测试：复刻用户真实场景（含 _历史 版本目录 + 嵌套导航），验证修复效果。

use manualbundler::bundler;
use manualbundler::model::*;
use std::path::PathBuf;

fn main() {
    let input_dir = std::env::var("MB_AUDIT_INPUT_DIR")
        .unwrap_or_else(|_| "_test_out/pages".to_string());

    let config = BundlerConfig {
        input_dirs: vec![input_dir],
        title: "泳和批量改名".into(),
        breadcrumb_prefix: "泳和批量改名 / 用户手册 /".into(),
        nav: vec![
            NavNode {
                kind: NodeKind::Text,
                name: "首页".into(),
                file: None,
                children: vec![
                    NavNode::page("对比预览区", "对比预览区/对比预览区-手绘版-v3-20261119-0731.html"),
                ],
            },
            NavNode {
                kind: NodeKind::Text,
                name: "主功能区".into(),
                file: None,
                children: vec![
                    NavNode::page("匹配模式", "用户手册-匹配模式（点选拖拽匹配模式）v0.8.5.9.503 20261119-0635.html"),
                    NavNode::page("规则模式", "用户手册-规则模式.html"),
                    NavNode {
                        kind: NodeKind::Text,
                        name: "拼接类".into(),
                        file: None,
                        children: vec![
                            NavNode::page("元标签拼接模式", "用户手册-元标签拼接模式.html"),
                        ],
                    },
                ],
            },
        ],
        shell_style: ShellStyle::C,
        options: Options::default(),
        output_path: None,
        locale: None,
    };

    println!("[INFO] 开始打包...");
    match bundler::build_bundle(&config) {
        Ok(result) => {
            println!("=== 打包成功 ===");
            println!("  页面数: {}", result.stats.page_count);
            println!("  资源内嵌数: {}", result.stats.embedded_resource_count);
            println!("  最终体积: {:.2} MB", result.stats.final_bytes as f64 / 1024.0 / 1024.0);

            let out = PathBuf::from(std::env::var("MB_AUDIT_OUTPUT")
                .unwrap_or_else(|_| "_test_out/TEST-修复后.html".to_string()));
            std::fs::write(&out, &result.html).expect("写入失败");
            println!("[INFO] 已写入: {}", out.to_string_lossy());

            // 关键验证（PAGES 区间 = "const PAGES =" 到 "const FILEMAP =" 之间）
            let html = &result.html;
            let p_start = html.find("const PAGES =").map(|i| i + "const PAGES =".len()).unwrap_or(0);
            let p_end = html.find("const FILEMAP =").unwrap_or(html.len());
            let pages_zone = &html[p_start..p_end];
            let raw_script = pages_zone.matches("</script>").count();
            let escaped_script = pages_zone.matches("<\\/script>").count();
            let pages_count = pages_zone.matches("\"pg_").count();
            let checks: Vec<(&str, bool)> = vec![
                ("PAGES 内无裸露 </script>（P0 修复）", raw_script == 0),
                ("PAGES 内出现转义 <\\/script>（shim 保留）", escaped_script > 0),
                ("仅内嵌导航引用的 4 页（P1 修复）", pages_count == 4),
                ("面包屑含尾斜杠（P2 修复）", html.contains("用户手册 / <b>")),
                ("子分组 groups 序列化（P3）", html.contains("\"groups\":[{")),
                ("外壳 script 块完整收尾", html.contains("loadPage(") && html.rfind("</script>").is_some()),
            ];
            println!("\n=== 修复验证 ===");
            let mut all_ok = true;
            for (name, ok) in &checks {
                if !ok { all_ok = false; }
                println!("  {} {}", if *ok { "✓" } else { "✗" }, name);
            }
            if !all_ok { std::process::exit(1); }
        }
        Err(e) => {
            println!("[ERROR] 打包失败: {}", e);
            std::process::exit(1);
        }
    }
}
