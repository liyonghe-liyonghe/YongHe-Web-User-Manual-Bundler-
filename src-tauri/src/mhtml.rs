//! MHTML（.mht / .mhtml）解析与转换。
//!
//! 浏览器「另存为 MHTML」产生的文件是 multipart/related MIME 结构：
//! 主体 text/html 部分通常为 quoted-printable 编码，其余部分（图片/CSS/字体等）
//! 多为 base64 编码，各自带 Content-Location（或 Content-ID）标识。
//!
//! 本模块将 MHTML 转换为普通 HTML：HTML 部分解码后，把其中出现的资源 URL
//! 全部替换为对应的 data URI；text/css 部分先替换其内部 url() 引用，
//! 再作为 data:text/css 内联（这样 `<link href>` 也能被统一替换）。

use crate::error::AppError;
use base64::Engine;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct Part {
    content_type: String,      // 小写 media-type（不含参数），如 "text/html"
    transfer_encoding: String, // 小写，如 "quoted-printable" / "base64"
    location: Option<String>,  // Content-Location
    cid: Option<String>,       // Content-ID（去掉尖括号）
    body: Vec<u8>,
}

impl Part {
    /// 按传输编码解码为字节
    fn decoded_bytes(&self) -> Vec<u8> {
        match self.transfer_encoding.as_str() {
            "base64" => decode_base64_relaxed(&String::from_utf8_lossy(&self.body))
                .unwrap_or_else(|| self.body.clone()),
            "quoted-printable" => decode_quoted_printable(&self.body),
            _ => self.body.clone(), // 7bit / 8bit / binary / 未声明
        }
    }

    fn decoded_text(&self) -> String {
        String::from_utf8_lossy(&self.decoded_bytes()).into_owned()
    }
}

/// 把 MHTML 转换为资源全部内嵌（data URI）的普通 HTML
pub fn convert_mhtml_to_html(bytes: &[u8]) -> Result<String, AppError> {
    let text = String::from_utf8_lossy(bytes).into_owned();

    // 1. 从顶层头部提取 boundary（仅搜索首个空行之前的头部区域）
    let header_region = header_region_of(&text);
    let boundary = find_boundary(header_region)
        .or_else(|| find_boundary(&text[..text.len().min(4096)]))
        .ok_or_else(|| AppError::BundleError("MHTML 解析失败: 未找到 multipart boundary".into()))?;

    // 2. 递归收集所有叶子 part（处理嵌套 multipart）
    let mut parts: Vec<Part> = vec![];
    collect_parts(&text, &boundary, 0, &mut parts);

    // 3. 找 HTML 主体
    let html_part = parts
        .iter()
        .find(|p| p.content_type.starts_with("text/html"))
        .ok_or_else(|| AppError::BundleError("MHTML 解析失败: 未找到 text/html 主体".into()))?;
    let mut html = html_part.decoded_text();

    // 4. 第一轮：图片/字体等 → data URI；CSS 先留待下一轮
    let mut res_map: HashMap<String, String> = HashMap::new(); // Content-Location → data URI
    let mut cid_map: Vec<(String, String)> = vec![];           // "cid:xxx" → data URI
    let mut css_parts: Vec<(String, String)> = vec![];         // location → css 原文
    for p in &parts {
        if p.content_type.starts_with("text/html") {
            continue;
        }
        let ct = if p.content_type.is_empty() { "application/octet-stream" } else { p.content_type.as_str() };
        if p.content_type.starts_with("text/css") {
            css_parts.push((p.location.clone().unwrap_or_default(), p.decoded_text()));
            continue;
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&p.body);
        let uri = format!("data:{};base64,{}", ct, b64);
        if let Some(loc) = &p.location {
            res_map.insert(loc.clone(), uri.clone());
        }
        if let Some(cid) = &p.cid {
            if !cid.is_empty() {
                cid_map.push((format!("cid:{}", cid), uri));
            }
        }
    }

    // 5. 第二轮：CSS 内部 url() 引用替换后再编码为 data:text/css（供 <link href> 统一替换）
    for (loc, css_text) in &css_parts {
        let mut replaced = css_text.clone();
        replace_urls(&mut replaced, &res_map);
        let uri = format!(
            "data:text/css;charset=utf-8;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(replaced.as_bytes())
        );
        if !loc.is_empty() {
            res_map.insert(loc.clone(), uri);
        }
    }

    // 6. 第三轮：HTML 中 URL → data URI（含 HTML 转义变体），再处理 cid: 引用
    replace_urls(&mut html, &res_map);
    for (cid, uri) in &cid_map {
        html = html.replace(cid.as_str(), uri.as_str());
    }

    Ok(html)
}

// ==================== MIME 结构解析 ====================

/// 返回首个空行（头部结束）之前的文本，即头部区域
fn header_region_of(text: &str) -> &str {
    let idx = find_header_end(text);
    &text[..idx]
}

