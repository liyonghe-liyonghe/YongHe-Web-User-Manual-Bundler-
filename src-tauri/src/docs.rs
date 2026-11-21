//! 文档格式导入支持：PDF / Word(doc,docx) / RTF / WPS / ODF(odt,odp) / PPT(ppt,pptx) / TXT / MD
//! → 统一转换为可在手册中浏览的 HTML 页面。
//!
//! 还原能力（0.7.3 起）：
//! - docx：标题/对齐/加粗/斜体/下划线/颜色/字号/列表/**表格**/**内嵌图片**/**超链接**
//! - pptx：按形状 y 坐标排序的排版、标题占位符、**内嵌图片**、表格、对齐
//! - odt：标题/对齐/粗斜下划线（自动样式）、**内嵌图片**（href 或 binary-data）
//! - odp：按 frame y 坐标排序、**内嵌图片**、表格、样式
//! - rtf：加粗/斜体/下划线/字号、段落对齐、**\pict 图片**（png/jpeg 十六进制或二进制数据）
//! - pdf：**pdfium 逐页渲染为高清 PNG**（完整还原排版与图片，中文完美）；
//!        pdfium.dll 缺失时自动降级为 pdf-extract 文本提取
//! - doc/wps/ppt 旧二进制格式无公开完整规范，保持启发式文本提取（页内附提示）
//! - txt/md：纯文本 / Markdown（pulldown-cmark，图片由打包器内嵌）
//!
//! 任一转换失败不中断打包，降级为带原因提示的占位页。

use crate::error::AppError;
use std::collections::HashMap;
use std::io::Read;

// ==================== 多编码文本解码（0.8.4） ====================

/// 常用汉字集：jieba 词典单字频率 Top1500（简体）∪ Big5 Level-1 常用区（繁体），
/// 用于区分 GBK/Big5 等结构性不可分的双字节编码——正确解码的文本绝大部分
/// 落在常用集内，错误解码（如 Big5 字节被当 GBK 读）则在 GB2312 全区均匀散布，
/// 常用集命中率显著更低。
const COMMON_HANZI: &str = include_str!("common_hanzi.txt");

fn is_common_hanzi(c: char) -> bool {
    static SET: std::sync::OnceLock<std::collections::HashSet<char>> = std::sync::OnceLock::new();
    SET.get_or_init(|| COMMON_HANZI.chars().collect()).contains(&c)
}

/// 常用假名集（含清浊音/拗音/促音/长音）：正确日文解码高频命中，
/// 韩文/中文字节错位解码洪泛出的假名极少命中常用集
const COMMON_KANA: &str = concat!(
    "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん",
    "がぎぐげござじずぜぞだぢづでどばびぶべぼぱぴぷぺぽゃゅょっー",
    "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン",
    "ガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポャュョッー、。",
);

/// 常用谚文音节集 = KS X 1001 完成형 2350 音节（EUC-KR 标准区 lead B0-C8）。
/// 正确韩文解码的音节几乎全部落在此区；UHC 扩展区（洪泛误码来源）的生僻音节不命中。
const COMMON_HANGUL: &str = include_str!("common_hangul.txt");

/// 俄语高频西里尔字母（含大写）：KOI8-R/IBM866 与 cp1251 的互洪泛区分、
/// 以及希腊/希伯来/阿拉伯字节流被误读为西里尔时的高频命中率差异。
const COMMON_CYRILLIC: &str = "оеаинтсрвлкмдпуяОЕАИНТСРВЛКМДПУЯ";

/// 土耳其语专属字母：cp1254 中为 ğ ı İ ş Ş；同字节在 cp1252 下是 ð/Þ/ý/Ý 等生僻拉丁，
/// 生僻拉丁重罚 + 土耳其专属加分可无洪泛地分离两者。
const COMMON_TURKISH: &str = "ğĞıİşŞ";

/// 阿拉伯语高频字母（ا ل ي م ن و ر ت ب 等覆盖真实文本 ~70%）：
/// KOI8/cp1251 字节流被 cp1256 洪泛成阿拉伯文时字母分布均匀，
/// 高频集命中率显著更低，借此分离（全部平权会反超 KOI8 正确解码）
const COMMON_ARABIC: &str = "الينمورتبهعسكفدصشأإئةى";

/// 生僻拉丁字母：西欧文本极少出现，多为 cp1254/cp1257 误解码产物（ğıİşŞ 的 cp1252 形）
const RARE_LATIN: &str = "ðÐþÞýÝ";

fn is_common_cyrillic(c: char) -> bool {
    static SET: std::sync::OnceLock<std::collections::HashSet<char>> = std::sync::OnceLock::new();
    SET.get_or_init(|| COMMON_CYRILLIC.chars().collect()).contains(&c)
}

fn is_common_of(set: &'static str, c: char) -> bool {
    static KANA: std::sync::OnceLock<std::collections::HashSet<char>> = std::sync::OnceLock::new();
    static HANGUL: std::sync::OnceLock<std::collections::HashSet<char>> = std::sync::OnceLock::new();
    match set {
        "kana" => KANA.get_or_init(|| COMMON_KANA.chars().collect()).contains(&c),
        _ => HANGUL.get_or_init(|| COMMON_HANGUL.chars().collect()).contains(&c),
    }
}

fn in_static_set(set: &'static str, c: char) -> bool {
    // 各集合均极小（≤40 字符），无需缓存直接遍历
    set.chars().any(|x| x == c)
}

/// 西里尔/希腊/希伯来/阿拉伯/泰文：与 ASCII 字母词内混排是误解码特征，
/// 真实文本中这类文字词内不夹拉丁字母（混排处的相邻对按次扣分）
fn is_special_script(u: u32) -> bool {
    matches!(u,
        0x0400..=0x04FF      // 西里尔
        | 0x0370..=0x03FF | 0x1F00..=0x1FFF // 希腊
        | 0x0590..=0x05FF    // 希伯来
        | 0x0600..=0x06FF    // 阿拉伯
        | 0x0E00..=0x0E7F    // 泰文
    )
}

/// 汉字或假名（含兼容表意区与半角假名）：用于谚文混排惩罚
fn is_hanzi_or_kana(u: u32) -> bool {
    matches!(u,
        0x3040..=0x30FF      // 假名
        | 0xFF66..=0xFF9D    // 半角假名
        | 0x4E00..=0x9FFF    // 常用汉字区
        | 0x3400..=0x4DBF    // 扩展 A
        | 0xF900..=0xFAFF    // 兼容表意（含兼容谚文）
    )
}

/// CJK 候选的语种门槛：非本语种字节流被双字节候选洪泛成"汉字/汉音字"时，
/// 本语种特征字符占比必然极低，按比例大幅折分（0.8.5）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CjkGate {
    None,
    /// 日文候选：真实日文正文假名占比 ≥12%（纯汉字日文极罕见）
    Kana,
    /// 韩文候选：真实韩文正文谚文占比 ≥50%（现代韩文几乎纯谚文）
    Hangul,
    /// 简繁中文候选：汉字即本语种，无需门槛（误码由生僻字权重压制）
    Hanzi,
}

fn non_ascii_script_ratio(s: &str, pred: impl Fn(u32) -> bool) -> f64 {
    let mut total = 0usize;
    let mut hit = 0usize;
    for c in s.chars() {
        let u = c as u32;
        if u < 0x80 { continue; }
        total += 1;
        if pred(u) { hit += 1; }
    }
    if total == 0 { 0.0 } else { hit as f64 / total as f64 }
}

/// 对候选编码解码结果打分。
/// 权重标定原则：单字节编码每 1 字节产出 1 个字符、双字节编码每 2 字节产出
/// 1 个字符，故西里尔/拉丁字母权重须显著低于 CJK 常用字，否则 cp1251 会把
/// GBK 字节流洪泛成西里尔字母反超正确解码（0.8.4 矩阵测试实测）。
/// `single_byte_scale`：单字节编码再乘 (1-高字节占比)——真实西文/俄文文本
/// 高字节占比通常 <0.6，而 CJK 双字节流接近 1.0，作为兜底抑制。
fn score_decoded(s: &str, single_byte_scale: f64, gate: CjkGate) -> f64 {
    let mut score = 0.0f64;
    let mut prev: Option<char> = None;
    for c in s.chars() {
        let u = c as u32;
        score += match u {
            0xFFFD => -4.0,                                    // 替换符：编码错位强信号
            0x0400..=0x04FF => {
                // 西里尔（俄语高频/生僻；小写高频加权——KOI8/IBM866 字节被 cp1251
                // 误读时会整体变成大写，借此分离）
                if is_common_cyrillic(c) {
                    if c.is_uppercase() { 1.2 } else { 2.0 }
                } else {
                    0.6
                }
            }
            0x0370..=0x03FF | 0x1F00..=0x1FFF => {
                // 希腊：带调元音（άέήίόύώϊϋΐΰ 等）单独低权——俄文字节经 cp1253
                // 误读时大量命中带调位，以此压制洪泛
                let accented = matches!(c, 'ά'|'έ'|'ή'|'ί'|'ό'|'ύ'|'ώ'|'ϊ'|'ϋ'|'ΐ'|'ΰ'
                    |'Ά'|'Έ'|'Ή'|'Ί'|'Ό'|'Ύ'|'Ώ'|'Ϊ'|'Ϋ');
                if c.is_alphabetic() && !accented { 1.7 } else if c.is_alphabetic() { 0.5 } else { 0.0 }
            }
            0x0590..=0x05FF => {
                if c.is_alphabetic() { 1.7 } else { 0.5 }      // 希伯来（字母/注音点）
            }
            0x0600..=0x06FF => {
                if in_static_set(COMMON_ARABIC, c) { 1.7 } else { 0.5 } // 阿拉伯（高频/生僻）
            }
            0x0E00..=0x0E7F => {
                if c.is_alphabetic() { 1.7 } else { 0.5 }      // 泰文
            }
            0x3040..=0x30FF => {
                if is_common_of("kana", c) { 2.5 } else { 0.3 } // 假名（常用/生僻）
            }
            0xFF66..=0xFF9D => -2.0,                           // 半角假名：洪泛误码特征
            0xAC00..=0xD7A3 => {
                if is_common_of("hangul", c) { 2.5 } else { 0.3 } // 谚文（常用/生僻）
            }
            0x00A0..=0x00BF => -3.0,                           // Latin-1 符号区（¤©«°±µ·»№等）：误解码特征
            0x00C0..=0x00FF | 0x0100..=0x024F | 0x1E00..=0x1EFF => {
                if in_static_set(RARE_LATIN, c) {
                    0.3                                        // ðÞý 等生僻拉丁：cp1254 误解码特征
                } else if in_static_set(COMMON_TURKISH, c) {
                    1.8                                        // 土耳其专属字母
                } else if (0x1EA0..=0x1EF9).contains(&u) || matches!(c, 'ơ'|'Ơ'|'ư'|'Ư'|'đ'|'Đ') {
                    1.8                                        // 越南语专属字母（拉丁扩展附加区）
                } else if c.is_alphabetic() {
                    1.0                                        // 拉丁扩展：法德西葡等重音字母
                } else {
                    -3.0                                       // 拉丁区符号：误解码重罚
                }
            }
            0x0080..=0x009F => -4.0,                           // C1 控制符：误解码产物
            0x2013 | 0x2014 | 0x2018 | 0x2019 | 0x201C | 0x201D | 0x2022 | 0x2026 | 0x2122 => {
                -0.5                                           // cp12xx 误解码高产符号（‚„…†‡•—™）
            }
            0x2E80..=0x2EFF | 0x2F00..=0x2FDF | 0xF900..=0xFAFF | 0x3400..=0x4DBF => {
                -2.0                                            // 部首/兼容/扩展A：误解码偏爱区块
            }
            0x4E00..=0x9FFF => {
                if is_common_hanzi(c) { 2.5 } else { -1.5 }
            }
            0x3000..=0x303F | 0xFF00..=0xFF65 | 0xFF9E..=0xFFEF => 0.5, // CJK 标点/全角
            _ => {
                if c.is_ascii_graphic() || c == ' ' { 0.1 } else { 0.0 }
            }
        };
        // 词内混排惩罚：ASCII 字母与特殊文字（西里尔/希腊/希伯来/阿拉伯/泰文）相邻
        // ——法德西葡的重音字母被 cp1251 误读为西里尔时必然嵌入 ASCII 词内，
        // 而真实俄文/希腊文等词内不夹 ASCII 字母（空格/标点分隔），借此分离洪泛
        if let Some(p) = prev {
            let pu = p as u32;
            let p_special = is_special_script(pu);
            let c_special = is_special_script(u);
            if (p_special && c.is_ascii_alphabetic()) || (c_special && p.is_ascii_alphabetic()) {
                score -= 1.5;
            }
            // 谚文与汉字/假名词内混排：GBK/EUC-KR 把非 CJK 字节流洪泛成谚文夹汉字的
            // 特征（现代韩文正文几乎纯谚文；日文假名夹汉字属正常排版，不受罚）
            let p_hangul = (0xAC00..=0xD7A3).contains(&pu);
            let c_hangul = (0xAC00..=0xD7A3).contains(&u);
            let p_hkana = is_hanzi_or_kana(pu);
            let c_hkana = is_hanzi_or_kana(u);
            if (p_hangul && c_hkana) || (c_hangul && p_hkana) {
                score -= 3.0;
            }
        }
        prev = Some(c);
    }
    let mut score = score * single_byte_scale;
    // 语种门槛折分：假名/谚文特征占比不足的双字节解码视为洪泛误码
    match gate {
        CjkGate::Kana => {
            if non_ascii_script_ratio(s, |u| (0x3040..=0x30FF).contains(&u)) < 0.12 {
                score *= 0.3;
            }
        }
        CjkGate::Hangul => {
            if non_ascii_script_ratio(s, |u| (0xAC00..=0xD7A3).contains(&u)) < 0.5 {
                score *= 0.3;
            }
        }
        _ => {}
    }
    score
}

