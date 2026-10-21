import MarkdownIt from 'markdown-it';
import hljs from 'highlight.js/lib/common';

// html:false 让正文里的原始标签只能作为文本出现；链接统一新窗口打开。
const md = new MarkdownIt({ html: false, linkify: true, breaks: true, highlight: highlight });

function highlight(code: string, language: string): string {
  if (language && hljs.getLanguage(language)) {
    try {
      const value = hljs.highlight(code, { language, ignoreIllegals: true }).value;
      return `<pre class="hljs code-block"><code>${value}</code></pre>`;
    } catch {
      /* 退化为纯文本 */
    }
  }
  return `<pre class="hljs code-block"><code>${md.utils.escapeHtml(code)}</code></pre>`;
}

const defaultLinkOpen =
  md.renderer.rules.link_open ?? ((tokens, index, options, _env, self) => self.renderToken(tokens, index, options));

md.renderer.rules.link_open = (tokens, index, options, env, self) => {
  tokens[index].attrSet('target', '_blank');
  tokens[index].attrSet('rel', 'noreferrer noopener');
  return defaultLinkOpen(tokens, index, options, env, self);
};

export function Markdown({ text }: { text: string }) {
  return <div className="markdown" dangerouslySetInnerHTML={{ __html: md.render(text) }} />;
}

/// 列表里只要一行预览：去掉标记符号与多余空行。
export function plainPreview(text: string, limit = 160): string {
  const flattened = text
    .replace(/```[\s\S]*?```/g, '（代码）')
    .replace(/^\s*[-*+]\s+/gm, '')
    .replace(/^#{1,6}\s+/gm, '')
    .replace(/[`*_~]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
  return flattened.length > limit ? `${flattened.slice(0, limit)}…` : flattened;
}
