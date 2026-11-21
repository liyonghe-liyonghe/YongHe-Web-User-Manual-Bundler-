# -*- coding: utf-8 -*-
"""
语言-编码测试矩阵生成器：
为 6 种语言 × 2 种主流编码生成 11 种文档格式（pdf/doc/docx/rtf/wps/odt/odp/ppt/pptx/md/txt）
的测试文件，放入 MB_DOCS_DIR 环境变量指定目录下的 <lang>-<encoding> 文件夹。

- 二进制容器格式（pdf/doc/docx/wps/odt/odp/ppt/pptx）每语言生成一份，复制到该语言的两个编码文件夹
- 纯文本格式（txt/md/rtf）按 语言-编码 各自独立生成真实编码字节
"""
import os, shutil, sys, json, time

DOCS = os.environ.get("MB_DOCS_DIR") or os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "_test_out", "docs")
OUT_LOG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gen_result.json")

LANGS = [
    dict(code="zh-CN", codec="gbk",      enc2="GBK",          folder2="zh-CN-GBK",          marker="简体中文样式矩阵",
         word_font="SimSun", fcharset=134, ansicpg=936,
         sub1="粗体斜体下划线混合", sub2="红色与蓝色文字", li=["基础功能", "编码支持", "多语言渲染"],
         tbl=[["列一", "列二"], ["数据甲", "数据乙"]], txt1="纯文本第一行内容", txt2="第二行结束",
         rtf1="富文本格式行", font_name="SimSun"),
    dict(code="zh-TW", codec="big5",     enc2="Big5",         folder2="zh-TW-Big5",         marker="繁體中文樣式矩陣",
         word_font="PMingLiU", fcharset=136, ansicpg=950,
         sub1="粗體斜體底線混合", sub2="紅色與藍色文字", li=["基礎功能", "編碼支援", "多語言呈現"],
         tbl=[["欄一", "欄二"], ["資料甲", "資料乙"]], txt1="純文字第一行內容", txt2="第二行結束",
         rtf1="富文字格式行", font_name="PMingLiU"),
    dict(code="ja-JP", codec="shift_jis", enc2="Shift-JIS",  folder2="ja-JP-Shift-JIS",    marker="日本語スタイルマトリクス",
         word_font="MS Mincho", fcharset=128, ansicpg=932,
         sub1="太字・斜体・下線の混合", sub2="赤色と青色の文字", li=["基本機能", "エンコード対応", "多言語表示"],
         tbl=[["列一", "列二"], ["データ甲", "データ乙"]], txt1="テキスト一行目内容", txt2="二行目終了",
         rtf1="リッチテキスト行", font_name="MS Mincho"),
    dict(code="ko-KR", codec="euc_kr",   enc2="EUC-KR",       folder2="ko-KR-EUC-KR",       marker="한국어 스타일 매트릭스",
         word_font="Batang", fcharset=129, ansicpg=949,
         sub1="굵게 기울임 밑줄 혼합", sub2="빨강과 파랑 글자", li=["기본 기능", "인코딩 지원", "다국어 표시"],
         tbl=[["열 하나", "열 둘"], ["데이터 갑", "데이터 을"]], txt1="일반 텍스트 첫 줄 내용", txt2="둘째 줄 끝",
         rtf1="서식 있는 텍스트 줄", font_name="Batang"),
    dict(code="en-US", codec="cp1252",   enc2="Windows-1252", folder2="en-US-Windows-1252", marker="English Style Matrix",
         word_font="Arial", fcharset=0, ansicpg=1252,
         sub1="Bold Italic Underline Mix", sub2="Red and Blue Text", li=["Basic feature", "Encoding support", "Multilingual display"],
         tbl=[["Column One", "Column Two"], ["Data A", "Data B"]], txt1="Plain text first line content", txt2="Second line end",
         rtf1="Rich text format line", font_name="Arial"),
    dict(code="ru-RU", codec="cp1251",   enc2="Windows-1251", folder2="ru-RU-Windows-1251", marker="Русская матрица стилей",
         word_font="Times New Roman", fcharset=204, ansicpg=1251,
         sub1="Жирный курсив подчёркивание", sub2="Красный и синий текст", li=["Базовая функция", "Поддержка кодировок", "Многоязычный показ"],
         tbl=[["Колонка один", "Колонка два"], ["Данные А", "Данные Б"]], txt1="Обычный текст первая строка", txt2="Вторая строка конец",
         rtf1="Строка форматированного текста", font_name="Times New Roman"),
]

FMTS = ["PDF", "DOC", "DOCX", "RTF", "WPS", "ODT", "ODP", "PPT", "PPTX", "MD", "TXT"]
result = {"folders": [], "files": 0, "errors": []}


def fname(lang, fmt):
    return "Test_File_1234567890-%s.%s" % (fmt, fmt.lower())