/// 宽容多编码解码（0.8.4 起取代 UTF-8→GBK 双兜底；0.8.5 扩充候选至 19 种）：
/// BOM → UTF-8 → 候选编码评分取优（单字节编码按高字节密度缩放）。
fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE.decode(&bytes[2..]).0.into_owned();
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return encoding_rs::UTF_16BE.decode(&bytes[2..]).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // 候选顺序即同分决胜顺序：GBK 与 cp1251 各自语种优先；
            // cp1252 先于 cp1250/ISO-8859-2/ISO-8859-15（0.8.4 既有行为），
            // cp1253/cp1255/cp1256 依赖词内混排惩罚压制对西文的洪泛。
            let candidates: [(&'static encoding_rs::Encoding, CjkGate); 19] = [
                // CJK（GB18030 为 GBK 超集，GBK 优先保证简体字节序稳定）
                (encoding_rs::GBK, CjkGate::Hanzi),
                (encoding_rs::GB18030, CjkGate::Hanzi),
                (encoding_rs::BIG5, CjkGate::Hanzi),
                (encoding_rs::SHIFT_JIS, CjkGate::Kana),
                (encoding_rs::EUC_JP, CjkGate::Kana),
                (encoding_rs::EUC_KR, CjkGate::Hangul),
                // 西里尔（cp1251 为 Windows 俄文最常见，KOI8/IBM866 次之）
                (encoding_rs::WINDOWS_1251, CjkGate::None),
                (encoding_rs::KOI8_R, CjkGate::None),
                (encoding_rs::KOI8_U, CjkGate::None),
                (encoding_rs::IBM866, CjkGate::None),
                // 西欧/中欧
                (encoding_rs::WINDOWS_1252, CjkGate::None),
                (encoding_rs::ISO_8859_15, CjkGate::None),
                (encoding_rs::WINDOWS_1250, CjkGate::None),
                (encoding_rs::ISO_8859_2, CjkGate::None),
                // 土耳其（ğ ı İ ş Ş 专属加分，与 cp1252 重叠字节为生僻拉丁）
                (encoding_rs::WINDOWS_1254, CjkGate::None),
                // 希腊 / 希伯来 / 阿拉伯
                (encoding_rs::WINDOWS_1253, CjkGate::None),
                (encoding_rs::ISO_8859_7, CjkGate::None),
                (encoding_rs::WINDOWS_1255, CjkGate::None),
                (encoding_rs::WINDOWS_1256, CjkGate::None),
                // 泰文 TIS-620 字节区与 GBK 一级常用字行结构性重叠，
                // 无 n-gram 模型无法稳定分离，暂不纳入候选（0.8.4 行为不变）
            ];
            // 高字节占比：单字节编码候选的得分缩放因子
            let high = bytes.iter().filter(|b| **b >= 0x80).count();
            let scale = 1.0 - (high as f64 / bytes.len().max(1) as f64);
            let mut best_score = f64::MIN;
            let mut best_text = String::from_utf8_lossy(bytes).into_owned();
            for (enc, gate) in candidates {
                let (cow, _, _) = enc.decode(bytes);
                let s = score_decoded(
                    &cow,
                    if enc.is_single_byte() { scale.max(0.0) } else { 1.0 },
                    gate,
                );
                if s > best_score {
                    best_score = s;
                    best_text = cow.into_owned();
                }
            }
            best_text
        }
    }
}

// ==================== 类型判定 ====================

pub fn is_doc_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "pdf" | "doc" | "docx" | "rtf" | "wps" | "odt" | "odp" | "ppt" | "pptx" | "txt" | "md"
    )
}

fn ext_label(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => "PDF 文档",
        "doc" => "Word 文档(旧版)",
        "docx" => "Word 文档",
        "rtf" => "RTF 文档",
        "wps" => "WPS 文档",
        "odt" => "ODF 文字",
        "odp" => "ODF 演示",
        "ppt" => "PPT 演示(旧版)",
        "pptx" => "PPT 演示",
        "txt" => "纯文本",
        "md" => "Markdown",
        _ => "文档",
    }
}

/// 文件名去扩展名 + HTML 转义，作为页面标题
fn doc_title(rel: &str) -> String {
    let normalized = rel.replace('\\', "/");
    let stem = std::path::Path::new(&normalized)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel);
    escape_html(stem)
}

// ==================== 入口 ====================

/// 转换文档字节为完整 HTML 页面。永不返回 Err：内部失败时降级为提示页，
/// 并对不可控 panic 兜底，保证打包流程不中断。
pub fn convert_doc_to_html(bytes: &[u8], ext: &str, rel: &str) -> String {
    let title = doc_title(rel);
    let lower = ext.to_ascii_lowercase();
    let body = match std::panic::catch_unwind(|| convert_body(bytes, &lower)) {
        Ok(Ok(html_body)) => html_body,
        Ok(Err(e)) => degrade_body(&title, &lower, &format!("{}", e)),
        Err(_) => degrade_body(&title, &lower, "转换过程发生内部异常"),
    };
    doc_page_html(&title, &body, &lower)
}

fn convert_body(bytes: &[u8], ext: &str) -> Result<String, AppError> {
    if bytes.is_empty() {
        return Err(AppError::BundleError("文件内容为空".into()));
    }
    match ext {
        "txt" => Ok(txt_body(bytes)),
        "md" => md_body(bytes),
        "rtf" => rtf_body(bytes),
        "pdf" => pdf_body(bytes),
        "docx" => docx_body(bytes),
        "pptx" => pptx_body(bytes),
        "odt" => odt_body(bytes),
        "odp" => odp_body(bytes),
        // 旧版二进制格式：内部按魔数二次识别（可能是 zip 容器或 OLE）
        "doc" | "wps" | "ppt" => legacy_office_body(bytes, ext),
        _ => Err(AppError::BundleError(format!("不支持的文档格式: {}", ext))),
    }
}

/// 转换失败时的占位正文
fn degrade_body(title: &str, ext: &str, reason: &str) -> String {
    format!(
        "<div class=\"doc-error\"><h2>无法提取「{}」({}) 的内容</h2>\
         <p>原因：{}</p>\
         <p>建议：确认文件未损坏；若为旧版二进制格式（doc/wps/ppt），\
         可先另存为 docx/pptx 或导出 PDF 后重新导入。</p></div>",
        title,
        ext_label(ext),
        escape_html(reason)
    )
}

// ==================== 各格式转换器 ====================

fn txt_body(bytes: &[u8]) -> String {
    let text = decode_text(bytes);
    format!("<pre class=\"doc-plain\">{}</pre>", escape_html(&text))
}

fn md_body(bytes: &[u8]) -> Result<String, AppError> {
    let text = decode_text(bytes);
    // 启用 GFM 表格等扩展：默认 Parser::new 不解析 markdown 表格（4x4 矩阵测试实测）
    let mut opts = pulldown_cmark::Options::empty();
    opts.insert(pulldown_cmark::Options::ENABLE_TABLES);
    opts.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    opts.insert(pulldown_cmark::Options::ENABLE_TASKLISTS);
    let parser = pulldown_cmark::Parser::new_ext(&text, opts);
    let mut out = String::new();
    pulldown_cmark::html::push_html(&mut out, parser);
    Ok(format!("<div class=\"doc-md\">{}</div>", out))
}

// ---------- PDF：pdfium 逐页渲染（保真）→ 降级文本提取 ----------

fn pdf_body(bytes: &[u8]) -> Result<String, AppError> {
    match pdfium_render_pages(bytes) {
        Ok(html) if !html.trim().is_empty() => return Ok(html),
        Ok(_) => {}
        Err(_) => { /* 渲染失败 → 降级 */ }
    }
    // 降级：文本提取
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| AppError::BundleError(format!("PDF 解析失败: {}", e)))?;
    if text.trim().is_empty() {
        return Err(AppError::BundleError(
            "PDF 中未提取到文本（可能为扫描件/纯图片 PDF，且未找到 pdfium.dll）".into(),
        ));
    }
    Ok(format!(
        "<p class=\"doc-note\">⚠ 未找到 pdfium.dll，已降级为文本提取，排版与图片不保留。</p><pre class=\"doc-plain\">{}</pre>",
        escape_html(text.trim())
    ))
}

/// pdfium 逐页渲染为 PNG 图片
fn pdfium_render_pages(bytes: &[u8]) -> Result<String, AppError> {
    use pdfium_render::prelude::*;
    let dll = locate_pdfium_dll()
        .ok_or_else(|| AppError::BundleError("未找到 pdfium.dll".into()))?;
    let bindings = Pdfium::bind_to_library(&dll)
        .map_err(|e| AppError::BundleError(format!("加载 pdfium 失败: {}", e)))?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|e| AppError::BundleError(format!("PDF 打开失败: {}", e)))?;
    let mut out = String::new();
    for (i, page) in doc.pages().iter().enumerate() {
        let render_cfg = PdfRenderConfig::new().set_target_width(1400);
        let bitmap = page
            .render_with_config(&render_cfg)
            .map_err(|e| AppError::BundleError(format!("PDF 第 {} 页渲染失败: {}", i + 1, e)))?;
        let img = bitmap.as_image();
        let mut png_buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png_buf, image::ImageFormat::Png)
            .map_err(|e| AppError::BundleError(format!("PNG 编码失败: {}", e)))?;
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png_buf.get_ref());
        out.push_str(&format!(
            "<section class=\"doc-pdf-page\"><div class=\"doc-slide-no\">第 {} 页</div>\
             <img src=\"data:image/png;base64,{}\" alt=\"\"></section>",
            i + 1,
            b64
        ));
    }
    Ok(out)
}

/// pdfium.dll 查找顺序：exe 同级 resources/ → exe 同级 → cwd/resources/ → CARGO_MANIFEST_DIR/resources/
fn locate_pdfium_dll() -> Option<std::path::PathBuf> {
    let mut cands: Vec<std::path::PathBuf> = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("resources").join("pdfium.dll"));
            cands.push(dir.join("pdfium.dll"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        cands.push(cwd.join("resources").join("pdfium.dll"));
    }
    cands.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("pdfium.dll"),
    );
    cands.into_iter().find(|p| p.exists())
}

// ---------- DOCX：富化版 ----------

#[derive(Default, Clone)]
struct RunStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    color: Option<String>,
    size_pt: Option<f32>,
}

/// OOXML 关系文件（.rels）解析：Id → (Target, 是否外部)
#[derive(Clone)]
struct OoxmlRel {
    target: String,
    external: bool,
}

fn parse_ooxml_rels(bytes: &[u8], rels_path: &str) -> HashMap<String, OoxmlRel> {
    let mut map = HashMap::new();
    let Some(xml_bytes) = read_zip_entry_opt(bytes, rels_path) else {
        return map;
    };
    let xml = String::from_utf8_lossy(&xml_bytes);
    for tok in xml_tokens(&xml) {
        if let Tok::Open(name, attrs) | Tok::Self_(name, attrs) = tok {
            if name == "Relationship" {
                if let Some(id) = attrs_get(&attrs, "Id") {
                    let target = attrs_get(&attrs, "Target").unwrap_or_default();
                    let external = attrs_get(&attrs, "TargetMode")
                        .map(|m| m.eq_ignore_ascii_case("external"))
                        .unwrap_or(false);
                    map.insert(id, OoxmlRel { target, external });
                }
            }
        }
    }
    map
}

/// 以 base_dir 为基准解析 zip 内相对路径（处理 .. 与绝对路径）
fn resolve_zip_path(base_dir: &str, target: &str) -> String {
    if let Some(stripped) = target.strip_prefix('/') {
        return stripped.to_string();
    }
    let mut segs: Vec<String> = base_dir
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    for seg in target.split('/') {
        match seg {
            "." => {}
            ".." => {
                segs.pop();
            }
            s if !s.is_empty() => segs.push(s.to_string()),
            _ => {}
        }
    }
    segs.join("/")
}

/// 浏览器 <img> 可显示的图片扩展名
fn img_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        _ => None, // tiff/emf/wmf 等浏览器不支持 → 跳过
    }
}

/// 从 zip 读取图片并生成 data URI <img>；失败返回 None
fn zip_image_html(bytes: &[u8], zip_path: &str) -> Option<String> {
    let data = read_zip_entry_opt(bytes, zip_path)?;
    let mime = img_mime(zip_path)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
    Some(format!(
        "<img src=\"data:{};base64,{}\" alt=\"\" class=\"doc-img\">",
        mime, b64
    ))
}

fn run_html(text: &str, style: &RunStyle, href: Option<&str>) -> String {
    let mut inner = escape_html(text).replace('\n', "<br>");
    if style.underline {
        inner = format!("<u>{}</u>", inner);
    }
    if style.italic {
        inner = format!("<i>{}</i>", inner);
    }
    if style.bold {
        inner = format!("<b>{}</b>", inner);
    }
    let mut styles = vec![];
    if let Some(c) = &style.color {
        if c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            styles.push(format!("color:#{}", c));
        }
    }
    if let Some(pt) = style.size_pt {
        styles.push(format!("font-size:{:.0}pt", pt));
    }
    if !styles.is_empty() {
        inner = format!("<span style=\"{}\">{}</span>", styles.join(";"), inner);
    }
    if let Some(h) = href {
        inner = format!(
            "<a href=\"{}\" target=\"_blank\" rel=\"noreferrer\">{}</a>",
            escape_attr(h),
            inner
        );
    }
    inner
}

fn para_html_docx(runs: &[String], align: &str, is_li: bool, style: &str) -> String {
    let body: String = runs.join("");
    let cls = match align {
        "center" => Some("doc-center"),
        "right" => Some("doc-right"),
        "both" | "justify" => Some("doc-justify"),
        _ => None,
    };
    let attr = cls.map(|c| format!(" class=\"{}\"", c)).unwrap_or_default();
    if let Some(level) = heading_level(style) {
        return format!("<h{level}>{}</h{level}>", body, level = level);
    }
    if is_li {
        return format!("<p class=\"doc-li{}\">{}</p>", cls.map(|c| format!(" {}", c)).unwrap_or_default(), body);
    }
    if body.is_empty() {
        return String::new();
    }
    format!("<p{}>{}</p>", attr, body)
}

fn table_html(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from("<table class=\"doc-table\">");
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            let body = if cell.is_empty() { "&nbsp;".to_string() } else { cell.clone() };
            out.push_str(&format!("<td>{}</td>", body));
        }
        out.push_str("</tr>");
    }
    out.push_str("</table>");
    out
}

