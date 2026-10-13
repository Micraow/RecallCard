// text/2 的纯语法层。解析成功不代表来源、完成、发送或权限已核验。
// 此模块不执行工具、不读取存储、不操作网页；宿主仍须检查活动轮次及授权。
import { strictJson, byteLength, isRecord, fail } from './strict-json.js';
import { RECALL_CAPABILITIES } from './recall-capabilities.js';
export { RECALL_CAPABILITIES } from './recall-capabilities.js';
const TICKET = /^T_[A-Za-z0-9_-]{16}$/u;
const ALIAS = /^[SP][1-9][0-9]{0,5}$/u;
const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u;
const UNPAIRED_SURROGATE = /[\ud800-\udfff]/u;
export const textHeader = ticket => {
  if (!TICKET.test(ticket)) fail('本轮传输票据无效');
  return `RECALLCARD/2 ${ticket}`;
};
export function newTextTicket() {
  // 12 字节 = 96 位，不把票据视为授权凭据。
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
  const bytes = crypto.getRandomValues(new Uint8Array(12));
  let encoded = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | (bytes[i + 1] << 8) | bytes[i + 2];
    encoded += alphabet[(n >>> 18) & 63] + alphabet[(n >>> 12) & 63] + alphabet[(n >>> 6) & 63] + alphabet[n & 63];
  }
  return `T_${encoded}`;
}
export function parseTextFrame(text, expectedTicket) {
  if (typeof text !== 'string' || byteLength(text) > RECALL_CAPABILITIES.limits.web_final_bytes) fail('完整回复超过 64 KiB 或不是文本；没有截断执行');
  let normalized = text.replace(/\r\n/gu, '\n');
  if (normalized.endsWith('\n')) normalized = normalized.slice(0, -1);
  if (normalized.includes('\r') || CONTROL.test(normalized) || UNPAIRED_SURROGATE.test(normalized)) fail('回复包含未支持的控制字符或无效 Unicode');
  const lines = normalized.split('\n');
  const match = /^RECALLCARD\/2 (T_[A-Za-z0-9_-]{16})$/u.exec(lines[0]);
  if (!match || lines.at(-1) !== 'END RECALLCARD') fail('请提供一个完整的 RECALLCARD/2 助手回复，不加引用、代码围栏或帧外文字');
  const ticket = match[1];
  if (expectedTicket !== undefined && (!TICKET.test(expectedTicket) || ticket !== expectedTicket)) fail('回复不属于当前轮；旧票据不能重放');
  const data = strictJson(lines[1]);
  if (!isRecord(data) || typeof data.action !== 'string' || !Object.hasOwn(RECALL_CAPABILITIES.actions, data.action)) fail('只支持 search、fetch 或 final');
  const { action } = data;
  const required = action === 'search' ? ['action', 'query'] : action === 'fetch' ? ['action', 'id'] : ['action'];
  if (Object.keys(data).length !== required.length || required.some(key => !Object.hasOwn(data, key))) fail('动作字段必须精确匹配，不接受范围、权限、路径或预算参数');
  if (action !== 'final') {
    if (lines.length !== 3 || byteLength(text) > RECALL_CAPABILITIES.limits.web_action_bytes) fail('检索帧只能有三行且不超过 4 KiB');
    if (action === 'search' && (typeof data.query !== 'string' || !data.query.trim() || byteLength(data.query) > RECALL_CAPABILITIES.limits.web_query_bytes || /[\u0000-\u001f\u007f]/u.test(data.query) || UNPAIRED_SURROGATE.test(data.query))) fail('query 必须是 1–2048 UTF-8 字节的单行问题');
    if (action === 'fetch' && (typeof data.id !== 'string' || !ALIAS.test(data.id))) fail('fetch 只接受本轮已返回的 S/P 编号');
    return { ticket, ...data };
  }
  if (lines.length < 5 || lines[2] !== '') fail('final 动作后须空一行，再写非空回答正文');
  const body = lines.slice(3, -1).join('\n');
  if (!body.trim() || lines.slice(3, -1).some(line => line === 'END RECALLCARD' || /^RECALLCARD\/2(?:\s|$)/u.test(line))) fail('回答正文为空或包含多帧歧义；协议示例须缩进或引用');
  return { ticket, action, body };
}
export function webOpeningInstructions(ticket) {
  const spec = RECALL_CAPABILITIES;
  return [spec.common, spec.web,
    `本轮传输头：${textHeader(ticket)}`,
    '只能选择一项：\n{"action":"search","query":"具体问题或关键词"}\n{"action":"fetch","id":"S1"}\n{"action":"final"}',
    spec.web_evidence].join('\n\n');
}
export const mcpRecallInstructions = () => `${RECALL_CAPABILITIES.common}\n${RECALL_CAPABILITIES.mcp}`;
