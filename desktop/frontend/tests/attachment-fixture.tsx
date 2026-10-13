import { createRoot } from "react-dom/client";
import { MessageAssets } from "../src/pages/Sources";
import type { SourceAssets } from "../src/service/types";

// 仅用于浏览器验证的合成视图；不接入桌面服务、文件或网络附件。
const root = createRoot(document.getElementById("attachment-fixture")!);
export function renderAttachments(assets: SourceAssets) {
  root.render(<MessageAssets assets={assets} />);
}
