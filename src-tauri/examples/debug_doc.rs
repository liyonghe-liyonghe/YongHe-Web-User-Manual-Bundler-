//! 调试：单步验证 odt/odp 转换器对生成文件的解析结果。

use manualbundler::docs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let ext = &args[2];
    let bytes = std::fs::read(path).unwrap();
    let html = docs::convert_doc_to_html(&bytes, ext, "debug");
    let body = html.split("<body>").last().unwrap_or(&html);
    let body = body.split("</body>").next().unwrap_or(body);
    println!("{}", body);
}
