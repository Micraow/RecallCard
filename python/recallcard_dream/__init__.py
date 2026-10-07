"""RecallCard 可选 Dream API；所有网络操作均需显式授权。"""
from .client import (
    DreamClient, DreamError, NetworkApproval, TransportResponse,
    PROMPT_TEMPLATE_HASH, STABLE_PREFIX_BYTES, load_json, validate_job, validate_result,
)

__all__ = ["DreamClient", "DreamError", "NetworkApproval", "TransportResponse", "PROMPT_TEMPLATE_HASH",
           "STABLE_PREFIX_BYTES", "load_json", "validate_job", "validate_result"]
