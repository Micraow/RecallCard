"""桌面宿主管理的单次云编码；批准来自启动参数，持久总预算由 Rust 宿主预留。"""
from .embedding import EmbeddingClient, EmbeddingSpace, MAX_TEXT_BYTES, _string, fail


class ManagedCloudClient(EmbeddingClient):
    """每个进程最多一个外部请求且不重试；不能由请求正文切换到此客户端。"""
    def __init__(self, approval, **kwargs):
        # 不能用 worker 重启清空总预算：宿主在启动之前完成独立持久预留。
        super().__init__(approval, max_network_calls=1, max_retries=0,
                         max_transmitted_bytes=4 * 1024 * 1024, **kwargs)

    def embed_documents(self, space: EmbeddingSpace, texts, requested_scopes):
        if not isinstance(texts, list) or not 1 <= len(texts) <= min(4, 8192 // space.dimensions):
            fail("input_limit", "每轮当前 Memory 编码数量或向量总量超过上限")
        if sum(len(_string(t, MAX_TEXT_BYTES, "当前 Memory").encode("utf-8")) for t in texts) > 65536:
            fail("input_limit", "本轮当前 Memory 编码字节超过上限")
        return self.embed(space, texts, requested_scopes, "corpus")