def body_lines(lang):
    L = lang
    return [
        L["marker"] + " 极限版",
        L["sub1"],
        L["sub2"],
        "· " + L["li"][0],
        "· " + L["li"][1],
        "· " + L["li"][2],
        L["tbl"][0][0] + " | " + L["tbl"][0][1],
        L["tbl"][1][0] + " | " + L["tbl"][1][1],
    ]


# ---------- 1. txt / md / rtf：真实编码字节 ----------

def rtf_escape(text, codec):
    if codec == "utf-8":
        # 现代 Word 风格：文件保持纯 ASCII，非 ASCII 用 \uNNNN + 兜底符
        out = bytearray()
        for ch in text:
            if ord(ch) < 0x80:
                out.append(ord(ch))
            else:
                out.extend(b"\\u%d ?" % ord(ch))
        return bytes(out)
    out = bytearray()
    for ch in text:
        if ord(ch) < 0x80:
            if ch in "\\{}":
                out.extend(b"\\" + ch.encode("ascii"))
            else:
                out.append(ord(ch))
        else:
            # 非 ASCII 字符的全部编码字节统一写 \'xx（真实 CJK RTF 写法）
            for byte in ch.encode(codec, errors="replace"):
                out.extend(b"\\'%02x" % byte)
    return bytes(out)


def gen_text_files(lang, folder, codec):
    L = lang
    # TXT
    txt = "\r\n".join([L["marker"], L["txt1"], L["txt2"], "TXT/MD/RTF encoding test"]) + "\r\n"
    open(os.path.join(folder, fname(L, "TXT")), "wb").write(txt.encode(codec))
    # MD
    md = "\n".join([
        "# " + L["marker"],
        "",
        "**" + L["sub1"] + "** and *" + L["sub2"] + "*",
        "",
        "| " + L["tbl"][0][0] + " | " + L["tbl"][0][1] + " |",
        "| --- | --- |",
        "| " + L["tbl"][1][0] + " | " + L["tbl"][1][1] + " |",
        "",
        "- " + L["li"][0],
        "- " + L["li"][1],
        "- " + L["li"][2],
        "",
    ])
    open(os.path.join(folder, fname(L, "MD")), "wb").write(md.encode(codec))
    # RTF：ansicpg 声明 + \'xx 转义字节
    parts = []
    parts.append(b"{\\rtf1\\ansi\\ansicpg%d\\deff0{\\fonttbl{\\f0\\fcharset%d %s;}}" % (
        L["ansicpg"], L["fcharset"], L["word_font"].encode("ascii")))
    parts.append(rtf_escape("\\fs24\\qc " + L["marker"] + " - RTF\\par", codec))
    parts.append(rtf_escape("\\b " + L["sub1"] + "\\b0  \\i " + L["sub2"] + "\\i0\\par", codec))
    parts.append(rtf_escape("\\tab " + L["rtf1"] + "\\par", codec))
    parts.append(rtf_escape(L["li"][0] + " / " + L["li"][1] + " / " + L["li"][2] + "\\par", codec))
    parts.append(rtf_escape(L["tbl"][1][0] + " | " + L["tbl"][1][1] + "\\par", codec))
    parts.append(b"}")
    open(os.path.join(folder, fname(L, "RTF")), "wb").write(b"\n".join(parts))


# ---------- 2. docx（python-docx） ----------

def gen_docx(lang, folder):
    from docx import Document
    from docx.shared import RGBColor, Pt
    L = lang
    d = Document()
    d.add_heading(L["marker"] + " - DOCX", level=1)
    p = d.add_paragraph()
    r = p.add_run(L["sub1"]); r.bold = True; r.italic = True; r.underline = True
    r = p.add_run("  " + L["sub2"]); r.font.color.rgb = RGBColor(0xC0, 0x00, 0x00); r.font.size = Pt(14)
    for item in L["li"]:
        d.add_paragraph(item, style="List Bullet")
    t = d.add_table(rows=2, cols=2); t.style = "Table Grid"
    t.cell(0, 0).text = L["tbl"][0][0]; t.cell(0, 1).text = L["tbl"][0][1]
    t.cell(1, 0).text = L["tbl"][1][0]; t.cell(1, 1).text = L["tbl"][1][1]
    d.add_paragraph(L["txt1"] + " / " + L["txt2"])
    d.save(os.path.join(folder, fname(L, "DOCX")))


# ---------- 3. odt / odp（odfpy） ----------

