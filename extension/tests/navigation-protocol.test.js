// Recovery regression on the Oct 9 source: real protocol, synthetic requests.
// This is not a Native process or browser acceptance receipt.
import test from 'node:test';
import assert from 'node:assert/strict';
import { validateArguments, makeCapsule } from '../protocol.js';

test('目录根、保留入口与六层目录可从网页接力读取', () => {
  for (const ref of ['view:nav/_root', 'view:nav/_unfiled', 'view:nav/topics/linux/arch', `view:nav/${Array(5).fill('a'.repeat(48)).join('/')}/last`]) {
    const args = { refs: [ref], detail: 'compact', budget_bytes: 2048 };
    assert.equal(validateArguments('read', args), args);
  }
});
test('目录分页沿用同一引用、模式与游标，不冒充正文偏移', () => {
  for (const detail of ['compact', 'full']) {
    assert.doesNotThrow(() => validateArguments('read', { refs: ['view:nav/_root'], detail, cursor: 'synthetic:1' }));
  }
  for (const args of [
    { refs: ['view:nav/_root'], offset_bytes: 0 },
    { refs: ['view:nav/_root', 'view:nav/topics'], cursor: 'synthetic:1' },
    { refs: ['view:profile'], cursor: 'synthetic:1' },
    { refs: ['view:nav/_root'], detail: 'context' },
  ]) assert.throws(() => validateArguments('read', args));
  assert.throws(() => validateArguments('sources', { refs: ['view:nav/_root'] }));
});
test('目录路径合同不放行文件、URL、转义或越界层级', () => {
  for (const path of ['', '../secret', '/topics', 'topics/', 'topics//linux', 'topics/..', 'topics/%2e%2e', 'topics\\linux', 'Topics', '_other', 'topics/_root', Array(7).fill('x').join('/'), 'a'.repeat(49), Array(6).fill('a'.repeat(48)).join('/')]) {
    assert.throws(() => validateArguments('read', { refs: [`view:nav/${path}`] }), path);
  }
});
test('目录搜索显式启用，不能混入历史时点或错误参数类型', () => {
  for (const args of [{ query: '合成项目', target: 'views' }, { query: '合成项目', target: 'all', include_navigation: true }]) {
    assert.equal(validateArguments('search', args), args);
  }
  for (const args of [
    { query: '合成项目', include_navigation: 'true' },
    { query: '合成项目', target: 'events', include_navigation: true },
    { query: '合成项目', target: 'views', as_of: '2026-01-01T00:00:00Z' },
    { query: '合成项目', include_navigation: true, as_of: '2026-01-01T00:00:00Z' },
  ]) assert.throws(() => validateArguments('search', args));
});
test('Bootstrap接力说明给出目录优先入口和目录分页合同', () => {
  const capsule = makeCapsule({ action: 'bootstrap', request_id: 'synthetic-root' }, {}, { nonce: 'synthetic-nonce', session_ref: 'chatgpt:synthetic' });
  assert.match(capsule.text, /view:nav\/_root/);
  assert.match(capsule.text, /compact/);
  assert.match(capsule.text, /目录.*next_cursor/);
});