fn docx_body(bytes: &[u8]) -> Result<String, AppError> {
    let xml = read_zip_entry(bytes, "word/document.xml")?;
    let xml = String::from_utf8_lossy(&xml);
    let rels = parse_ooxml_rels(bytes, "word/_rels/document.xml.rels");
    let toks = xml_tokens(&xml);

    let mut out = String::new();
    // 段落状态
    let mut para_runs: Vec<String> = vec![];
    let mut para_align = String::new();
    let mut para_is_li = false;
    let mut para_style = String::new();
    // run 状态
    let mut run_style = RunStyle::default();
    let mut run_text = String::new();
    let mut in_wt = false;
    let mut href: Option<String> = None;
    // 表格状态
    let mut in_tbl = 0usize;
    let mut in_tc = false;
    let mut cur_cell: Vec<String> = vec![];
    let mut cur_row: Vec<String> = vec![];
    let mut table_rows: Vec<Vec<String>> = vec![];

    macro_rules! flush_run {
        () => {
            if !run_text.is_empty() {
                para_runs.push(run_html(
                    &run_text,
                    &run_style,
                    href.as_deref(),
                ));
                run_text.clear();
            }
        };
    }
    macro_rules! flush_par {
        () => {{
            flush_run!();
            let html = para_html_docx(&para_runs, &para_align, para_is_li, &para_style);
            para_runs.clear();
            if in_tc {
                cur_cell.push(html);
            } else if !html.is_empty() {
                out.push_str(&html);
            }
            para_align.clear();
            para_is_li = false;
            para_style.clear();
        }};
    }
    macro_rules! apply_bool_style {
        ($attrs:expr, $field:expr) => {
            match attrs_get($attrs, "w:val").as_deref() {
                Some("0") | Some("false") | Some("off") => $field = false,
                _ => $field = true,
            }
        };
    }

    for tok in &toks {
        match tok {
            Tok::Open(name, attrs) | Tok::Self_(name, attrs) => match name.as_str() {
                "w:p" => {
                    para_runs.clear();
                    para_align.clear();
                    para_is_li = false;
                    para_style.clear();
                    if name == "w:p" {
                        run_style = RunStyle::default();
                        run_text.clear();
                        in_wt = false;
                        href = None;
                    }
                }
                "w:pStyle" => para_style = attrs_get(attrs, "w:val").unwrap_or_default(),
                "w:jc" => para_align = attrs_get(attrs, "w:val").unwrap_or_default(),
                "w:numPr" => para_is_li = true,
                "w:r" if name == "w:r" => {
                    flush_run!();
                    run_style = RunStyle::default();
                }
                "w:b" => apply_bool_style!(attrs, run_style.bold),
                "w:i" => apply_bool_style!(attrs, run_style.italic),
                "w:u" => apply_bool_style!(attrs, run_style.underline),
                "w:color" => run_style.color = attrs_get(attrs, "w:val"),
                "w:sz" => {
                    run_style.size_pt = attrs_get(attrs, "w:val")
                        .and_then(|v| v.parse::<f32>().ok())
                        .map(|h| h / 2.0)
                }
                "w:t" if name == "w:t" => in_wt = true,
                "w:hyperlink" if name == "w:hyperlink" => {
                    href = attrs_get(attrs, "r:id").and_then(|rid| {
                        rels.get(&rid).filter(|r| r.external).map(|r| r.target.clone())
                    });
                }
                "w:tbl" => {
                    in_tbl += 1;
                    if in_tbl == 1 {
                        table_rows.clear();
                    }
                }
                "w:tr" => cur_row.clear(),
                "w:tc" => {
                    in_tc = true;
                    cur_cell.clear();
                }
                "w:br" => run_text.push('\n'),
                "w:tab" => run_text.push('\t'),
                "a:blip" => {
                    if let Some(rid) = attrs_get(attrs, "r:embed").or_else(|| attrs_get(attrs, "r:link")) {
                        if let Some(rel) = rels.get(&rid).filter(|r| !r.external) {
                            let zip_path = resolve_zip_path("word", &rel.target);
                            if let Some(img) = zip_image_html(bytes, &zip_path) {
                                flush_run!();
                                para_runs.push(img);
                            }
                        }
                    }
                }
                "v:imagedata" => {
                    if let Some(rid) = attrs_get(attrs, "r:id") {
                        if let Some(rel) = rels.get(&rid).filter(|r| !r.external) {
                            let zip_path = resolve_zip_path("word", &rel.target);
                            if let Some(img) = zip_image_html(bytes, &zip_path) {
                                flush_run!();
                                para_runs.push(img);
                            }
                        }
                    }
                }
                _ => {}
            },
            Tok::Txt(t) => {
                if in_wt {
                    run_text.push_str(t);
                }
            }
            Tok::Close(name) => match name.as_str() {
                "w:t" => in_wt = false,
                "w:r" => flush_run!(),
                "w:hyperlink" => {
                    flush_run!();
                    href = None;
                }
                "w:p" => flush_par!(),
                "w:tc" => {
                    in_tc = false;
                    cur_row.push(cur_cell.join(""));
                    cur_cell.clear();
                }
                "w:tr" => table_rows.push(std::mem::take(&mut cur_row)),
                "w:tbl" => {
                    in_tbl = in_tbl.saturating_sub(1);
                    if in_tbl == 0 {
                        out.push_str(&table_html(&table_rows));
                        table_rows.clear();
                    }
                }
                _ => {}
            },
        }
    }
    if out.trim().is_empty() {
        return Err(AppError::BundleError("docx 中未提取到内容".into()));
    }
    Ok(out)
}

// ---------- PPTX：富化版（形状 y 排序 + 图片 + 表格） ----------

fn pptx_body(bytes: &[u8]) -> Result<String, AppError> {
    let slides = zip_slide_list(bytes, "ppt/slides/slide", ".xml")?;
    if slides.is_empty() {
        return Err(AppError::BundleError("pptx 中未找到幻灯片".into()));
    }
    let mut out = String::new();
    for (idx, name) in slides.iter().enumerate() {
        let xml_bytes = read_zip_entry(bytes, name)?;
        let xml = String::from_utf8_lossy(&xml_bytes);
        // 该 slide 的关系文件（name 目录前缀已含尾部斜杠，直接拼 _rels）
        let rels_path = {
            let stem = name.rsplit('/').next().unwrap_or(name);
            format!("{}_rels/{}.rels", &name[..name.len() - stem.len()], stem)
        };
        let rels = parse_ooxml_rels(bytes, &rels_path);
        let toks = xml_tokens(&xml);
        let mut shapes = split_pptx_shapes(&toks);
        shapes.sort_by_key(|(y, _)| *y); // 稳定排序：同 y 保持 XML 顺序
        let mut body = String::new();
        for (_, chunk) in &shapes {
            body.push_str(&render_pptx_chunk(chunk, bytes, &rels));
        }
        out.push_str(&format!(
            "<section class=\"doc-slide\"><div class=\"doc-slide-no\">第 {} 页</div>{}</section>",
            idx + 1,
            body
        ));
    }
    Ok(out)
}

/// 把 slide tokens 切分为顶层形状块：(y 坐标, chunk)
fn split_pptx_shapes(toks: &[Tok]) -> Vec<(i64, &[Tok])> {
    const SHAPE_TAGS: [&str; 3] = ["p:sp", "p:pic", "p:graphicFrame"];
    let mut out: Vec<(i64, &[Tok])> = vec![];
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    let mut y: Option<i64> = None;
    for (i, tok) in toks.iter().enumerate() {
        match tok {
            Tok::Open(name, _) if SHAPE_TAGS.contains(&name.as_str()) => {
                if depth == 0 {
                    start = Some(i);
                    y = None;
                }
                depth += 1;
            }
            Tok::Close(name) if SHAPE_TAGS.contains(&name.as_str()) => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            out.push((y.unwrap_or(i64::MAX), &toks[s..=i]));
                        }
                        start = None;
                    }
                }
            }
            Tok::Self_(name, attrs) | Tok::Open(name, attrs)
                if depth >= 1 && name == "a:off" && y.is_none() =>
            {
                y = attrs_get(attrs, "y").and_then(|v| v.parse().ok());
            }
            _ => {}
        }
    }
    out
}

/// 渲染单个形状块：文本框（标题/段落/对齐）、图片、表格
fn render_pptx_chunk(chunk: &[Tok], zip_bytes: &[u8], rels: &HashMap<String, OoxmlRel>) -> String {
    let Some(Tok::Open(first, _)) = chunk.first() else {
        return String::new();
    };
    match first.as_str() {
        "p:pic" => {
            for tok in chunk {
                if let Tok::Open(name, attrs) | Tok::Self_(name, attrs) = tok {
                    if name == "a:blip" {
                        if let Some(rid) = attrs_get(attrs, "r:embed") {
                            if let Some(rel) = rels.get(&rid).filter(|r| !r.external) {
                                let zip_path = resolve_zip_path("ppt/slides", &rel.target);
                                if let Some(img) = zip_image_html(zip_bytes, &zip_path) {
                                    return format!("<div class=\"doc-img-box\">{}</div>", img);
                                }
                            }
                        }
                    }
                }
            }
            String::new()
        }
        "p:graphicFrame" => {
            // a:tbl 表格：a:tr 行 / a:tc 单元格 / a:p 段落（文本 a:t）
            let mut rows: Vec<Vec<String>> = vec![];
            let mut row: Vec<String> = vec![];
            let mut cell: Vec<String> = vec![];
            let mut para = String::new();
            let mut in_at = false;
            for tok in chunk {
                match tok {
                    Tok::Open(name, _) if name == "a:tbl" => {
                        rows.clear();
                    }
                    Tok::Open(name, _) if name == "a:tr" => row.clear(),
                    Tok::Open(name, _) if name == "a:tc" => cell.clear(),
                    Tok::Open(name, _) if name == "a:t" => in_at = true,
                    Tok::Txt(t) if in_at => para.push_str(t),
                    Tok::Close(name) => match name.as_str() {
                        "a:t" => in_at = false,
                        "a:p" => {
                            let t = para.trim().to_string();
                            if !t.is_empty() {
                                cell.push(format!("<p>{}</p>", escape_html(&t)));
                            }
                            para.clear();
                        }
                        "a:tc" => row.push(cell.join("")),
                        "a:tr" => rows.push(std::mem::take(&mut row)),
                        _ => {}
                    },
                    _ => {}
                }
            }
            table_html(&rows)
        }
        _ => {
            // 文本形状 p:sp（也覆盖罕见包装）
            let mut title = false;
            let mut paras: Vec<String> = vec![];
            let mut para = String::new();
            let mut in_at = false;
            let mut align = String::new();
            for tok in chunk {
                match tok {
                    Tok::Open(name, attrs) | Tok::Self_(name, attrs) if name == "p:ph" => {
                        let ty = attrs_get(attrs, "type").unwrap_or_default();
                        if ty == "title" || ty == "ctrTitle" {
                            title = true;
                        }
                    }
                    Tok::Open(name, _) if name == "a:p" => para.clear(),
                    Tok::Self_(name, attrs) if name == "a:algn" => {
                        align = attrs_get(attrs, "val").unwrap_or_default();
                    }
                    Tok::Open(name, _) if name == "a:t" => in_at = true,
                    Tok::Close(name) if name == "a:t" => in_at = false,
                    Tok::Txt(t) if in_at => para.push_str(t),
                    Tok::Close(name) if name == "a:p" => {
                        let t = para.trim().to_string();
                        if !t.is_empty() {
                            let cls = match align.as_str() {
                                "ctr" => " doc-center",
                                "r" => " doc-right",
                                "just" | "dist" => " doc-justify",
                                _ => "",
                            };
                            let esc = escape_html(&t);
                            if title {
                                paras.push(format!("<h2>{}</h2>", esc));
                            } else if cls.is_empty() {
                                paras.push(format!("<p>{}</p>", esc));
                            } else {
                                paras.push(format!("<p class=\"{}\">{}</p>", cls.trim(), esc));
                            }
                        }
                        align.clear();
                        para.clear();
                    }
                    _ => {}
                }
            }
            paras.join("")
        }
    }
}

// ---------- ODF（odt / odp）：富化版 ----------

#[derive(Default, Clone)]
struct OdfStyle {
    align: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// 解析 content.xml 中 office:automatic-styles 的段落/文字样式
fn parse_odf_styles(xml: &str) -> HashMap<String, OdfStyle> {
    let toks = xml_tokens(xml);
    let mut map = HashMap::new();
    let mut cur_name: Option<String> = None;
    let mut st = OdfStyle::default();
    for tok in &toks {
        match tok {
            Tok::Open(name, attrs) if name == "style:style" => {
                cur_name = attrs_get(attrs, "style:name");
                st = OdfStyle::default();
            }
            Tok::Self_(name, attrs) if name == "style:style" => {
                if let Some(n) = attrs_get(attrs, "style:name") {
                    map.insert(n, OdfStyle::default());
                }
                cur_name = None;
            }
            Tok::Self_(name, attrs) if name == "style:paragraph-properties" => {
                st.align = attrs_get(attrs, "fo:text-align");
            }
            Tok::Self_(name, attrs) if name == "style:text-properties" => {
                if let Some(w) = attrs_get(attrs, "fo:font-weight") {
                    if matches!(w.as_str(), "bold" | "600" | "700" | "800" | "900") {
                        st.bold = true;
                    }
                }
                if let Some(s) = attrs_get(attrs, "fo:font-style") {
                    if s == "italic" {
                        st.italic = true;
                    }
                }
                if let Some(u) = attrs_get(attrs, "fo:text-underline-style") {
                    if u != "none" {
                        st.underline = true;
                    }
                }
            }
            Tok::Close(name) if name == "style:style" => {
                if let Some(n) = cur_name.take() {
                    map.insert(n, st.clone());
                }
            }
            _ => {}
        }
    }
    map
}

fn odf_text_wrap(text: &str, st: &OdfStyle) -> String {
    let mut inner = escape_html(text);
    if st.underline {
        inner = format!("<u>{}</u>", inner);
    }
    if st.italic {
        inner = format!("<i>{}</i>", inner);
    }
    if st.bold {
        inner = format!("<b>{}</b>", inner);
    }
    inner
}

fn odf_para_html(pieces: &[String], level: u8, is_h: bool, align: Option<&str>) -> String {
    let body: String = pieces.join("");
    if body.is_empty() {
        return String::new();
    }
    let cls = match align {
        Some("center") => " doc-center",
        Some("end") => " doc-right",
        Some("justify") => " doc-justify",
        _ => "",
    };
    if is_h {
        let l = level.clamp(1, 6);
        format!("<h{l}>{}</h{l}>", body)
    } else if cls.is_empty() {
        format!("<p>{}</p>", body)
    } else {
        format!("<p class=\"{}\">{}</p>", cls.trim(), body)
    }
}

/// ODF 图片：优先 zip 内 href，其次 office:binary-data（base64）
fn odf_image_html(
    zip_bytes: &[u8],
    attrs: &str,
    binary_data: Option<&str>,
) -> Option<String> {
    if let Some(href) = attrs_get(attrs, "xlink:href") {
        if !href.is_empty() && !href.starts_with('#') && !href.starts_with("http") {
            let zip_path = resolve_zip_path("", &href);
            if let Some(img) = zip_image_html(zip_bytes, &zip_path) {
                return Some(img);
            }
        }
        return None;
    }
    if let Some(b64) = binary_data {
        let cleaned: String = b64.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        if let Ok(data) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, cleaned.as_bytes()) {
            if let Some(mime) = img_mime("x.png") {
                return Some(format!(
                    "<img src=\"data:{};base64,{}\" alt=\"\" class=\"doc-img\">",
                    mime,
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data)
                ));
            }
        }
    }
    None
}