def gen_odt(lang, folder):
    from odf.opendocument import OpenDocumentText
    from odf.style import Style, TextProperties, ParagraphProperties
    from odf.text import H, P, Span
    from odf.table import Table, TableRow, TableCell
    L = lang
    d = OpenDocumentText()
    bold = Style(name="TBold", family="text")
    bold.addElement(TextProperties(fontweight="bold"))
    d.automaticstyles.addElement(bold)
    center = Style(name="PCtr", family="paragraph")
    center.addElement(ParagraphProperties(textalign="center"))
    d.automaticstyles.addElement(center)
    d.text.addElement(H(outlinelevel=1, text=L["marker"] + " - ODT"))
    p = P(); p.addElement(Span(stylename=bold, text=L["sub1"])); p.addText(" / " + L["sub2"])
    d.text.addElement(p)
    pc = P(stylename=center, text=L["txt1"])
    d.text.addElement(pc)
    t = Table(name="Matrix")
    for row in L["tbl"]:
        tr = TableRow()
        for cell in row:
            tc = TableCell(valuetype="string")
            tc.addElement(P(text=cell))
            tr.addElement(tc)
        t.addElement(tr)
    d.text.addElement(t)
    d.text.addElement(P(text=L["txt2"]))
    d.save(os.path.join(folder, fname(L, "ODT")))


def gen_odp(lang, folder):
    from odf.opendocument import OpenDocumentPresentation
    from odf.style import Style, MasterPage, PageLayout, PageLayoutProperties, TextProperties, GraphicProperties, ParagraphProperties
    from odf.draw import Page, Frame, TextBox
    from odf.text import P
    L = lang
    d = OpenDocumentPresentation()
    pagelayout = PageLayout(name="PL1")
    pagelayout.addElement(PageLayoutProperties(margin="0pt", pagewidth="794pt", pageheight="595pt", printorientation="landscape"))
    d.automaticstyles.addElement(pagelayout)
    masterpage = MasterPage(name="Standard", pagelayoutname=pagelayout)
    d.masterstyles.addElement(masterpage)
    gstyle = Style(name="GBox", family="graphic")
    gstyle.addElement(GraphicProperties(fillcolor="#ffffff", stroke="none"))
    d.automaticstyles.addElement(gstyle)
    tstyle = Style(name="TTitle", family="presentation")
    tstyle.addElement(TextProperties(fontsize="28pt", fontweight="bold"))
    tstyle.addElement(ParagraphProperties(textalign="center"))
    d.automaticstyles.addElement(tstyle)
    page = Page(name="page1", masterpagename=masterpage)
    d.presentation.addElement(page)
    fr = Frame(stylename=gstyle, width="600pt", height="80pt", x="97pt", y="60pt")
    tb = TextBox(); tb.addElement(P(text=L["marker"] + " - ODP"))
    fr.addElement(tb); page.addElement(fr)
    fr2 = Frame(stylename=gstyle, width="600pt", height="200pt", x="97pt", y="180pt")
    tb2 = TextBox()
    for line in [L["sub1"], L["sub2"], L["li"][0] + " / " + L["li"][1] + " / " + L["li"][2]]:
        tb2.addElement(P(text=line))
    fr2.addElement(tb2); page.addElement(fr2)
    d.save(os.path.join(folder, fname(L, "ODP")))


# ---------- 4. pdf / doc（Word COM） ----------

