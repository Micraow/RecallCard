import assert from 'node:assert/strict';
import test from 'node:test';
import { appError, isRunning, validateJobs } from '../src/service/contracts.ts';
import { dateLabel, eventText, evidenceLabel, scopeLabel } from '../src/service/types.ts';

test('应用错误保留原因、来源成员、恢复动作和提交边界', () => {
  const error = appError({ code: 'invalid_json', message: '会话不完整', action: '重新下载', file_name: 'synthetic.zip', member: 'conversations.json', retryable: true, committed_events: 32 });
  assert.equal(error.code, 'invalid_json'); assert.equal(error.member, 'conversations.json'); assert.equal(error.committed_events, 32); assert.equal(error.action, '重新下载');
});
test('旧服务字符串错误不被泛化成没有原因的通知', () => {
  assert.equal(appError('当前资料范围已失效').message, '当前资料范围已失效');
  assert.equal(appError(new Error('需要重新读取')).message, '需要重新读取');
});
test('不兼容任务合同明确失败，不当作零任务', () => {
  assert.throws(() => validateJobs({ jobs: [] }), /格式不受支持/);
  assert.throws(() => validateJobs([{ schema: 'old', job_id: 'synthetic', state: 'completed', progress: {} }]), /合同版本不一致/);
  assert.deepEqual(validateJobs([]), []);
});
test('未知任务状态不伪造完成', () => {
  assert.throws(() => validateJobs([{ schema: 'recallcard.application-job/1', job_id: 'synthetic', state: 'interrupted', progress: {} }]), /合同版本不一致/);
});
test('暂停与需要输入不是后台正在运行', () => {
  assert.equal(isRunning({ state: 'paused' } as any), false);
  assert.equal(isRunning({ state: 'needs_input' } as any), false);
  assert.equal(isRunning({ state: 'queued' } as any), true);
});
test('未知时间不使用当前导入时间补写', () => {
  assert.equal(dateLabel(null), '时间未知');
  assert.equal(dateLabel('invalid'), '时间未知');
});
test('来源正文按内容片段读取而不执行 HTML', () => {
  assert.equal(eventText({ content: '<script>unsafe</script>', parts: [{ text: '<script>raw</script>' }, { text: '正文' }] } as any), '<script>raw</script>\n正文');
});
test('观察和 AI 建议不标为明确事实，个人和项目标签分开', () => {
  assert.equal(evidenceLabel('assistant_suggestion'), 'AI 建议');
  assert.equal(evidenceLabel('observed'), '观察所得');
  assert.equal(scopeLabel('personal'), '个人空间');
  assert.equal(scopeLabel('project:atlas'), 'atlas');
});