/// ODF 段落内部遍历（span 样式 / 链接 / 图片 / 换行），产出 html 片段列表
fn odf_collect_inline(
    toks: &[Tok],
    start: usize,
    end: usize,
    styles: &HashMap<String, OdfStyle>,
    zip_bytes: &[u8],
) -> Vec<String> {
    let mut pieces: Vec<String> = vec![];
    let mut span_stack: Vec<OdfStyle> = vec![];
    let mut href: Option<String> = None;
    let mut binary: Option<String> = None;
    let mut in_binary = false;
    if toks.is_empty() {
        return pieces;
    }
    for tok in &toks[start..=end.min(toks.len() - 1)] {
        match tok {
            Tok::Open(name, attrs) | Tok::Self_(name, attrs) => match name.as_str() {
                "text:span" | "style:text-style" => {
                    let st = attrs_get(attrs, "text:style-name")
                        .and_then(|n| styles.get(&n).cloned())
                        .unwrap_or_default();
                    span_stack.push(st);
                }
                "text:a" => {
                    href = attrs_get(attrs, "xlink:href").filter(|h| !h.is_empty());
                }
                "draw:image" => {
                    if let Some(img) = odf_image_html(zip_bytes, attrs, binary.as_deref()) {
                        pieces.push(img);
                    }
                    binary = None;
                }
                "office:binary-data" => {
                    in_binary = true;
                    binary = Some(String::new());
                }
                "text:line-break" => pieces.push("<br>".to_string()),
                "text:tab" | "text:s" => pieces.push(" ".to_string()),
                _ => {}
            },
            Tok::Txt(t) => {
                if in_binary {
                    if let Some(b) = binary.as_mut() {
                        b.push_str(t);
                    }
                } else {
                    let merged = span_stack.iter().fold(OdfStyle::default(), |acc, s| OdfStyle {
                        align: acc.align.or(s.align.clone()),
                        bold: acc.bold || s.bold,
                        italic: acc.italic || s.italic,
                        underline: acc.underline || s.underline,
                    });
                    let mut piece = odf_text_wrap(t, &merged);
                    if let Some(h) = &href {
                        piece = format!(
                            "<a href=\"{}\" target=\"_blank\" rel=\"noreferrer\">{}</a>",
                            escape_attr(h),
                            piece
                        );
                    }
                    pieces.push(piece);
                }
            }
            Tok::Close(name) => match name.as_str() {
                "text:span" | "style:text-style" => {
                    span_stack.pop();
                }
                "text:a" => href = None,
                "office:binary-data" => in_binary = false,
                _ => {}
            },
        }
    }
    pieces
}

/// ODF 段落对齐：从段落 text:style-name 解析
fn para_align_of(attrs: &str, styles: &HashMap<String, OdfStyle>) -> Option<String> {
    attrs_get(attrs, "text:style-name")
        .and_then(|n| styles.get(&n).cloned())
        .and_then(|s| s.align)
}

fn odt_body(bytes: &[u8]) -> Result<String, AppError> {
    let xml_bytes = read_zip_entry(bytes, "content.xml")?;
    let xml = String::from_utf8_lossy(&xml_bytes);
    let styles = parse_odf_styles(&xml);
    let toks = xml_tokens(&xml);

    let mut out = String::new();
    // 表格状态（0.8.x 补齐：此前 table:table 不被识别，结构拍平、格内图片丢失）
    let mut in_tbl = false;
    let mut tbl_rows: Vec<Vec<String>> = vec![];
    let mut tbl_row: Vec<String> = vec![];
    let mut tbl_cell = String::new();

    let mut i = 0usize;
    while i < toks.len() {
        if let Tok::Open(name, attrs) | Tok::Self_(name, attrs) = &toks[i] {
            match name.as_str() {
                "table:table" if !in_tbl => {
                    in_tbl = true;
                    tbl_rows.clear();
                    i += 1;
                    continue;
                }
                "draw:image" if in_tbl => {
                    // 表格格里未包在 text:p 中的图片（draw:frame 直接子元素，
                    // 常为自闭合 <draw:image/>），text:p 处理器不会扫到这里
                    if let Some(img) = odf_image_html(bytes, attrs, None) {
                        tbl_cell.push_str(&format!("<div class=\"doc-img-box\">{}</div>", img));
                    }
                    i += 1;
                    continue;
                }
                "text:h" | "text:p" => {
                    let is_h = name == "text:h";
                    let close = find_matching_close(&toks, i, name);
                    let pieces = odf_collect_inline(&toks, i, close, &styles, bytes);
                    let level = if is_h {
                        attrs_get(attrs, "text:outline-level")
                            .and_then(|v| v.parse::<u8>().ok())
                            .unwrap_or(1)
                            .clamp(1, 6)
                    } else {
                        1
                    };
                    let align = para_align_of(attrs, &styles);
                    let html = odf_para_html(&pieces, level, is_h, align.as_deref());
                    if in_tbl {
                        tbl_cell.push_str(&html);
                    } else {
                        out.push_str(&html);
                    }
                    i = close + 1;
                    continue;
                }
                _ => {}
            }
        } else if let Tok::Close(name) = &toks[i] {
            match name.as_str() {
                "table:table-cell" if in_tbl => {
                    tbl_row.push(std::mem::take(&mut tbl_cell));
                }
                "table:table-row" if in_tbl => {
                    tbl_rows.push(std::mem::take(&mut tbl_row));
                }
                "table:table" if in_tbl => {
                    in_tbl = false;
                    out.push_str(&table_html(&tbl_rows));
                    tbl_rows.clear();
                }
                _ => {}
            }
        }
        i += 1;
    }
    if out.trim().is_empty() {
        return Err(AppError::BundleError("odt 中未提取到内容".into()));
    }
    Ok(out)
}

/// 找到 toks[start] 对应的匹配 Close 索引（同名标签深度计数）
fn find_matching_close(toks: &[Tok], start: usize, name: &str) -> usize {
    let mut depth = 0usize;
    for (i, tok) in toks.iter().enumerate().skip(start) {
        match tok {
            Tok::Open(n, _) if n == name => depth += 1,
            Tok::Close(n) if n == name => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    toks.len().saturating_sub(1)
}

fn odp_body(bytes: &[u8]) -> Result<String, AppError> {
    let xml_bytes = read_zip_entry(bytes, "content.xml")?;
    let xml = String::from_utf8_lossy(&xml_bytes);
    let styles = parse_odf_styles(&xml);
    let toks = xml_tokens(&xml);

    let mut out = String::new();
    let mut slide_no = 0usize;
    let mut slide_open = false;
    let mut frames: Vec<(i64, String)> = vec![];
    let mut cur_frame_y: Option<i64> = None;
    let mut cur_frame_html = String::new();
    let mut in_frame = false;
    // 表格状态（frame 内）
    let mut in_tbl = false;
    let mut tbl_rows: Vec<Vec<String>> = vec![];
    let mut tbl_row: Vec<String> = vec![];
    let mut tbl_cell = String::new();

    macro_rules! flush_slide {
        () => {{
            frames.sort_by_key(|(y, _)| *y); // 稳定排序：同 y 保持 XML 顺序
            for (_, html) in &frames {
                out.push_str(html);
            }
            out.push_str("</section>");
            frames.clear();
            slide_open = false;
        }};
    }

    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i] {
            Tok::Open(name, attrs) | Tok::Self_(name, attrs) => match name.as_str() {
                "draw:page" if *name == "draw:page" => {
                    if slide_open {
                        flush_slide!();
                    }
                    slide_no += 1;
                    slide_open = true;
                    frames.clear();
                    out.push_str(&format!(
                        "<section class=\"doc-slide\"><div class=\"doc-slide-no\">第 {} 页</div>",
                        slide_no
                    ));
                    i += 1;
                }
                "draw:frame" if *name == "draw:frame" && !in_frame => {
                    in_frame = true;
                    cur_frame_y = attrs_get(attrs, "svg:y").and_then(|v| v.parse().ok());
                    cur_frame_html.clear();
                    i += 1;
                }
                "draw:image" if in_frame => {
                    if let Some(img) = odf_image_html(bytes, attrs, None) {
                        cur_frame_html.push_str(&format!("<div class=\"doc-img-box\">{}</div>", img));
                    }
                    i += 1;
                }
                "table:table" if in_frame => {
                    in_tbl = true;
                    tbl_rows.clear();
                    i += 1;
                }
                "table:table-row" if in_tbl => {
                    tbl_row.clear();
                    i += 1;
                }
                "table:table-cell" if in_tbl => {
                    tbl_cell.clear();
                    i += 1;
                }
                "text:h" if slide_open => {
                    let close = find_matching_close(&toks, i, "text:h");
                    let pieces = odf_collect_inline(&toks, i, close, &styles, bytes);
                    let level = attrs_get(attrs, "text:outline-level")
                        .and_then(|v| v.parse::<u8>().ok())
                        .unwrap_or(1)
                        .clamp(1, 6);
                    let align = para_align_of(attrs, &styles);
                    let html = odf_para_html(&pieces, level, true, align.as_deref());
                    if in_tbl {
                        tbl_cell.push_str(&html);
                    } else if in_frame {
                        cur_frame_html.push_str(&html);
                    } else {
                        // 页面级直接段落（无 frame 包装）
                        out.push_str(&html);
                    }
                    i = close + 1;
                }
                "text:p" if slide_open => {
                    let close = find_matching_close(&toks, i, "text:p");
                    let pieces = odf_collect_inline(&toks, i, close, &styles, bytes);
                    let align = para_align_of(attrs, &styles);
                    let html = odf_para_html(&pieces, 1, false, align.as_deref());
                    if in_tbl {
                        tbl_cell.push_str(&html);
                    } else if in_frame {
                        cur_frame_html.push_str(&html);
                    } else {
                        out.push_str(&html);
                    }
                    i = close + 1;
                }
                _ => {
                    i += 1;
                }
            },
            Tok::Close(name) => match name.as_str() {
                "table:table-cell" if in_tbl => {
                    tbl_row.push(std::mem::take(&mut tbl_cell));
                    i += 1;
                }
                "table:table-row" if in_tbl => {
                    tbl_rows.push(std::mem::take(&mut tbl_row));
                    i += 1;
                }
                "table:table" if in_tbl => {
                    in_tbl = false;
                    cur_frame_html.push_str(&table_html(&tbl_rows));
                    tbl_rows.clear();
                    i += 1;
                }
                "draw:frame" if in_frame => {
                    in_frame = false;
                    frames.push((cur_frame_y.unwrap_or(i64::MAX), std::mem::take(&mut cur_frame_html)));
                    i += 1;
                }
                "draw:page" => {
                    if slide_open {
                        flush_slide!();
                    }
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            },
            Tok::Txt(_) => {
                i += 1;
            }
        }
    }
    if slide_open {
        flush_slide!();
    }
    if out.trim().is_empty() {
        return Err(AppError::BundleError("odp 中未提取到内容".into()));
    }
    Ok(out)
}

// ---------- RTF：富化版 ----------

/// RTF \ansicpg 代码页 → encoding_rs 编码（0.8.4 扩展：此前仅 936/54936→GBK；0.8.5 增补 KOI8/IBM866）
fn codepage_encoding(num: i64) -> Option<&'static encoding_rs::Encoding> {
    Some(match num {
        932 => encoding_rs::SHIFT_JIS,
        936 => encoding_rs::GBK,
        54936 => encoding_rs::GB18030,
        949 => encoding_rs::EUC_KR,
        950 => encoding_rs::BIG5,
        874 => encoding_rs::WINDOWS_874,
        866 => encoding_rs::IBM866,
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1252 => encoding_rs::WINDOWS_1252,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        20866 => encoding_rs::KOI8_R,
        21866 => encoding_rs::KOI8_U,
        _ => return None,
    })
}

#[derive(Default, Clone)]
struct RtfRun {
    text: String,
    img_html: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    size_pt: Option<f32>,
}

struct RtfPara {
    align: u8, // 0=l 1=c 2=r 3=j
    runs: Vec<RtfRun>,
}

#[derive(Clone)]
struct RtfGroup {
    skip: bool,
    ucskip: usize,
    bold: bool,
    italic: bool,
    underline: bool,
    size: Option<f32>,
    pict: bool,
    pict_fmt: u8, // 0 无 1 png 2 jpeg 3 其他
}

fn rtf_body(bytes: &[u8]) -> Result<String, AppError> {
    let paras = rtf_parse(bytes);
    let has_content = paras.iter().any(|p| {
        p.runs.iter().any(|r| {
            r.img_html.is_some() || !r.text.trim().is_empty()
        })
    });
    if !has_content {
        return Err(AppError::BundleError("RTF 中未提取到内容".into()));
    }
    let mut out = String::new();
    for p in &paras {
        let mut body = String::new();
        for r in &p.runs {
            if let Some(img) = &r.img_html {
                body.push_str(&format!("<span class=\"doc-img-box\">{}</span>", img));
                continue;
            }
            let t = r.text.trim();
            if t.is_empty() && r.text.trim().is_empty() && !r.text.contains('\n') {
                continue;
            }
            let mut inner = escape_html(&r.text).replace('\n', "<br>");
            if r.underline {
                inner = format!("<u>{}</u>", inner);
            }
            if r.italic {
                inner = format!("<i>{}</i>", inner);
            }
            if r.bold {
                inner = format!("<b>{}</b>", inner);
            }
            if let Some(pt) = r.size_pt {
                inner = format!("<span style=\"font-size:{:.0}pt\">{}</span>", pt, inner);
            }
            body.push_str(&inner);
        }
        if body.trim().is_empty() {
            continue;
        }
        let cls = match p.align {
            1 => " class=\"doc-center\"",
            2 => " class=\"doc-right\"",
            3 => " class=\"doc-justify\"",
            _ => "",
        };
        out.push_str(&format!("<p{}>{}</p>", cls, body));
    }
    if out.trim().is_empty() {
        return Err(AppError::BundleError("RTF 中未提取到文本".into()));
    }
    Ok(out)
}

