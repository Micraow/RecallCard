import { useEffect, useRef, type ReactNode } from 'react';
import { Icon, type IconName } from './Icon';
import type { AppError } from '../service/contracts';

export function ErrorNotice({ error, retry }: { error: AppError; retry?: () => void }) {
  return <div className="error-notice" role="alert"><Icon name="alert" /><div><strong>{error.message}</strong>{error.file_name && <p>{error.file_name}{error.member ? ` / ${error.member}` : ''}</p>}{error.action && <p>{error.action}</p>}{error.committed_events > 0 && <p>已保存 {error.committed_events.toLocaleString()} 条消息，已保存的内容会保留。</p>}<details><summary>诊断信息</summary><span>错误代码：{error.code}</span></details></div>{retry && <button className="button compact" onClick={retry}>重试</button>}</div>;
}
export function EmptyState({ icon, title, children, action }: { icon: IconName; title: string; children: ReactNode; action?: ReactNode }) {
  return <div className="empty-state"><span className="empty-icon"><Icon name={icon} size={26} /></span><h2>{title}</h2><div className="empty-copy">{children}</div>{action && <div className="empty-action">{action}</div>}</div>;
}
export function Loading({ label = '正在读取本机资料' }: { label?: string }) { return <div className="loading-state" role="status"><span className="spinner" />{label}…</div>; }
export function Badge({ children, tone = 'muted' }: { children: ReactNode; tone?: 'muted' | 'success' | 'warning' | 'accent' | 'danger' }) { return <span className={`badge ${tone}`}>{children}</span>; }
export function Dialog({ title, children, close, busy = false, wide = false }: { title: string; children: ReactNode; close: () => void; busy?: boolean; wide?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.showModal();
    return () => { ref.current?.close(); previous?.focus(); };
  }, []);
  return <dialog ref={ref} className={wide ? 'modal wide' : 'modal'} aria-labelledby="dialog-title" onCancel={event => { event.preventDefault(); if (!busy) close(); }} onClick={event => { if (event.target === ref.current && !busy) close(); }}><div className="modal-header"><h2 id="dialog-title">{title}</h2><button className="icon-button" aria-label="关闭" onClick={close} disabled={busy}><Icon name="close" /></button></div>{children}</dialog>;
}
export function Prose({ text }: { text: string }) {
  // 原文以文本节点显示，导入内容从不成为 HTML 或可执行链接。
  return <div className="prose">{text.split(/\n\s*\n/).map((block, index) => <p key={index}>{block}</p>)}</div>;
}
