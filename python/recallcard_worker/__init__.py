"""RecallCard 可选标准库 worker；Rust 主程序无需依赖此模块。"""
from .embedding import (
    EmbeddingClient, EmbeddingError, EmbeddingSpace, NetworkApproval,
    index_corpus, reciprocal_rank_fusion, search_index,
)

__all__ = ["EmbeddingClient", "EmbeddingError", "EmbeddingSpace", "NetworkApproval",
           "index_corpus", "reciprocal_rank_fusion", "search_index"]