fn hex_decode(s: &str) -> Vec<u8> {
    let filtered: Vec<char> = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    filtered
        .chunks(2)
        .filter(|c| c.len() == 2)
        .filter_map(|c| u8::from_str_radix(&format!("{}{}", c[0], c[1]), 16).ok())
        .collect()
}

fn rtf_parse(bytes: &[u8]) -> Vec<RtfPara> {
    let mut paras: Vec<RtfPara> = vec![];
    let mut cur = RtfPara { align: 0, runs: vec![] };
    let mut cur_style = RtfGroup {
        skip: false,
        ucskip: 1,
        bold: false,
        italic: false,
        underline: false,
        size: None,
        pict: false,
        pict_fmt: 0,
    };
    let mut cur_text = String::new();
    let mut pending_ansi: Vec<u8> = vec![];
    let mut codepage_enc: Option<&'static encoding_rs::Encoding> = None;
    let mut stack: Vec<RtfGroup> = vec![];
    let mut pict_hex = String::new();
    let mut pict_bin: Vec<u8> = vec![];
    let mut pict_pos = 0usize;

    macro_rules! flush_ansi {
        () => {
            if !pending_ansi.is_empty() {
                let enc = codepage_enc.unwrap_or(encoding_rs::WINDOWS_1252);
                let (text, _, _) = enc.decode(&pending_ansi);
                cur_text.push_str(&text);
                pending_ansi.clear();
            }
        };
    }
    macro_rules! flush_run {
        () => {{
            flush_ansi!();
            if !cur_text.is_empty() {
                cur.runs.push(RtfRun {
                    text: std::mem::take(&mut cur_text),
                    img_html: None,
                    bold: cur_style.bold,
                    italic: cur_style.italic,
                    underline: cur_style.underline,
                    size_pt: cur_style.size,
                });
            }
        }};
    }
    macro_rules! push_para {
        () => {{
            flush_run!();
            if !cur.runs.is_empty() {
                paras.push(std::mem::replace(&mut cur, RtfPara { align: 0, runs: vec![] }));
            }
        }};
    }

    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        let b = bytes[i];
        let cur_skip = stack.last().map(|g| g.skip).unwrap_or(false);
        let cur_pict = stack.last().map(|g| g.pict).unwrap_or(false);
        let in_pict_data = cur_pict && stack.len() == pict_pos;
        match b {
            b'{' => {
                flush_run!();
                let parent = stack.last().cloned().unwrap_or(cur_style.clone());
                stack.push(parent);
                i += 1;
            }
            b'}' => {
                flush_run!();
                if let Some(popped) = stack.pop() {
                    // 结束 \pict 数据组
                    if popped.pict && stack.len() == pict_pos - 1 {
                        let mut data = hex_decode(&pict_hex);
                        data.extend_from_slice(&pict_bin);
                        let mime = match popped.pict_fmt {
                            1 => Some("image/png"),
                            2 => Some("image/jpeg"),
                            _ => None,
                        };
                        if let Some(mime) = mime {
                            if !data.is_empty() {
                                let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
                                cur.runs.push(RtfRun {
                                    text: String::new(),
                                    img_html: Some(format!(
                                        "<img src=\"data:{};base64,{}\" alt=\"\" class=\"doc-img\">",
                                        mime, b64
                                    )),
                                    bold: false,
                                    italic: false,
                                    underline: false,
                                    size_pt: None,
                                });
                            }
                        }
                        pict_hex.clear();
                        pict_bin.clear();
                        cur_style.bold = popped.bold;
                        cur_style.italic = popped.italic;
                        cur_style.underline = popped.underline;
                        cur_style.size = popped.size;
                        cur_style.ucskip = popped.ucskip;
                        cur_style.skip = false;
                        cur_style.pict = false;
                        cur_style.pict_fmt = 0;
                        i += 1;
                        continue;
                    }
                    // 恢复父组样式
                    cur_style.bold = popped.bold;
                    cur_style.italic = popped.italic;
                    cur_style.underline = popped.underline;
                    cur_style.size = popped.size;
                    cur_style.ucskip = popped.ucskip;
                    cur_style.skip = stack.last().map(|g| g.skip).unwrap_or(false);
                    cur_style.pict = stack.last().map(|g| g.pict).unwrap_or(false);
                    cur_style.pict_fmt = stack.last().map(|g| g.pict_fmt).unwrap_or(0);
                }
                if stack.is_empty() {
                    break;
                }
                i += 1;
            }
            b'\\' if i + 1 < n => {
                let c = bytes[i + 1];
                if c == b'*' {
                    if let Some(g) = stack.last_mut() {
                        g.skip = true;
                    }
                    i += 2;
                    continue;
                }
                if c == b'\'' {
                    if i + 3 < n {
                        let h = (bytes[i + 2] as char).to_digit(16);
                        let l = (bytes[i + 3] as char).to_digit(16);
                        if let (Some(h), Some(l)) = (h, l) {
                            if in_pict_data {
                                pict_bin.push(((h << 4) | l) as u8);
                            } else if !cur_skip {
                                pending_ansi.push(((h << 4) | l) as u8);
                            }
                            i += 4;
                            continue;
                        }
                    }
                    i += 2;
                    continue;
                }
                if c.is_ascii_alphabetic() {
                    let mut j = i + 1;
                    while j < n && bytes[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    let word = std::str::from_utf8(&bytes[i + 1..j]).unwrap_or("");
                    let mut num: Option<i64> = None;
                    let mut k = j;
                    if k < n && (bytes[k] == b'-' || bytes[k].is_ascii_digit()) {
                        let s = k;
                        if bytes[k] == b'-' {
                            k += 1;
                        }
                        while k < n && bytes[k].is_ascii_digit() {
                            k += 1;
                        }
                        num = std::str::from_utf8(&bytes[s..k]).ok().and_then(|s| s.parse().ok());
                    }
                    if k < n && bytes[k] == b' ' {
                        k += 1;
                    }
                    match word {
                        "pngblip" | "jpegblip" | "emfblip" | "wmetafile" if cur_pict => {
                            flush_run!();
                            let fmt = match word {
                                "pngblip" => 1,
                                "jpegblip" => 2,
                                _ => 3,
                            };
                            // fmt 必须落到 \pict 所在组：若 \pngblip 写在 {\pngblip} 子组内
                            // （Word/WPS 真实文件常见形态），此前写在子组上，子组闭合即丢失，
                            // \pict 组收尾时 pict_fmt=0 导致图片被静默丢弃
                            if pict_pos > 0 && pict_pos <= stack.len() {
                                stack[pict_pos - 1].pict_fmt = fmt;
                            }
                            if let Some(g) = stack.last_mut() {
                                g.pict_fmt = fmt;
                            }
                        }
                        "bin" => {
                            let count = num.unwrap_or(0).max(0) as usize;
                            let end = (k + count).min(n);
                            if in_pict_data {
                                pict_bin.extend_from_slice(&bytes[k.min(n)..end]);
                            }
                            i = end;
                            continue;
                        }
                        "pict" => {
                            flush_run!();
                            if let Some(g) = stack.last_mut() {
                                g.pict = true;
                                g.skip = false;
                            }
                            pict_pos = stack.len();
                            pict_hex.clear();
                            pict_bin.clear();
                        }
                        _ if cur_pict && stack.len() >= pict_pos && stack.len() > pict_pos => {
                            // \pict 内嵌套组中的其他控制字：忽略
                        }
                        _ if in_pict_data => {
                            // \pict 数据区控制字（\picw 等）：忽略
                        }
                        "par" | "sect" | "page" if !cur_skip => {
                            flush_run!();
                            push_para!();
                            cur.align = 0;
                        }
                        "line" if !cur_skip => {
                            flush_ansi!();
                            cur_text.push('\n');
                        }
                        "tab" if !cur_skip => {
                            flush_ansi!();
                            cur_text.push('\t');
                        }
                        "b" if !cur_skip => {
                            flush_run!();
                            cur_style.bold = num != Some(0);
                        }
                        "i" if !cur_skip => {
                            flush_run!();
                            cur_style.italic = num != Some(0);
                        }
                        "ul" | "uld" | "uldb" | "uldash" if !cur_skip => {
                            flush_run!();
                            cur_style.underline = true;
                        }
                        "ulnone" if !cur_skip => {
                            flush_run!();
                            cur_style.underline = false;
                        }
                        "fs" if !cur_skip => {
                            flush_run!();
                            cur_style.size = num.map(|v| (v.max(0) as f32) / 2.0);
                        }
                        "qc" if !cur_skip => cur.align = 1,
                        "qr" if !cur_skip => cur.align = 2,
                        "qj" if !cur_skip => cur.align = 3,
                        "ql" if !cur_skip => cur.align = 0,
                        "uc" if !cur_skip => {
                            cur_style.ucskip = num.unwrap_or(1).max(0) as usize;
                            if let Some(g) = stack.last_mut() {
                                g.ucskip = cur_style.ucskip;
                            }
                        }
                        "u" if !cur_skip => {
                            if let Some(cp) = num {
                                let cp = if cp < 0 { cp + 65536 } else { cp };
                                if let Some(ch) = char::from_u32(cp as u32) {
                                    flush_ansi!();
                                    cur_text.push(ch);
                                }
                            }
                            let skip = stack.last().map(|g| g.ucskip).unwrap_or(1);
                            i = k + skip.min(n.saturating_sub(k));
                            continue;
                        }
                        "ansicpg" if !cur_skip => {
                            codepage_enc = codepage_encoding(num.unwrap_or(1252));
                        }
                        "fonttbl" | "colortbl" | "stylesheet" | "info" | "object"
                        | "themedata" | "datastore" | "listtable" | "listoverridetable"
                        | "rsidtbl" | "generator" | "xmlnstbl" | "latentstyles" | "wgrffmtfilter"
                            if !cur_skip && !cur_pict =>
                        {
                            flush_run!();
                            if let Some(g) = stack.last_mut() {
                                g.skip = true;
                            }
                        }
                        _ => {}
                    }
                    i = k;
                    continue;
                }
                // 转义符号 \{ \} \\ → 原始字节入 pending_ansi（按代码页解码）：
                // 对 ASCII 等价；且 DBCS 第二字节（如 SJIS「本」= 96 7B）写为 \'96\{ 时
                // 可与前一 \'96 字节配对解码，不会切断双字节序列。
                // 注意先入队后统一 flush，不能在配对中途 flush。
                if !cur_skip && !cur_pict {
                    pending_ansi.push(c);
                }
                i += 2;
            }
            _ => {
                if in_pict_data {
                    if b.is_ascii_hexdigit() {
                        pict_hex.push(b as char);
                    }
                } else if !cur_skip {
                    if b == b'\r' || b == b'\n' {
                        flush_ansi!();
                    } else if b >= 0x20 && b < 0x80 {
                        pending_ansi.push(b);
                    }
                }
                i += 1;
            }
        }
    }
    flush_run!();
    push_para!();
    paras
}

// ---------- 旧版二进制（doc/wps/ppt）：启发式 ----------

fn legacy_office_body(bytes: &[u8], ext: &str) -> Result<String, AppError> {
    if bytes.starts_with(b"PK\x03\x04") {
        // 实为 ZIP 容器（新版 WPS/Word 另存的兼容格式）→ 依次尝试
        if let Ok(b) = docx_body(bytes) {
            return Ok(b);
        }
        if let Ok(b) = pptx_body(bytes) {
            return Ok(b);
        }
        if let Ok(b) = odt_body(bytes) {
            return Ok(b);
        }
        if let Ok(b) = odp_body(bytes) {
            return Ok(b);
        }
        return Err(AppError::BundleError("ZIP 容器中未找到可识别的文档结构".into()));
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        // PowerPoint 二进制（0.8.4）：按 TextCharsAtom/TextBytesAtom 记录头精确提取。
        // 此前裸 UTF-16 启发式在真实 PPT 上只能拿到二进制噪声（矩阵测试实测）。
        let mut paras = if ext == "ppt" { ppt_record_text(bytes) } else { vec![] };
        if paras.is_empty() {
            paras = utf16_segments(bytes);
            if total_chars(&paras) < 40 {
                let ansi = ansi_segments(bytes);
                if total_chars(&ansi) > total_chars(&paras) {
                    paras = ansi;
                }
            }
        }
        // 过滤混入正文的 OLE 目录流名与元数据片段
        paras.retain(|p| !is_ole_meta_paragraph(p));
        if paras.is_empty() {
            return Err(AppError::BundleError(
                "旧版二进制格式中未提取到文本（启发式提取不适用于该文件）".into(),
            ));
        }
        let mut out = String::new();
        for p in &paras {
            out.push_str(&format!("<p>{}</p>", escape_html(p)));
        }
        out.push_str(
            "<p class=\"doc-note\">⚠ 旧版二进制格式为启发式文本提取，排版与图片不保留，建议另存为 docx/pptx 后重新导入。</p>",
        );
        return Ok(out);
    }
    Err(AppError::BundleError(format!(
        "无法识别的 {} 文件结构（既非 ZIP 容器也非 OLE 复合文档）",
        ext.to_uppercase()
    )))
}

/// PowerPoint 97-2003 二进制文本记录扫描：
/// 记录头 = ver/instance(2B) + recType(2B LE) + recLen(4B LE)。
/// - TextCharsAtom (0x0FA0)：UTF-16LE 文本
/// - TextBytesAtom (0x0FA8)：单字节 ANSI 文本
/// 提取后按可读率校验剔除伪记录头。
fn ppt_record_text(bytes: &[u8]) -> Vec<String> {
    let mut paras: Vec<String> = vec![];
    let mut i = 0usize;
    while i + 8 <= bytes.len() {
        let typ = u16::from_le_bytes([bytes[i + 2], bytes[i + 3]]);
        if typ != 0x0FA0 && typ != 0x0FA8 {
            i += 1;
            continue;
        }
        let len = u32::from_le_bytes([
            bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7],
        ]) as usize;
        let end = (i + 8 + len).min(bytes.len());
        if end <= i + 8 || len > 0x20000 {
            i += 8;
            continue;
        }
        let data = &bytes[i + 8..end];
        let text = if typ == 0x0FA0 && len % 2 == 0 {
            let units: Vec<u16> = data
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        } else if typ == 0x0FA8 {
            decode_text(data)
        } else {
            String::new()
        };
        // 可读率校验：正文记录应几乎全为可读字符，伪头解码出的乱码直接丢弃
        let total = text.chars().count();
        if total >= 2 {
            let readable = text
                .chars()
                .filter(|c| {
                    let u = *c as u32;
                    utf16_char_readable(u as u16)
                        || matches!(u, 0x2022 | 0x2018 | 0x2019 | 0x201C | 0x201D | 0x0B)
                })
                .count();
            if (readable as f64) / (total as f64) >= 0.8 {
                for seg in text.split(['\r', '\n', '\u{0B}']) {
                    let t = seg.trim_matches('\u{0}').trim();
                    if t.chars().count() >= 2 {
                        paras.push(t.to_string());
                    }
                }
            }
        }
        i = end;
    }
    dedup(paras)
}

