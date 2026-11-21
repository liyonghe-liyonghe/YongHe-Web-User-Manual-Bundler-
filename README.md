# 泳和-网页/用户手册合成器

把 PDF、Word、PPT、WPS、ODF、RTF、Markdown、TXT 等异构文档编排成一本**可离线打开的单文件 HTML 网页手册**。图片、样式、脚本、文档全部内嵌，导出的 HTML 一个文件即可分发，断网可看。

- 版本：`0.8.7`
- 许可：MIT（见 `LICENSE`）
- 首个公开标签：`v0.8.7`
- 仓库地址：待定（由发布者创建仓库后填入；本件不预置任何未确认地址）

## 功能

- **异构文档统一转换**：支持 PDF/DOC/DOCX/WPS/ODT/ODP/PPT/PPTX/RTF/MD/TXT/HTML 等格式；PDF 逐页保真渲染，动态库缺失时降级为文本提取。
- **多编码宽容解码**：19 种编码候选配合汉字/假名/谚文门控，旧编码（GBK/Big5/Shift-JIS 等）文档自动还原。
- **导航结构可视化编排**：左栏投喂目录、中栏拖拽分组排序改名、右栏实时预览。
- **缺件自动补齐**：页面引用的本地资源缺失时，自动在已记目录搜索补齐，兜底弹窗选择来源文件夹。
- **单文件离线分发**：内嵌全部资源与 http(s) 外链图片，导出 HTML 断网可看。
- **多语言界面**：13 种界面语言（含阿拉伯语 RTL）；界面与增量辅助控件多语，页面正文语种由素材决定，程序不翻译正文。13 种词条内嵌于 `src/i18n.js`（避免 `file://` 下 fetch 被阻止）；`src/locales/zh-CN.json` 是中文基准包的同步副本，不参与运行时加载。
- **并行处理**：rayon 按核心数并行处理页面转换与媒体解码。

## 运行

方式一：下载 Release 附件中的可执行文件，直接运行（绿色版，无需安装）。

方式二：从源码构建后运行（见下节）。

## 从源码构建

前置工具链：

- Rust（ edition 2021；Tauri 2.x）
- Tauri v2 桌面依赖（Windows 需 WebView2、MSVC 构建工具）
- Node.js（仅用于 npm 脚本入口；前端为原生 JS，无框架构建步骤）

步骤：

```powershell
cd src-tauri
cargo build --release
```

开发模式运行：

```powershell
cargo tauri dev
```

> `src-tauri/resources/pdfium.dll` 为 PDF 保真渲染动态库，须与可执行文件同目录部署；其许可见 `src-tauri/resources/pdfium-LICENSE.txt`。

## 测试

```powershell
cd src-tauri
cargo test
```

现查 `#[test]` 用例 50 个（锚点见 `发布准备-现状锚点登记-20261121.md`）。

## 目录结构

```
src/                 前端（原生 JS/HTML/CSS，i18n 内嵌语言包）
src-tauri/           Rust 后端（Tauri v2）
  src/               转换、打包、MHTML、命令层
  examples/          本地测试与矩阵审计工具（非随库发布物）
  resources/         pdfium.dll 及其许可
  capabilities/      桌面权限配置
  icons/             应用图标
_tools/              测试素材生成脚本（开发辅助，非随库发布物）
```

随库白名单与排除清单见 `发布准备-随库白名单清单-20261121.md` 等准备件。

## 许可

本项目 MIT 许可，详见 `LICENSE`。第三方组件 pdfium 采用 BSD-3-Clause，全文见 `src-tauri/resources/pdfium-LICENSE.txt`。
