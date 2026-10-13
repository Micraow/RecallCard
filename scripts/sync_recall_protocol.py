#!/usr/bin/env python3
"""从 crate 内唯一能力定义生成扩展顶层模块；--check 只读校验。"""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/recallcard/src/recall-capabilities.json"
TARGET = ROOT / "extension/recall-capabilities.js"


def expected():
    value = json.loads(SOURCE.read_text(encoding="utf-8"))
    assert value["version"] == "recallcard.read-semantics/1"
    assert set(value["actions"]) == {"search", "fetch", "final"}
    return ("// 由 scripts/sync_recall_protocol.py 生成；请修改 crate 内 recall-capabilities.json。\n"
            + "export const RECALL_CAPABILITIES = "
            + json.dumps(value, ensure_ascii=False, indent=2) + ";\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    content = expected()
    if not TARGET.exists() or TARGET.read_text(encoding="utf-8") != content:
        if args.check:
            raise SystemExit("读取能力定义不同步；请运行 python3 scripts/sync_recall_protocol.py")
        TARGET.write_text(content, encoding="utf-8")
    print("读取能力定义一致")


if __name__ == "__main__":
    main()
