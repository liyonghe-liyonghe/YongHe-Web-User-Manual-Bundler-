# -*- coding: utf-8 -*-
"""GitHub Release 资产装配器（泳和-网页用户手册合成器）。

承发布准备方案 §C →G2/G3/G4/G5/G13/G15：
  - 基于脚本自身位置解析项目根，零盘符绝对路径依赖（B14）；
  - 产两个输出目录：源码装配目录 release-src-<ts> 与成品附件目录
    release-assets-<ts>，二者共用同一运行时间戳字符串（B2/B3/B5）；
  - 运行 rc=0，末行打印 GITHUB-ASSETS: OK（G4）；
  - 二次运行先清理旧族再重建，件数一致、不累积（B15/G15）；
  - 成品目录含四要素完整性清单（文件名/字节数/SHA-256/手册名），
    哈希按 sha256 现算、与原件同算法同值（B13/G13）；
  - 随库白名单与四类排除清单一律读自同目录 github_release_manifest.json，
    本脚本不内嵌任何随库判据，确保白名单单一事实源（B8/B9）。

用法：python make_github_release_assets.py
"""
import hashlib
import json
import os
import shutil
import sys
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent
MANIFEST = ROOT / "github_release_manifest.json"
SRC_PREFIX = "release-src-"
ASSET_PREFIX = "release-assets-"
ANCHOR = "GITHUB-ASSETS"


def msg(text):
    print(text, flush=True)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_manifest():
    with open(MANIFEST, "r", encoding="utf-8") as f:
        return json.load(f)


def clean_old_families():
    removed = []
    for prefix in (SRC_PREFIX, ASSET_PREFIX):
        for p in ROOT.glob(prefix + "*"):
            if p.is_dir():
                shutil.rmtree(p)
                removed.append(p.name)
    return removed


def copy_item(rel, dst_root):
    src = ROOT / rel
    dst = dst_root / rel
    if not src.exists():
        raise FileNotFoundError("随库件缺失：" + rel)
    if src.is_dir():
        shutil.copytree(src, dst, dirs_exist_ok=True)
        return sum(1 for _ in src.rglob("*") if _.is_file())
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    return 1


def build_src_tree(mf, src_root):
    count = 0
    manifest = load_manifest()
    for item in manifest["whitelist"]:
        count += copy_item(item["path"], src_root)
    # 副本 .gitignore = 开发树忽略规则（若存在）+ manifest 追加项
    lines = []
    dev_gitignore = ROOT / ".gitignore"
    if dev_gitignore.is_file():
        lines.extend(dev_gitignore.read_text(encoding="utf-8").splitlines())
    lines.append("")
    lines.append("# ---- 发布装配副本追加项（源自 github_release_manifest.json）----")
    lines.extend(manifest["gitignore_append"])
    (src_root / ".gitignore").write_text("\n".join(lines) + "\n", encoding="utf-8")
    count += 1
    return count


def build_assets_tree(mf, asset_root):
    exe = ROOT / mf["release_exe"]
    if not exe.is_file():
        raise FileNotFoundError("Release 主件缺失：" + mf["release_exe"])
    copied = asset_root / mf["release_exe"]
    shutil.copy2(exe, copied)
    rows = [
        ("文件名", "字节数", "SHA-256", "手册名"),
        (mf["release_exe"], str(exe.stat().st_size), sha256_file(copied), mf["manual_name"]),
    ]
    lines = [
        "# 发布资产完整性清单",
        "",
        "- 手册名：" + mf["manual_name"],
        "- 公开标签：" + mf["release_tag"],
        "- 现算时刻：" + datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
        "- 哈希算法：SHA-256（与原件同算法同值，逐块现算）",
        "",
        "| 文件名 | 字节数 | SHA-256 | 手册名 |",
        "|---|---|---|---|",
    ]
    for r in rows[1:]:
        lines.append("| " + " | ".join(r) + " |")
    (asset_root / "完整性清单.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 2, sha256_file(copied)


def main():
    mf = load_manifest()
    ts = datetime.now().strftime("%Y%m%d-%H%M%S")
    src_root = ROOT / (SRC_PREFIX + ts)
    asset_root = ROOT / (ASSET_PREFIX + ts)

    removed = clean_old_families()
    msg("[INFO] 旧族清理：" + (", ".join(removed) if removed else "无"))
    src_root.mkdir(parents=True)
    asset_root.mkdir(parents=True)

    src_count = build_src_tree(mf, src_root)
    asset_count, exe_sha = build_assets_tree(mf, asset_root)

    msg("[INFO] 源码装配目录：" + src_root.name + "（" + str(src_count) + " 件）")
    msg("[INFO] 成品附件目录：" + asset_root.name + "（" + str(asset_count) + " 件）")
    msg("[INFO] 共用时间戳：" + ts)
    msg("[INFO] Release 主件 SHA-256：" + exe_sha)
    msg(ANCHOR + ": OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