def gen_word(lang, folder):
    import win32com.client
    import pythoncom
    pythoncom.CoInitialize()
    L = lang
    word = win32com.client.DispatchEx("Word.Application")
    word.Visible = False
    word.DisplayAlerts = 0
    doc = None
    try:
        doc = word.Documents.Add()
        rng = doc.Range()
        rng.Text = "\r".join(body_lines(L)) + "\r"
        rng.Font.Size = 14
        rng.Font.Name = L["font_name"]
        rng.Font.NameFarEast = L["word_font"]
        # 标题行加粗放大
        h = doc.Range(0, len(body_lines(L)[0].encode("utf-16-le")) // 2)
        h.Font.Size = 20; h.Font.Bold = True
        pdf_path = os.path.join(folder, fname(L, "PDF"))
        try:
            doc.SaveAs2(pdf_path, FileFormat=17)
        except Exception:
            doc.ExportAsFixedFormat(pdf_path, 17)
        doc.SaveAs2(os.path.join(folder, fname(L, "DOC")), FileFormat=0)
    finally:
        try:
            if doc is not None: doc.Close(False)
        except Exception: pass
        try: word.Quit()
        except Exception: pass
        pythoncom.CoUninitialize()


# ---------- 5. ppt / pptx（PowerPoint COM） ----------

def gen_ppt(lang, folder):
    import win32com.client
    import pythoncom
    pythoncom.CoInitialize()
    L = lang
    app = win32com.client.DispatchEx("PowerPoint.Application")
    try:
        app.Visible = True
        app.WindowState = 2  # 最小化，减少干扰
    except Exception:
        pass
    pres = None
    try:
        pres = app.Presentations.Add()
        slide = pres.Slides.Add(1, 2)  # ppLayoutText
        slide.Shapes.Title.TextFrame.TextRange.Text = L["marker"] + " - PPT"
        body = pres.Slides(1).Shapes.Placeholders(2).TextFrame.TextRange
        body.Text = "\r".join([L["sub1"], L["sub2"], L["li"][0], L["li"][1], L["li"][2],
                               L["tbl"][1][0] + " / " + L["tbl"][1][1]])
        try:
            pres.SaveAs(os.path.join(folder, fname(L, "PPTX")))
        except Exception:
            pres.SaveAs(os.path.join(folder, fname(L, "PPTX")), 24)
        pres.SaveAs(os.path.join(folder, fname(L, "PPT")), 1)  # ppSaveAsPresentation
    finally:
        try:
            if pres is not None: pres.Close()
        except Exception: pass
        try: app.Quit()
        except Exception: pass
        pythoncom.CoUninitialize()


# ---------- 6. wps（WPS 文字 COM，探测保存格式） ----------

def gen_wps(lang, folder):
    import win32com.client
    import pythoncom
    pythoncom.CoInitialize()
    L = lang
    app = win32com.client.DispatchEx("Kwps.Application")
    try: app.Visible = False
    except Exception: pass
    doc = None
    try:
        doc = app.Documents.Add()
        doc.Content.Text = "\r".join(body_lines(L)) + "\r"
        path = os.path.join(folder, fname(L, "WPS"))
        saved = False
        for ff in (0, 4096, 1, 2, 12):
            try:
                try: doc.SaveAs2(path, FileFormat=ff)
                except Exception: doc.SaveAs(path, FileFormat=ff)
                with open(path, "rb") as f: magic = f.read(4)
                if magic in (b"PK\x03\x04", b"\xd0\xcf\x11\xe0"):
                    print("  [wps] %s FileFormat=%s -> magic=%s OK" % (L["code"], ff, magic.hex()))
                    saved = True
                    break
            except Exception as e:
                print("  [wps] %s FileFormat=%s fail: %s" % (L["code"], ff, str(e)[:60]))
        if not saved:
            raise RuntimeError("WPS 无法保存可识别格式")
    finally:
        try:
            if doc is not None: doc.Close(False)
        except Exception: pass
        try: app.Quit()
        except Exception: pass
        pythoncom.CoUninitialize()


# ---------- 主流程 ----------

def main():
    only = sys.argv[1] if len(sys.argv) > 1 else "all"
    stages = {"text": gen_text_files, "docx": gen_docx, "odt": gen_odt, "odp": gen_odp,
              "word": gen_word, "ppt": gen_ppt, "wps": gen_wps}
    if only == "all":
        stages_list = list(stages.items())
    else:
        stages_list = [(only, stages[only])]

    for lang in LANGS:
        f_utf = os.path.join(DOCS, "%s-UTF8" % lang["code"])
        f_enc = os.path.join(DOCS, lang["folder2"])
        for f in (f_utf, f_enc):
            os.makedirs(f, exist_ok=True)
        result["folders"] += [f_utf, f_enc]
        print("== %s ==" % lang["code"])
        for name, fn in stages_list:
            t0 = time.time()
            try:
                if name == "text":
                    fn(lang, f_utf, "utf-8")   # UTF8 文件夹 → UTF-8 字节
                    fn(lang, f_enc, lang["codec"])  # 编码文件夹 → 对应旧编码字节
                else:
                    fn(lang, f_utf)
                    for fmt in ("PDF", "DOC", "DOCX", "WPS", "ODT", "ODP", "PPT", "PPTX"):
                        src = os.path.join(f_utf, fname(lang, fmt))
                        if os.path.exists(src):
                            shutil.copy2(src, os.path.join(f_enc, os.path.basename(src)))
                result["errors"].append(None)
                print("  [OK] %-5s (%.1fs)" % (name, time.time() - t0))
            except Exception as e:
                result["errors"].append("%s/%s: %s" % (lang["code"], name, e))
                print("  [FAIL] %-5s %s" % (name, str(e)[:200]))

    # 统计
    total = 0
    for lang in LANGS:
        for f in ("%s-UTF8" % lang["code"], lang["folder2"]):
            n = len([x for x in os.listdir(os.path.join(DOCS, f)) if x.endswith((".pdf", ".doc", ".docx", ".rtf", ".wps", ".odt", ".odp", ".ppt", ".pptx", ".md", ".txt"))])
            result["folders"].append(f)
            total += n
            print("%-22s %d files" % (f, n))
    result["files"] = total
    json.dump({k: v for k, v in result.items()}, open(OUT_LOG, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print("TOTAL:", total)
    if any(result["errors"]):
        print("ERRORS:", [e for e in result["errors"] if e])
        sys.exit(1)


if __name__ == "__main__":
    main()