// ==================== 旧格式启发式文本提取 ====================

fn total_chars(paras: &[String]) -> usize {
    paras.iter().map(|p| p.chars().count()).sum()
}

/// OLE 复合文档的目录流名／元数据键。UTF-16 启发式提取会把目录区里的
/// 这些名字当作正文段落输出（实测 WPS/doc/ppt 均命中），需过滤。
fn is_ole_meta_paragraph(p: &str) -> bool {
    let k = p.trim().to_ascii_lowercase();
    // 1. 已知目录流名与元数据键
    if matches!(k.as_str(),
        "root entry" | "summaryinformation" | "documentsummaryinformation"
        | "worddocument" | "worddocument2" | "0table" | "1table" | "data"
        | "objectpool" | "wpscustomdata" | "powerpoint document"
        | "ksoproductbuildver" | "icv" | "normal.dotm"
    ) {
        return true;
    }
    // 2. 纯十六进制指纹/GUID 片段（如 ICV: "60B27430E3BC49398A843DE5593900CE_13"）
    let hexy = |c: char| c.is_ascii_hexdigit() || c == '_' || c == '-';
    if k.len() >= 16 && k.chars().all(hexy) && k.chars().filter(|c| c.is_ascii_digit()).count() >= 4 {
        return true;
    }
    // 3. 区域码前缀版本号（如 KSOProductBuildVer 值 "2052-12.1.0.25222"）
    let digits_dots_dash = |c: char| c.is_ascii_digit() || c == '.' || c == '-';
    let mut parts = k.splitn(2, '-');
    if let (Some(loc), Some(rest)) = (parts.next(), parts.next()) {
        if (3..=4).contains(&loc.len()) && loc.chars().all(|c| c.is_ascii_digit())
            && rest.len() >= 3 && rest.chars().all(digits_dots_dash) {
            return true;
        }
    }
    false
}

fn utf16_char_readable(u: u16) -> bool {
    // 注意：刻意不收 CJK 扩展A区(3400–4DBF)与兼容表意区(F900–FAFF)——
    // 旧版二进制噪声经 UTF-16 重配对恰好集中落在这两区（如流名 "Calibri" 的 BE 误读、二进制碎片），
    // 收入会把大量垃圾段落当成正文输出。
    // 0.8.4：收入西里尔(0400–04FF)与谚文音节(AC00–D7A3)，否则俄文/韩文 Word/WPS 正文被整体丢弃。
    matches!(u,
        0x09 | 0x0A | 0x0D
        | 0x20..=0x7E
        | 0x00A0..=0x00FF
        | 0x0400..=0x04FF
        | 0x2010..=0x206F
        | 0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x4E00..=0x9FFF
        | 0xAC00..=0xD7A3
        | 0xFF00..=0xFFEF
    )
}

fn utf16_segments(bytes: &[u8]) -> Vec<String> {
    let mut best = collect_utf16_at(bytes, 0);
    if total_chars(&best) < 40 {
        let shifted = collect_utf16_at(bytes, 1);
        if total_chars(&shifted) > total_chars(&best) {
            best = shifted;
        }
    }
    best
}

fn collect_utf16_at(bytes: &[u8], offset: usize) -> Vec<String> {
    let mut paras: Vec<String> = vec![];
    let mut cur = String::new();
    let mut i = offset;
    while i + 1 < bytes.len() {
        let u = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if utf16_char_readable(u) {
            cur.push(char::from_u32(u as u32).unwrap_or(' '));
        } else {
            push_paragraphs(&mut cur, &mut paras);
        }
        i += 2;
    }
    push_paragraphs(&mut cur, &mut paras);
    paras.retain(|p| p.chars().count() >= 2);
    dedup(paras)
}

fn ansi_segments(bytes: &[u8]) -> Vec<String> {
    let mut segs: Vec<Vec<u8>> = vec![];
    let mut cur: Vec<u8> = vec![];
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let ok_seq_len = if b < 0x80 {
            if (0x20..=0x7E).contains(&b) || b == b'\t' || b == b'\r' || b == b'\n' {
                1
            } else {
                0
            }
        } else if (0x81..=0xFE).contains(&b)
            && i + 1 < bytes.len()
            && (0x40..=0xFE).contains(&bytes[i + 1])
            && bytes[i + 1] != 0x7F
        {
            2
        } else {
            0
        };
        if ok_seq_len > 0 {
            cur.extend_from_slice(&bytes[i..i + ok_seq_len]);
            i += ok_seq_len;
        } else {
            if cur.len() >= 8 {
                segs.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            i += 1;
        }
    }
    if cur.len() >= 8 {
        segs.push(cur);
    }
    let mut paras: Vec<String> = vec![];
    for seg in segs {
        let text = decode_text(&seg); // 0.8.4 起多编码评分，不再固定 GBK
        let mut buf = text.trim().to_string();
        push_paragraphs(&mut buf, &mut paras);
    }
    paras.retain(|p| p.chars().count() >= 2);
    dedup(paras)
}

fn push_paragraphs(buf: &mut String, out: &mut Vec<String>) {
    if buf.trim().is_empty() {
        buf.clear();
        return;
    }
    for line in buf.split(|c| c == '\n' || c == '\r') {
        let t = line.trim();
        if t.chars().count() >= 2 {
            out.push(t.to_string());
        }
    }
    buf.clear();
}

fn dedup(mut paras: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    paras.retain(|p| seen.insert(p.clone()));
    if paras.len() > 5000 {
        paras.truncate(5000);
    }
    paras
}

// ==================== 公共 HTML 生成 ====================

fn heading_level(style: &str) -> Option<u8> {
    let s = style.trim();
    if let Some(rest) = s.strip_prefix("Heading") {
        if let Ok(n) = rest.parse::<u8>() {
            return Some(n.clamp(1, 6));
        }
    }
    if let Ok(n) = s.parse::<u8>() {
        if (1..=6).contains(&n) {
            return Some(n);
        }
    }
    match s {
        "Title" => Some(1),
        "Subtitle" => Some(2),
        _ => None,
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    escape_html(s)
}

// ==================== ZIP 容器 ====================

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<std::io::Cursor<&[u8]>>, AppError> {
    let cur = std::io::Cursor::new(bytes);
    zip::ZipArchive::new(cur).map_err(|e| AppError::BundleError(format!("无法读取 ZIP 容器: {}", e)))
}

fn read_zip_entry(bytes: &[u8], name: &str) -> Result<Vec<u8>, AppError> {
    let mut z = open_zip(bytes)?;
    let mut f = z
        .by_name(name)
        .map_err(|e| AppError::BundleError(format!("容器中缺少 {}: {}", name, e)))?;
    let mut buf = vec![];
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

fn read_zip_entry_opt(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    read_zip_entry(bytes, name).ok()
}

/// 列出 zip 中 ppt/slides/slideN.xml 之类按编号排序的条目
fn zip_slide_list(bytes: &[u8], prefix: &str, suffix: &str) -> Result<Vec<String>, AppError> {
    let z = open_zip(bytes)?;
    let mut slides: Vec<(usize, String)> = vec![];
    for name in z.file_names() {
        if let Some(mid) = name.strip_prefix(prefix).and_then(|s| s.strip_suffix(suffix)) {
            if let Ok(num) = mid.parse::<usize>() {
                slides.push((num, name.to_string()));
            }
        }
    }
    slides.sort_by_key(|(n, _)| *n);
    Ok(slides.into_iter().map(|(_, s)| s).collect())
}

// ==================== 轻量 XML tokenizer ====================

#[derive(Debug, Clone)]
enum Tok {
    Open(String, String),
    Close(String),
    Self_(String, String),
    Txt(String),
}

fn xml_tokens(s: &str) -> Vec<Tok> {
    let bytes = s.as_bytes();
    let mut out: Vec<Tok> = vec![];
    let mut i = 0usize;
    let mut txt_start = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if i > txt_start {
                out.push(Tok::Txt(xml_unescape(&s[txt_start..i])));
            }
            let rest = &s[i..];
            if rest.starts_with("<!--") {
                match rest.find("-->") {
                    Some(p) => {
                        i += p + 3;
                        txt_start = i;
                        continue;
                    }
                    None => break,
                }
            }
            if rest.starts_with("<![CDATA[") {
                match rest.find("]]>") {
                    Some(p) => {
                        out.push(Tok::Txt(rest[9..p].to_string()));
                        i += p + 3;
                        txt_start = i;
                        continue;
                    }
                    None => break,
                }
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                match rest.find('>') {
                    Some(p) => {
                        i += p + 1;
                        txt_start = i;
                        continue;
                    }
                    None => break,
                }
            }
            let Some(gt_rel) = rest.find('>') else { break };
            let gt = i + gt_rel;
            let inner = &s[i + 1..gt];
            if let Some(name) = inner.strip_prefix('/') {
                out.push(Tok::Close(name.trim().to_string()));
            } else if let Some(core) = inner.strip_suffix('/') {
                let (n, a) = split_tag(core);
                out.push(Tok::Self_(n, a));
            } else {
                let (n, a) = split_tag(inner);
                out.push(Tok::Open(n, a));
            }
            i = gt + 1;
            txt_start = i;
        } else {
            i += 1;
        }
    }
    if txt_start < s.len() {
        out.push(Tok::Txt(xml_unescape(&s[txt_start..])));
    }
    out
}

