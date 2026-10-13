// 仅纯 JSON 语法工具；不访问网站、存储、剪贴板或权限。
const encoder = new TextEncoder();
export const byteLength = text => encoder.encode(text).length;
export const isRecord = value => value !== null && typeof value === 'object' && !Array.isArray(value);
export function fail(message) { throw new Error(message); }
const MAX_ACTION_BYTES = 16 * 1024;
// JSON.parse alone silently accepts duplicate keys. This syntax walk rejects them,
// including escaped spellings, before the parsed object reaches any action handler.
export function strictJson(text) {
  if (typeof text !== 'string' || byteLength(text) > MAX_ACTION_BYTES) fail('请求超过 16 KiB 或不是文本');
  let parsed;
  try { parsed = JSON.parse(text); } catch { fail('JSON 不完整或格式无效'); }
  let at = 0;
  const space = () => { while (/\s/u.test(text[at] || '') && at < text.length) at++; };
  const string = () => {
    const start = at++;
    while (at < text.length) {
      const char = text[at++];
      if (char === '\\') at++;
      else if (char === '"') return JSON.parse(text.slice(start, at));
    }
    fail('JSON 字符串不完整');
  };
  const walk = (depth) => {
    if (depth > 12) fail('JSON 嵌套过深');
    space();
    if (text[at] === '{') {
      at++; space();
      const seen = new Set();
      while (text[at] !== '}') {
        const key = string();
        if (seen.has(key) || ['__proto__', 'prototype', 'constructor'].includes(key)) fail('JSON 包含重复或不安全字段');
        seen.add(key); space(); at++; walk(depth + 1); space();
        if (text[at] !== ',') break;
        at++; space();
      }
      at++;
    } else if (text[at] === '[') {
      at++; space();
      while (text[at] !== ']') {
        walk(depth + 1); space();
        if (text[at] !== ',') break;
        at++; space();
      }
      at++;
    } else if (text[at] === '"') string();
    else { while (at < text.length && !/[\s,}\]]/u.test(text[at])) at++; }
  };
  walk(0);
  return parsed;
}