/// 找首个空行位置（\r\n\r\n 或 \n\n），找不到则返回全文长度
fn find_header_end(text: &str) -> usize {
    let mut search_from = 0;
    while let Some(nl) = text[search_from..].find('\n') {
        let abs = search_from + nl;
        let rest = &text[abs + 1..];
        if rest.starts_with("\r\n") {
            return abs + 3;
        }
        if rest.starts_with('\n') {
            return abs + 2;
        }
        search_from = abs + 1;
        if search_from >= text.len() {
            break;
        }
    }
    text.len()
}

/// 从文本头部提取 boundary="..." 或 boundary=xxx（不区分大小写）
fn find_boundary(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mut search_pos = 0;
    while let Some(rel) = lower[search_pos..].find("boundary=") {
        let pos = search_pos + rel;
        let rest = text[pos + 9..].trim_start();
        if let Some(stripped) = rest.strip_prefix('"') {
            if let Some(end) = stripped.find('"') {
                if !stripped[..end].is_empty() {
                    return Some(stripped[..end].to_string());
                }
            }
        } else {
            let end = rest
                .find(|c: char| c == '\r' || c == '\n' || c == ';' || c == ' ')
                .unwrap_or(rest.len());
            let val = rest[..end].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
        search_pos = pos + 9;
    }
    None
}

/// 按 boundary 递归收集叶子 part
fn collect_parts(text: &str, boundary: &str, depth: usize, out: &mut Vec<Part>) {
    if depth > 4 {
        return;
    }
    let delimiter = format!("--{}", boundary);
    for segment in text.split(delimiter.as_str()) {
        let seg = segment.trim_start_matches("\r\n").trim_start_matches('\n');
        let seg_trimmed = seg.trim();
        // 结束标记 "--" 或空段跳过
        if seg_trimmed.is_empty() || seg_trimmed == "--" || seg_trimmed.starts_with("--\r") || seg_trimmed.starts_with("--\n") {
            continue;
        }
        let end = find_header_end(seg);
        let (header_text, body_text) = seg.split_at(end.min(seg.len()));

        // 解析头部（含折叠行续行）
        let mut headers: Vec<(String, String)> = vec![];
        for line in header_text.lines() {
            if line.starts_with(' ') || line.starts_with('\t') {
                if let Some(last) = headers.last_mut() {
                    last.1.push(' ');
                    last.1.push_str(line.trim());
                }
            } else if let Some(colon) = line.find(':') {
                headers.push((
                    line[..colon].trim().to_lowercase(),
                    line[colon + 1..].trim().to_string(),
                ));
            }
        }
        let get = |name: &str| -> Option<String> {
            headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        };
        let content_type = get("content-type").unwrap_or_default().to_lowercase();
        let transfer_encoding = get("content-transfer-encoding").unwrap_or_default().to_lowercase();
        let location = get("content-location");
        let cid = get("content-id").map(|v| v.trim_matches(|c| c == '<' || c == '>').to_string());

        // 嵌套 multipart：用其自身 boundary 递归
        if content_type.starts_with("multipart/") {
            if let Some(inner_boundary) = find_boundary(&content_type) {
                collect_parts(body_text, &inner_boundary, depth + 1, out);
                continue;
            }
        }

        out.push(Part {
            content_type,
            transfer_encoding,
            location,
            cid,
            body: body_text.as_bytes().to_vec(),
        });
    }
}

// ==================== 编码解码 ====================

fn decode_quoted_printable(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == b'=' {
            // 软换行：行尾的 =\r\n 或 =\n 直接丢弃
            if i + 2 < input.len() && input[i + 1] == b'\r' && input[i + 2] == b'\n' {
                i += 3;
                continue;
            }
            if i + 1 < input.len() && input[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            // =XX 十六进制转义
            if i + 2 < input.len() {
                let h = (input[i + 1] as char).to_digit(16);
                let l = (input[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (h, l) {
                    out.push(((h << 4) | l) as u8);
                    i += 3;
                    continue;
                }
            }
            out.push(b); // 异常 '=' 原样保留
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

/// 宽松 base64：去空白，失败时补 padding 重试
fn decode_base64_relaxed(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let eng = &base64::engine::general_purpose::STANDARD;
    if let Ok(v) = eng.decode(cleaned.as_bytes()) {
        return Some(v);
    }
    let mut padded = cleaned.clone();
    let rem = padded.len() % 4;
    if rem != 0 {
        padded.push_str(&"=".repeat(4 - rem));
    }
    eng.decode(padded.as_bytes()).ok()
}

// ==================== URL → data URI 替换 ====================

/// 把 map 中的 URL 替换为对应 data URI；长 URL 优先，避免前缀误替换。
/// 同时替换 HTML 转义变体（& → &amp;）。
fn replace_urls(target: &mut String, map: &HashMap<String, String>) {
    if map.is_empty() {
        return;
    }
    let mut entries: Vec<(&String, &String)> = map.iter().collect();
    entries.sort_by_key(|(url, _)| std::cmp::Reverse(url.len()));
    for (url, uri) in entries {
        *target = target.replace(url.as_str(), uri.as_str());
        let escaped = url.replace('&', "&amp;");
        if escaped != *url {
            *target = target.replace(escaped.as_str(), uri.as_str());
        }
    }
}

// ==================== 提取标题 ====================

/// 从（已转换的）HTML 文本中提取 <title>
pub fn extract_html_title(html: &str) -> Option<String> {
    let start = html.find("<title")?;
    let after = &html[start..];
    let close = after.find('>')?;
    let rest = &after[close + 1..];
    let end = rest.find("</title")?;
    let title = rest[..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"From: <Saved by Blink>
Snapshot-Content-Location: https://example.com/page
MIME-Version: 1.0
Content-Type: multipart/related;
	type="text/html";
	boundary="----MultipartBoundary--TESTBOUND----"


------MultipartBoundary--TESTBOUND----
Content-Type: text/html
Content-ID: <frame-abc@mhtml.blink>
Content-Transfer-Encoding: quoted-printable
Content-Location: https://example.com/page

<!DOCTYPE html><html><head><meta charset=3D"utf-8"><title>=E6=B5=8B=E8=AF=95=E9=A1=B5</title>
<link rel=3D"stylesheet" href=3D"https://example.com/style.css">
<style>body{background:url("https://example.com/style-bg.png")}</style></head>
<body><img src=3D"https://example.com/img.png" alt="a&amp;b">
<img src=3D"cid:icon-1@mhtml.blink"><a href=3D"https://example.com/link?a=3D1&b=3D2">x</a>
</body></html>

------MultipartBoundary--TESTBOUND----
Content-Type: text/css
Content-Transfer-Encoding: quoted-printable
Content-Location: https://example.com/style.css

body{color:red;background:url(https://example.com/style-bg.png)}

------MultipartBoundary--TESTBOUND----
Content-Type: image/png
Content-Transfer-Encoding: base64
Content-Location: https://example.com/img.png

iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==

------MultipartBoundary--TESTBOUND----
Content-Type: image/png
Content-Transfer-Encoding: base64
Content-Location: https://example.com/style-bg.png

iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEElEQVR4nGP8z8DwnwEIwAgA/ACqATcBAAAAAElFTkSuQmCC

------MultipartBoundary--TESTBOUND----
Content-Type: image/png
Content-Transfer-Encoding: base64
Content-ID: <icon-1@mhtml.blink>

iVBORw0KGgo=

------MultipartBoundary--TESTBOUND------
"#;

    #[test]
    fn test_convert_basic() {
        let html = convert_mhtml_to_html(SAMPLE.as_bytes()).unwrap();
        // QP 解码出的中文标题
        assert!(html.contains("测试页"), "title QP decode failed: {}", html);
        // 图片 URL → data URI
        assert!(html.contains("data:image/png;base64,"), "img not inlined");
        assert!(!html.contains("src=\"https://example.com/img.png\""), "img url not replaced");
        // CSS 以 data URI 内联
        assert!(html.contains("data:text/css;charset=utf-8;base64,"), "css not inlined");
        // HTML 转义变体也被替换
        assert!(!html.contains("href=\"https://example.com/style.css\""), "css link not replaced");
        // cid 引用
        assert!(html.matches("data:image/png;base64,").count() >= 2, "cid img missing");
        assert!(!html.contains("cid:icon-1@mhtml.blink"), "cid not replaced");
        // CSS 内部 url() 也被替换（data uri 内含 image/png）
        assert!(html.contains("data:image/png"), "css url() not replaced");
        // 原始 URL 不应残留（&amp; 变体）
        assert!(!html.contains("https://example.com/img.png"), "raw img url remains");
        assert!(!html.contains("https://example.com/style-bg.png"), "raw css url remains");
    }

    #[test]
    fn test_extract_title() {
        let html = convert_mhtml_to_html(SAMPLE.as_bytes()).unwrap();
        assert_eq!(extract_html_title(&html).as_deref(), Some("测试页"));
    }

    #[test]
    fn test_qp_decode() {
        // a=3Db=\r\nc → "a=" 解码为 '='，软换行移除 → "a=bc"
        assert_eq!(decode_quoted_printable(b"a=3Db=\r\nc"), b"a=bc");
        assert_eq!(decode_quoted_printable(b"x=41=42"), b"xAB");
        assert_eq!(decode_quoted_printable(b"line1=\nline2"), b"line1line2");
    }

    #[test]
    fn test_base64_relaxed() {
        assert_eq!(decode_base64_relaxed("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64_relaxed("aGVs\nbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64_relaxed("aGVsbG8"), Some(b"hello".to_vec()));
    }

    #[test]
    fn test_no_html_part() {
        let bad = "Content-Type: multipart/related; boundary=\"XX\"\r\n\r\n--XX--\r\n";
        assert!(convert_mhtml_to_html(bad.as_bytes()).is_err());
    }
}
