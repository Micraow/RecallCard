# RecallCard 可选 Python worker

仅使用 Python 3.10+ 标准库。默认不联网，不安装模型，不读取 Vault；主程序离线功能无需 Python。

完整协议、批准外发的参数、增量检查点、限额和验证边界见 [向量 worker 文档](../docs/embedding.md)。

```sh
# 从仓库根目录运行；全部测试使用内存 fake transport
python3 -m unittest discover -s python/tests -v

# 离线 JSONL 入口；没有显式授权参数就不能调用供应商
PYTHONPATH=python python3 -m recallcard_worker --stdio
```

API key 只能由环境变量提供，不能提交进仓库或写入 Vault。生产使用前先让用户批准具体接收服务、数据与范围；测试不需要有效 key。