fn split_tag(inner: &str) -> (String, String) {
    let trimmed = inner.trim();
    match trimmed.find(|c: char| c.is_whitespace()) {
        Some(p) => (trimmed[..p].to_string(), trimmed[p..].trim().to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

fn attrs_get(attrs: &str, name: &str) -> Option<String> {
    for part in attrs.split_whitespace() {
        let Some(eq) = part.find('=') else { continue };
        let key = &part[..eq];
        if key != name {
            continue;
        }
        let val = part[eq + 1..].trim_matches(|c| c == '"' || c == '\'');
        return Some(val.to_string());
    }
    None
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let semi = after.find(';');
        let entity = semi.map(|e| &after[1..e]).unwrap_or("");
        let replaced = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match (replaced, semi) {
            (Some(ch), Some(e)) => {
                out.push(ch);
                rest = &after[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ==================== 文档页统一外观 ====================

fn doc_page_html(title: &str, body: &str, ext: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head><meta charset="utf-8"><title>{title}</title>
<style>
body{{font-family:"Segoe UI","Microsoft YaHei","PingFang SC",sans-serif;color:#24292f;background:#fff;max-width:880px;margin:0 auto;padding:36px 44px;line-height:1.8;font-size:15px;}}
.doc-meta{{display:flex;justify-content:space-between;color:#8b949e;font-size:12px;border-bottom:1px solid #d8dee4;padding-bottom:10px;margin-bottom:22px;}}
h1,h2,h3,h4{{color:#1f2328;line-height:1.4;margin:1.4em 0 .6em;}}
h1{{font-size:1.7em;border-bottom:2px solid #d8dee4;padding-bottom:8px;}}
h2{{font-size:1.4em;}}h3{{font-size:1.2em;}}
p{{margin:.7em 0;word-break:break-word;}}
p.doc-center,h1.doc-center,h2.doc-center{{text-align:center;}}
.doc-right{{text-align:right;}}
.doc-justify{{text-align:justify;}}
p.doc-li{{padding-left:1.4em;text-indent:-1.4em;}}
p.doc-li::before{{content:"•  ";color:#59636e;}}
pre.doc-plain{{white-space:pre-wrap;word-break:break-word;background:#f6f8fa;border:1px solid #d8dee4;border-radius:6px;padding:16px 20px;font-family:Consolas,"Microsoft YaHei",monospace;font-size:13px;line-height:1.7;}}
table{{border-collapse:collapse;margin:.8em 0;width:100%;}}
th,td{{border:1px solid #d8dee4;padding:6px 12px;vertical-align:top;text-align:left;}}
tr:nth-child(odd){{background:#f6f8fa;}}
img.doc-img{{max-width:100%;height:auto;border-radius:4px;}}
.doc-img-box,.doc-img{{margin:10px 0;}}
.doc-img-box{{text-align:center;}}
.doc-slide{{border:1px solid #d8dee4;border-radius:8px;padding:18px 26px;margin:16px 0;background:#fff;}}
.doc-slide-no,.doc-pdf-no{{color:#8b949e;font-size:11px;letter-spacing:1px;margin-bottom:6px;text-transform:uppercase;}}
.doc-slide h2{{margin-top:0;font-size:1.3em;}}
.doc-pdf-page{{border:1px solid #d8dee4;border-radius:8px;padding:12px;margin:16px 0;background:#fff;}}
.doc-pdf-page img{{width:100%;display:block;}}
.doc-note{{color:#9a6700;background:#fff8c5;border:1px solid #d4a72c66;border-radius:6px;padding:10px 14px;font-size:13px;}}
.doc-error{{border:1px solid #daa2a2;background:#fff3f3;border-radius:8px;padding:20px 26px;}}
.doc-error h2{{margin-top:0;color:#a12622;font-size:1.2em;}}
blockquote{{border-left:4px solid #d8dee4;color:#59636e;margin:.8em 0;padding:.1em 1em;}}
table.doc-table td p{{margin:.3em 0;}}
code{{background:#f6f8fa;border-radius:4px;padding:2px 6px;font-family:Consolas,monospace;font-size:.9em;}}
a{{color:#0969da;}}
</style></head>
<body>
<div class="doc-meta"><span>📄 {title}</span><span>{label}</span></div>
{body}
</body>
</html>"#,
        title = title,
        label = ext_label(ext),
        body = body
    )
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    // 1x1 红色 PNG
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00,
        0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
        0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    fn b64_png() -> String {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, TINY_PNG)
    }

    #[test]
    fn test_ext_detection() {
        for e in ["pdf", "doc", "docx", "rtf", "wps", "odt", "odp", "ppt", "pptx", "txt", "md"] {
            assert!(is_doc_ext(e), "{}", e);
        }
        assert!(!is_doc_ext("html"));
        assert!(!is_doc_ext("png"));
    }

    #[test]
    fn test_txt_gbk() {
        let bytes = [0xD6, 0xD0, 0xCE, 0xC4, 0xB2, 0xE2, 0xCA, 0xD4];
        let html = convert_doc_to_html(&bytes, "txt", "note.txt");
        assert!(html.contains("中文测试"), "{}", html);
        assert!(html.contains("纯文本"));
    }

    #[test]
    fn test_md() {
        let md = b"# Title A\n\n- item 1\n- item 2\n\n**bold** text";
        let html = convert_doc_to_html(md, "md", "readme.md");
        assert!(html.contains("<h1>Title A</h1>"), "{}", html);
        assert!(html.contains("<li>item 1</li>"));
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[test]
    fn test_rtf_chinese() {
        let rtf = b"{\\rtf1\\ansi\\ansicpg936{\\fonttbl{\\f0 SimSun;}}Hello \\b bold \\b0 world\\par \\'D6\\'D0\\'CE\\'C4 \\u22202? tail}";
        let html = convert_doc_to_html(rtf, "rtf", "a.rtf");
        assert!(html.contains("Hello"), "{}", html);
        assert!(html.contains("中文"), "GBK rtf decode failed: {}", html);
        assert!(!html.contains("fonttbl"), "fonttbl should be skipped");
    }

    #[test]
    fn test_rtf_style_and_pict() {
        // 1x1 PNG 的十六进制
        let png_hex: String = TINY_PNG.iter().map(|b| format!("{:02x}", b)).collect();
        let rtf = "{\\rtf1\\ansi Title\\par\\qc {\\b centered bold}\\par\\fs36 big text\\par{\\pict{\\*\\blipuid 1234}\\pngblip\n"
            .to_string()
            + &png_hex
            + "}}";
        let html = convert_doc_to_html(rtf.as_bytes(), "rtf", "styled.rtf");
        assert!(html.contains("<b>centered bold</b>"), "{}", html);
        assert!(html.contains("doc-center"), "alignment missing: {}", html);
        assert!(html.contains("data:image/png;base64,"), "pict not embedded: {}", html);
        assert!(html.contains("font-size:18pt"), "fs36 → 18pt missing: {}", html);
    }

    #[test]
    fn test_rtf_pict_nested_pngblip() {
        // 0.8.x 矩阵测试实测：\\pngblip 写在 {\\pngblip} 子组内时 fmt 丢失、图片被静默丢弃
        let png_hex: String = TINY_PNG.iter().map(|b| format!("{:02x}", b)).collect();
        let rtf = "{\\rtf1\\ansi {\\pict{\\pngblip} ".to_string() + &png_hex + "}";
        let html = convert_doc_to_html(rtf.as_bytes(), "rtf", "nested.rtf");
        assert!(html.contains("data:image/png;base64,"), "nested pict dropped: {}", html);
    }

    #[test]
    fn test_docx() {
        let doc_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>安装指南</w:t></w:r></w:p>
<w:p><w:r><w:t>第一步：&amp;安装&lt;依赖&gt;</w:t></w:r><w:r><w:br/><w:t>第二行</w:t></w:r></w:p>
<w:p><w:r><w:t>plain paragraph</w:t></w:r></w:p>
</w:body></w:document>"#;
        let zip_bytes = make_zip(&[("word/document.xml", doc_xml.as_bytes())]);
        let html = convert_doc_to_html(&zip_bytes, "docx", "指南.docx");
        assert!(html.contains("<h1>安装指南</h1>"), "{}", html);
        assert!(html.contains("&amp;安装&lt;依赖&gt;"), "entity escaped wrongly: {}", html);
        assert!(html.contains("&gt;<br>第二行"));
        assert!(html.contains("<p>plain paragraph</p>"));
        assert!(html.contains("Word 文档"));
    }

    #[test]
    fn test_docx_rich() {
        let doc_xml = r#"<w:document xmlns:w="w" xmlns:a="a">
<w:body>
<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/><w:i/><w:color w:val="FF0000"/><w:sz w:val="32"/></w:rPr><w:t>居中粗斜红字</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr/></w:pPr><w:r><w:t>列表项一</w:t></w:r></w:p>
<w:p><w:r><w:t>列A</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>cell2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:drawing><a:blip r:embed="rId5"/></w:drawing></w:r></w:p>
<w:p><w:hyperlink r:id="rId7"><w:r><w:t>外部链接</w:t></w:r></w:hyperlink></w:p>
</w:body></w:document>"#;
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId5" Type="image" Target="media/image1.png"/>
<Relationship Id="rId7" Type="hyperlink" Target="https://example.com" TargetMode="External"/>
</Relationships>"#;
        let zip_bytes = make_zip(&[
            ("word/document.xml", doc_xml.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/media/image1.png", TINY_PNG),
        ]);
        let html = convert_doc_to_html(&zip_bytes, "docx", "rich.docx");
        assert!(html.contains("doc-center"), "align missing: {}", html);
        assert!(html.contains("<b><i>居中粗斜红字</i></b>"), "style missing: {}", html);
        assert!(html.contains("color:#FF0000"));
        assert!(html.contains("font-size:16pt"));
        assert!(html.contains("doc-li"), "list missing: {}", html);
        assert!(html.contains("<td><p>cell1</p></td>"), "table missing: {}", html);
        assert!(html.contains(&format!("data:image/png;base64,{}", b64_png())), "image missing: {}", html);
        assert!(html.contains("https://example.com"), "hyperlink missing: {}", html);
    }

    #[test]
    fn test_pptx() {
        let slide1 = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>产品介绍</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:txBody><a:p><a:r><a:t>第一页正文</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;
        let slide2 = r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>功能列表</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;
        let zip_bytes = make_zip(&[
            ("ppt/slides/slide2.xml", slide2.as_bytes()),
            ("ppt/slides/slide1.xml", slide1.as_bytes()),
            ("ppt/slides/slide10.xml", slide2.as_bytes()),
        ]);
        let html = convert_doc_to_html(&zip_bytes, "pptx", "deck.pptx");
        let p1 = html.find("产品介绍").unwrap();
        let p2 = html.find("功能列表").unwrap();
        assert!(p1 < p2, "slides out of order");
        assert!(html.contains("第 1 页") && html.contains("第 3 页"));
        assert!(html.contains("<h2>产品介绍</h2>"));
        assert!(html.contains("<p>第一页正文</p>"));
    }

    #[test]
    fn test_pptx_image_and_order() {
        // 两个形状：y=500 的图片形状在前（XML 中第二个），y=100 的文本在前
        let slide = r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:r="r"><p:cSld><p:spTree>
<p:sp><p:spPr><a:xfrm><a:off x="100" y="500"/><a:ext cx="1" cy="1"/></a:xfrm></p:spPr><p:txBody><a:p><a:r><a:t>下面的文本</a:t></a:r></a:p></p:txBody></p:sp>
<p:pic><p:spPr><a:xfrm><a:off x="0" y="100"/></a:xfrm></p:spPr><p:blipFill><a:blip r:embed="rId2"/></p:blipFill></p:pic>
</p:spTree></p:cSld></p:sld>"#;
        let rels = r#"<Relationships xmlns="x"><Relationship Id="rId2" Target="../media/pic1.png"/></Relationships>"#;
        let zip_bytes = make_zip(&[
            ("ppt/slides/slide1.xml", slide.as_bytes()),
            ("ppt/slides/_rels/slide1.xml.rels", rels.as_bytes()),
            ("ppt/media/pic1.png", TINY_PNG),
        ]);
        let html = convert_doc_to_html(&zip_bytes, "pptx", "order.pptx");
        assert!(html.contains("data:image/png;base64,"), "image missing: {}", html);
        let img_pos = html.find("data:image/png;base64,").unwrap();
        let text_pos = html.find("下面的文本").unwrap();
        assert!(img_pos < text_pos, "y-order wrong: image should come before text (y=100 < y=500)");
    }

    #[test]
    fn test_odt() {
        let content = r#"<office:document-content xmlns:text="t" xmlns:office="o">
<office:body><office:text>
<text:h text:outline-level="1">概述</text:h>
<text:p>这是 &amp; 符号与&lt;标签&gt;</text:p>
<text:h text:outline-level="2">细节</text:h>
<text:p>正文内容</text:p>
</office:text></office:body></office:document-content>"#;
        let zip_bytes = make_zip(&[("content.xml", content.as_bytes())]);
        let html = convert_doc_to_html(&zip_bytes, "odt", "doc.odt");
        assert!(html.contains("<h1>概述</h1>"), "{}", html);
        assert!(html.contains("<h2>细节</h2>"));
        assert!(html.contains("这是 &amp; 符号与&lt;标签&gt;"));
    }

    #[test]
    fn test_odt_image_and_styles() {
        let content = r#"<office:document-content xmlns:text="t" xmlns:office="o" xmlns:draw="d" xmlns:xlink="x" xmlns:style="s" xmlns:fo="f">
<office:automatic-styles>
<style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:text-align="center"/></style:style>
<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
</office:automatic-styles>
<office:body><office:text>
<text:p text:style-name="P1"><text:span text:style-name="T1">粗体居中</text:span></text:p>
<text:p><draw:image xlink:href="Pictures/shot.png"/></text:p>
</office:text></office:body></office:document-content>"#;
        let zip_bytes = make_zip(&[
            ("content.xml", content.as_bytes()),
            ("Pictures/shot.png", TINY_PNG),
        ]);
        let html = convert_doc_to_html(&zip_bytes, "odt", "styled.odt");
        assert!(html.contains("<b>粗体居中</b>"), "bold missing: {}", html);
        assert!(html.contains("doc-center"), "align missing: {}", html);
        assert!(html.contains("data:image/png;base64,"), "image missing: {}", html);
    }

    #[test]
    fn test_odt_table_with_cell_image() {
        // 0.8.x 矩阵测试实测：odt 表格不被识别（结构拍平）、格内未包 text:p 的图片丢失
        let content = r#"<office:document-content xmlns:text="t" xmlns:office="o" xmlns:table="tb" xmlns:draw="d" xmlns:xlink="x">
<office:body><office:text>
<text:h text:outline-level="1">ODT 4x4 矩阵图文混编测试</text:h>
<table:table><table:table-row>
<table:table-cell><text:p>R1C1 文本格</text:p></table:table-cell>
<table:table-cell><draw:frame><draw:image xlink:href="Pictures/m1.png"/></draw:frame></table:table-cell>
</table:table-row></table:table>
<text:p>表格后段落</text:p>
</office:text></office:body></office:document-content>"#;
        let zip_bytes = make_zip(&[
            ("content.xml", content.as_bytes()),
            ("Pictures/m1.png", TINY_PNG),
        ]);
        let html = convert_doc_to_html(&zip_bytes, "odt", "matrix.odt");
        assert!(html.contains("ODT 4x4 矩阵图文混编测试"), "{}", html);
        assert!(html.contains("doc-table"), "table structure missing: {}", html);
        assert!(html.contains("R1C1 文本格"), "cell text missing: {}", html);
        assert!(html.contains("data:image/png;base64,"), "cell image missing: {}", html);
        assert!(html.contains("表格后段落"), "content after table missing: {}", html);
    }

    #[test]
    fn test_md_table() {
        // 0.8.x 矩阵测试实测：md 表格未启用 GFM 表格扩展，管道符原样输出
        let md = "# 标题\n\n| A | B |\n|---|---|\n| 1 | 2 |\n".as_bytes();
        let html = convert_doc_to_html(md, "md", "t.md");
        assert!(html.contains("<table>"), "md table not rendered: {}", html);
        assert!(html.contains("<td>1</td>"), "cell missing: {}", html);
    }

    #[test]
    fn test_odp() {
        let content = r#"<office:document-content xmlns:text="t" xmlns:draw="d" xmlns:office="o">
<office:body><office:presentation>
<draw:page><text:p>封面标题</text:p><text:p>副标题</text:p></draw:page>
<draw:page><text:p>目录</text:p></draw:page>
</office:presentation></office:body></office:document-content>"#;
        let zip_bytes = make_zip(&[("content.xml", content.as_bytes())]);
        let html = convert_doc_to_html(&zip_bytes, "odp", "pres.odp");
        assert!(html.contains("第 1 页") && html.contains("第 2 页"), "{}", html);
        assert!(html.contains("封面标题"));
    }

    #[test]
    fn test_odp_frames_order_image() {
        let content = r#"<office:document-content xmlns:text="t" xmlns:draw="d" xmlns:office="o" xmlns:svg="s" xmlns:xlink="x">
<office:body><office:presentation>
<draw:page>
<draw:frame svg:y="800"><text:p>底部文本</text:p></draw:frame>
<draw:frame svg:y="100"><draw:image xlink:href="Pictures/top.png"/></draw:frame>
</draw:page>
</office:presentation></office:body></office:document-content>"#;
        let zip_bytes = make_zip(&[
            ("content.xml", content.as_bytes()),
            ("Pictures/top.png", TINY_PNG),
        ]);
        let html = convert_doc_to_html(&zip_bytes, "odp", "order.odp");
        assert!(html.contains("data:image/png;base64,"), "image missing: {}", html);
        let img_pos = html.find("data:image/png;base64,").unwrap();
        let text_pos = html.find("底部文本").unwrap();
        assert!(img_pos < text_pos, "frame order wrong");
    }

    #[test]
    fn test_legacy_utf16() {
        let mut bytes = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend_from_slice(&[0x00; 16]);
        let text = "这是旧版Word文档的正文内容，包含中文与英文mixed content。\r\n第二段文字，用于验证启发式提取器可以正确分段输出。";
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let html = convert_doc_to_html(&bytes, "doc", "old.doc");
        assert!(html.contains("这是旧版Word文档的正文内容"), "{}", html);
        assert!(html.contains("第二段文字，用于验证启发式提取器可以正确分段输出。"));
        assert!(html.contains("启发式"));
    }

    #[test]
    fn test_legacy_zip_fallback() {
        let doc_xml = r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>actually docx</w:t></w:r></w:p></w:body></w:document>"#;
        let zip_bytes = make_zip(&[("word/document.xml", doc_xml.as_bytes())]);
        let html = convert_doc_to_html(&zip_bytes, "doc", "fake.doc");
        assert!(html.contains("actually docx"), "{}", html);
    }

    #[test]
    fn test_legacy_ole_noise_filtered() {
        // OLE 目录流名与元数据片段不应出现在正文中（0.8.x 全格式合并测试实测泄漏）
        let mut bytes = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend_from_slice(&[0x00; 16]);
        let noise = "Root Entry\nSummaryInformation\nWordDocument\n0Table\nData\nWpsCustomData\nKSOProductBuildVer\nICV\n60B27430E3BC49398A843DE5593900CE_13\n2052-12.1.0.25222\n";
        let body = "真实正文第一段：编号 1234567890。\n真实正文第二段，用于确认过滤不误伤正常中文段落。";
        let text = format!("{}{}", noise, body);
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let html = convert_doc_to_html(&bytes, "wps", "old.wps");
        assert!(html.contains("真实正文第一段"), "{}", html);
        assert!(html.contains("真实正文第二段"));
        for bad in ["Root Entry", "SummaryInformation", "WordDocument", "0Table",
                    "WpsCustomData", "KSOProductBuildVer", "ICV",
                    "60B27430E3BC49398A843DE5593900CE_13", "2052-12.1.0.25222"] {
            assert!(!html.contains(bad), "OLE 噪声泄漏: {} in {}", bad, html);
        }
    }

    #[test]
    fn test_legacy_utf16_noise_ranges_not_readable() {
        // 扩展A区/兼容区字符不再视为可读（二进制噪声源）；
        // 0.8.4 起谚文与西里尔改为可读（俄文/韩文正文需要）
        assert!(!super::utf16_char_readable(0x4700)); // 䜀 (扩展A)
        assert!(!super::utf16_char_readable(0xF900)); // 兼容区
        assert!(super::utf16_char_readable(0xAC00)); // 가
        assert!(super::utf16_char_readable(0x043C)); // м
        assert!(super::utf16_char_readable(0x4E2D)); // 中
        assert!(super::utf16_char_readable(0x41));   // A
    }

    /// 测试辅助：按 encoding_rs 编码名编码字符串为字节
    fn enc_bytes(enc: &'static encoding_rs::Encoding, text: &str) -> Vec<u8> {
        let (cow, _, _) = enc.encode(text);
        cow.into_owned()
    }

    #[test]
    fn test_decode_text_multi_encoding() {
        // 0.8.4 矩阵测试实测：非 GBK 旧编码此前全部乱码
        // Shift-JIS（含片假名）
        let sjis = enc_bytes(encoding_rs::SHIFT_JIS, "日本語スタイルマトリクス\nテキスト一行目内容");
        assert!(decode_text(&sjis).contains("日本語スタイルマトリクス"), "{}", decode_text(&sjis));
        // Big5（GBK 误解码无法稳定命中全部常用字）
        let big5 = enc_bytes(encoding_rs::BIG5, "繁體中文樣式矩陣\n純文字第一行內容");
        assert!(decode_text(&big5).contains("繁體中文樣式矩陣"), "{}", decode_text(&big5));
        // EUC-KR
        let euckr = enc_bytes(encoding_rs::EUC_KR, "한국어 스타일 매트릭스\n일반 텍스트 첫 줄");
        assert!(decode_text(&euckr).contains("한국어 스타일 매트릭스"), "{}", decode_text(&euckr));
        // Windows-1251 西里尔
        let cp1251 = enc_bytes(encoding_rs::WINDOWS_1251, "Русская матрица стилей\nОбычный текст");
        assert!(decode_text(&cp1251).contains("Русская матрица стилей"), "{}", decode_text(&cp1251));
        // GBK 简体仍正确
        let gbk = enc_bytes(encoding_rs::GBK, "简体中文样式矩阵");
        assert!(decode_text(&gbk).contains("简体中文样式矩阵"));
        // UTF-8 直通
        let utf8 = "简体中文样式矩阵".as_bytes();
        assert_eq!(decode_text(utf8), "简体中文样式矩阵");
        // BOM
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("带BOM文本".as_bytes());
        assert!(decode_text(&bom).starts_with("带BOM文本"));
    }

    /// 0.8.5 扩充候选：希腊/希伯来/阿拉伯/泰文/土耳其/波罗的海 + KOI8/IBM866/GB18030/EUC-JP
    #[test]
    fn test_decode_text_expanded_encodings() {
        // Windows-1253 希腊（含带调元音）
        let cp1253 = enc_bytes(encoding_rs::WINDOWS_1253, "Ελληνική μήτρα στυλ\nΑπλό κείμενο δοκιμής");
        let decoded = decode_text(&cp1253);
        assert!(decoded.contains("Ελληνική"), "cp1253 decode failed: {}", decoded);
        // Windows-1255 希伯来
        let cp1255 = enc_bytes(encoding_rs::WINDOWS_1255, "מטריצת סגנון בעברית\nטקסט רגיל בשורה הראשונה");
        assert!(decode_text(&cp1255).contains("מטריצת סגנון"), "{}", decode_text(&cp1255));
        // Windows-1256 阿拉伯
        let cp1256 = enc_bytes(encoding_rs::WINDOWS_1256, "مصفوفة الأنماط بالعربية\nنص عادي في السطر الأول");
        assert!(decode_text(&cp1256).contains("مصفوفة الأنماط"), "{}", decode_text(&cp1256));
        // Windows-1254 土耳其（ğ ı İ ş Ş 专属字母）
        let cp1254 = enc_bytes(encoding_rs::WINDOWS_1254, "Türkçe stil matrisi\nBaşlıkta ıİğŞş harfleri");
        assert!(decode_text(&cp1254).contains("Başlıkta ıİğŞş"), "{}", decode_text(&cp1254));
        // KOI8-R 西里尔
        let koi8 = enc_bytes(encoding_rs::KOI8_R, "Русская матрица стилей\nОбычный текст");
        assert!(decode_text(&koi8).contains("Русская матрица"), "{}", decode_text(&koi8));
        // IBM866（DOS 俄文）
        let cp866 = enc_bytes(encoding_rs::IBM866, "Русская матрица стилей\nОбычный текст");
        assert!(decode_text(&cp866).contains("Русская матрица"), "{}", decode_text(&cp866));
        // EUC-JP 日文
        let eucjp = enc_bytes(encoding_rs::EUC_JP, "日本語スタイルマトリクス\n通常テキストの一行目");
        assert!(decode_text(&eucjp).contains("日本語スタイル"), "{}", decode_text(&eucjp));
        // GB18030（GBK 简体仍由 GBK 优先正确解码）
        let gb18030 = enc_bytes(encoding_rs::GB18030, "简体中文样式矩阵\n常规文本第一行");
        assert!(decode_text(&gb18030).contains("简体中文样式矩阵"), "{}", decode_text(&gb18030));
    }

    /// 0.8.5 回归防线：西欧重音字母不得被 cp1251/cp1253 洪泛（词内混排惩罚）
    #[test]
    fn test_decode_text_western_no_flood() {
        // 法文 cp1252：é 在 ASCII 词内，cp1251 误读会产生西里尔混排
        let fr = enc_bytes(encoding_rs::WINDOWS_1252, "Résumé du manuel\nCréer une page spéciale à côté");
        let decoded = decode_text(&fr);
        assert!(decoded.contains("Résumé du manuel"), "French cp1252 flooded: {}", decoded);
        // 德文 cp1252
        let de = enc_bytes(encoding_rs::WINDOWS_1252, "Benützerhandbücher für größere Seiten");
        assert!(decode_text(&de).contains("größere"), "{}", decode_text(&de));
        // 俄文 cp1251 仍正确（西里尔词内无 ASCII 混排）
        let ru = enc_bytes(encoding_rs::WINDOWS_1251, "Русская матрица стилей\nОбычный текст");
        assert!(decode_text(&ru).contains("Русская матрица стилей"), "{}", decode_text(&ru));
    }

    #[test]
    fn test_txt_legacy_encodings() {
        let big5 = enc_bytes(encoding_rs::BIG5, "繁體中文樣式矩陣");
        let html = convert_doc_to_html(&big5, "txt", "tw.txt");
        assert!(html.contains("繁體中文樣式矩陣"), "{}", html);
        let sjis = enc_bytes(encoding_rs::SHIFT_JIS, "日本語スタイルマトリクス");
        let html = convert_doc_to_html(&sjis, "txt", "jp.txt");
        assert!(html.contains("日本語スタイルマトリクス"), "{}", html);
    }

    #[test]
    fn test_md_legacy_encoding() {
        let big5 = enc_bytes(encoding_rs::BIG5, "# 繁體中文樣式矩陣\n\n| 欄一 | 欄二 |\n|---|---|\n| 資料甲 | 資料乙 |\n");
        let html = convert_doc_to_html(&big5, "md", "tw.md");
        assert!(html.contains("<h1>繁體中文樣式矩陣</h1>"), "{}", html);
        assert!(html.contains("<td>資料甲</td>"), "{}", html);
    }

    #[test]
    fn test_rtf_codepages() {
        // 0.8.4：ansicpg 932/949/950/1251 全代码页映射
        let mk = |cp: u16, enc: &'static encoding_rs::Encoding, text: &str| -> Vec<u8> {
            let mut out = format!(r#"{{\rtf1\ansi\ansicpg{}\deff0{{\fonttbl{{\f0 T;}}}}\fs24 "#, cp)
                .into_bytes();
            for ch in text.chars() {
                if (ch as u32) < 0x80 {
                    match ch {
                        '\\' => out.extend_from_slice(b"\\\\"),
                        '{' => out.extend_from_slice(b"\\{"),
                        '}' => out.extend_from_slice(b"\\}"),
                        _ => out.push(ch as u8),
                    }
                } else {
                    // 非 ASCII 字符的全部编码字节统一写 \'xx（真实 CJK RTF 写法）
                    let s = ch.to_string();
                    let (cow, _, _) = enc.encode(&s);
                    for b in cow.iter() {
                        out.extend_from_slice(format!("\\'{:02x}", b).as_bytes());
                    }
                }
            }
            out.extend_from_slice(b"\\par}");
            out
        };
        let cases: [(&'static encoding_rs::Encoding, u16, &str); 4] = [
            (encoding_rs::SHIFT_JIS, 932, "日本語スタイルマトリクス"),
            (encoding_rs::EUC_KR, 949, "한국어 스타일 매트릭스"),
            (encoding_rs::BIG5, 950, "繁體中文樣式矩陣"),
            (encoding_rs::WINDOWS_1251, 1251, "Русская матрица стилей"),
        ];
        for (enc, cp, text) in cases {
            let rtf = mk(cp, enc, text);
            let html = convert_doc_to_html(&rtf, "rtf", "cp.rtf");
            assert!(html.contains(text), "ansicpg{}({}) 解码失败: {}", cp, enc.name(), html);
        }
    }

    #[test]
    fn test_ppt_record_text() {
        // TextCharsAtom (0x0FA0)：UTF-16LE 记录
        let mut bytes = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        let text = "简体中文样式矩阵 - PPT\r副标题内容";
        let mut data: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let mut header = vec![0x00u8, 0x00, 0xA0, 0x0F];
        header.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.append(&mut data);
        let html = convert_doc_to_html(&bytes, "ppt", "old.ppt");
        assert!(html.contains("简体中文样式矩阵 - PPT"), "{}", html);
        assert!(html.contains("副标题内容"), "{}", html);
    }

    #[test]
    fn test_ppt_record_text_bytes_atom() {
        // TextBytesAtom (0x0FA8)：单字节 ANSI 记录（英文 PPT 常见）
        let mut bytes = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        let text = b"English Style Matrix - PPT";
        let mut header = vec![0x00u8, 0x00, 0xA8, 0x0F];
        header.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(text);
        let html = convert_doc_to_html(&bytes, "ppt", "old.ppt");
        assert!(html.contains("English Style Matrix - PPT"), "{}", html);
    }

    #[test]
    fn test_legacy_doc_korean_cyrillic() {
        // 0.8.4 矩阵测试实测：谚文/西里尔正文此前被 UTF-16 可读过滤整体丢弃
        let mut bytes = vec![0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        bytes.extend_from_slice(&[0x00; 16]);
        let text = "한국어 스타일 매트릭스\r\nРусская матрица стилей";
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let html = convert_doc_to_html(&bytes, "doc", "old.doc");
        assert!(html.contains("한국어 스타일 매트릭스"), "{}", html);
        assert!(html.contains("Русская матрица стилей"), "{}", html);
    }

    #[test]
    fn test_corrupt_doc_degrades() {
        let html = convert_doc_to_html(b"garbage-data-not-any-format", "docx", "broken.docx");
        assert!(html.contains("无法提取"), "should degrade: {}", html);
    }

    #[test]
    fn test_pdf() {
        let pdf = build_min_pdf("Hello PDF");
        let html = convert_doc_to_html(&pdf, "pdf", "doc.pdf");
        // pdfium 渲染：整页 PNG
        assert!(html.contains("data:image/png;base64,"), "pdfium render missing: {}", html);
        assert!(html.contains("第 1 页"));
        assert!(html.contains("PDF 文档"));
    }

    #[test]
    fn test_pdf_fallback_text() {
        // pdfium 加载失败时降级为文本提取 —— 用损坏的 PDF 触发渲染失败路径
        let mut pdf = build_min_pdf("Hello PDF");
        pdf.truncate(30); // 破坏结构
        let html = convert_doc_to_html(&pdf, "pdf", "doc.pdf");
        // 无论如何应有输出（渲染失败 → 文本提取 → 若全失败则占位页）
        assert!(html.contains("doc-meta"), "{}", html);
    }

    /// 生成带合法 xref 的最小 PDF
    fn build_min_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 24 Tf 72 720 Td ({}) Tj ET", text);
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", stream.len(), stream),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![];
        for (i, obj) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, obj).as_bytes());
        }
        let xref_pos = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF", objs.len() + 1, xref_pos).as_bytes());
        pdf
    }

    #[test]
    fn test_xml_unescape() {
        assert_eq!(xml_unescape("a&amp;b&#65;&#x42;"), "a&bAB");
        assert_eq!(xml_unescape("no entity"), "no entity");
        assert_eq!(xml_unescape("bad &unknown; stays"), "bad &unknown; stays");
    }

    #[test]
    fn test_hex_decode() {
        assert_eq!(hex_decode("89504e47"), vec![0x89, 0x50, 0x4E, 0x47]);
        assert_eq!(hex_decode("89 50\n4E47"), vec![0x89, 0x50, 0x4E, 0x47]);
        assert_eq!(hex_decode("abc"), vec![0xAB]); // 奇数长度丢弃末位
    }
}
