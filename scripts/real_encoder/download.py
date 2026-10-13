"""仅 CI 下载已固定官方模型；总字节、单次请求和总时限均受限。"""
import argparse
from pathlib import Path
import shutil
import time
import urllib.request

from common import HERE, read_json, require_ci, save_json, verify_file


def download(destination):
    require_ci()
    lock = read_json(HERE / "model-lock.json")
    if sum(v["bytes"] for v in lock["files"].values()) > 100 * 1024 * 1024:
        raise RuntimeError("模型下载预算超过 100 MiB")
    destination.mkdir(parents=True, exist_ok=False)
    if shutil.disk_usage(destination).free < 2 * 1024**3:
        raise RuntimeError("CI 磁盘余量不足 2 GiB")
    started = time.monotonic()
    checked = {}
    for name, expected in lock["files"].items():
        if Path(name).name != name:
            raise RuntimeError("固定文件名不合法")
        url = f"https://huggingface.co/{lock['repository']}/resolve/{lock['revision']}/{name}"
        path = destination / name
        received = 0
        with urllib.request.urlopen(url, timeout=30) as response, path.open("xb") as stream:
            while block := response.read(1024 * 1024):
                received += len(block)
                if received > expected["bytes"] or time.monotonic() - started > 180:
                    raise RuntimeError("模型下载超出字节或 180 秒时限")
                stream.write(block)
        checked[name] = verify_file(path, expected)
    save_json(destination / "verified-download.json", {"files": checked, "seconds": time.monotonic() - started,
              "download_bytes": sum(v["bytes"] for v in checked.values()), "model_revision": lock["revision"]})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    download(parser.parse_args().output)
