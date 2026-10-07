// 普通导入的一次批准、可恢复进度与官方导出入口；不接触网页登录信息。
export function createImportTasks(ui) {
  const { state, $, button, paragraph, hint, line, invoke, run, render, showNotice } = ui;
  let timer = null;
  let revision = 0;
  const data = () => state.importJobs ||= { preview: null, current: null, history: [], error: '', loaded: false };
  const args = () => ({ sessionId: state.vault.session_id, scope: state.scope });
  const active = job => job && ['running', 'cancelling'].includes(job.state);
  const same = (epoch, session, scope, value) => state.epoch === epoch && state.vault?.session_id === session && state.scope === scope && state.importJobs === value;
  function clearTimer() { if (timer != null) window.clearTimeout(timer); timer = null; }
  function validPreview(preview) {
    const count = value => Number.isSafeInteger(value) && value >= 0;
    if (!preview || typeof preview.preview_id !== 'string' || !preview.preview_id || preview.session_id !== state.vault?.session_id || preview.scope !== state.scope
      || !count(preview.event_count) || preview.event_count < 1 || preview.event_count > 100000
      || !Array.isArray(preview.files) || !preview.files.length || preview.files.length > 32
      || preview.files.some(file => typeof file?.file_name !== 'string' || !count(file.byte_count))
      || !Array.isArray(preview.conversations) || !preview.conversations.length
      || !Array.isArray(preview.samples) || preview.samples.some(sample => typeof sample?.content !== 'string')
      || !count(preview.redacted_event_count) || preview.redacted_event_count > preview.event_count) throw new Error('没有可确认的完整导入预览，请重新选择导出文件');
    return preview;
  }
  function validStatus(job) {
    if (!job || job.scope !== state.scope || typeof job.job_id !== 'string' || !job.job_id.trim() || typeof job.can_resume !== 'boolean' || !['running', 'cancelling', 'cancelled', 'interrupted', 'completed', 'failed'].includes(job.state)
      || !['events_total', 'events_processed', 'events_added', 'events_duplicates', 'files_total', 'conversations_total'].every(key => Number.isSafeInteger(job[key]) && job[key] >= 0) || job.events_total < 1 || job.events_processed > job.events_total || job.events_added + job.events_duplicates !== job.events_processed || (job.state === 'completed' && job.events_processed !== job.events_total)) throw new Error('导入进度暂时无法核实，请重新读取');
    return job;
  }
  function accept(job, expectedId = null) { const checked = validStatus(job); if (expectedId && checked.job_id !== expectedId) throw new Error('导入任务已改变，请重新读取'); revision += 1; data().current = checked; data().error = ''; schedule(); }
  function schedule() {
    clearTimer();
    if (!active(data().current)) return;
    const selected = data(), epoch = state.epoch, session = state.vault?.session_id, scope = state.scope, expectedRevision = revision, jobId = selected.current?.job_id;
    timer = window.setTimeout(async () => {
      timer = null;
      if (!same(epoch, session, scope, selected) || revision !== expectedRevision || !active(selected.current) || selected.current.job_id !== jobId) return;
      try {
        const result = await invoke('import_job_status', { sessionId: session, scope, jobId });
        if (!same(epoch, session, scope, selected) || revision !== expectedRevision || selected.current?.job_id !== jobId) return;
        if (result?.job_id !== jobId) throw new Error('导入任务已改变'); selected.current = validStatus(result); selected.error = '';
        render(); schedule();
      } catch (error) {
        if (!same(epoch, session, scope, selected) || revision !== expectedRevision || selected.current?.job_id !== jobId) return;
        selected.error = '暂时无法读取进度，导入可能仍在进行。请重新读取，或暂停后检查。'; render();
      }
    }, 600);
  }
  function restore(values) {
    if (!Array.isArray(values)) throw new Error('无法读取之前的导入，请重试');
    const jobs = values.map(validStatus);
    const selected = data(); selected.history = jobs; selected.loaded = true;
    // 服务按创建时间倒序给出。最新任务完成后，不把更早的暂停任务强行拉回首页。
    const running = jobs.find(active);
    const latest = jobs[0];
    const resume = latest?.can_resume && latest.state !== 'completed' ? latest : null;
    if (running || resume) { accept(running || resume); return true; }
    return false;
  }
  async function readResults(jobId, offset) {
    const result = await invoke('import_job_conversations', { ...args(), jobId, offset });
    if (!result || result.job_id !== jobId || result.scope !== state.scope || result.offset !== offset
      || !Number.isSafeInteger(result.total) || result.total < 0 || !Array.isArray(result.conversations)
      || result.conversations.length > 50 || result.conversations.some(item => typeof item?.session_ref !== 'string' || !item.session_ref || typeof item.title !== 'string')
      || new Set(result.conversations.map(item => item.session_ref)).size !== result.conversations.length
      || (result.next_offset != null && (!Number.isSafeInteger(result.next_offset) || result.next_offset !== offset + result.conversations.length || result.next_offset <= offset || result.next_offset >= result.total))) throw new Error('本批会话暂时无法核实，请重新读取');
    const status = validStatus(result.status);
    if (status.job_id !== jobId || active(status)) throw new Error('本批导入状态已改变，请先回到导入记录检查');
    return result;
  }
  async function history() {
    await run('正在查找之前的导入…', async current => {
      const values = await invoke('list_import_jobs', args());
      if (!current()) return;
      if (!Array.isArray(values)) throw new Error('无法读取之前的导入，请重试');
      data().history = values.map(validStatus); data().loaded = true;
      const running = data().history.find(active);
      if (running) accept(running);
    });
  }
  function choose() {
    if (state.busy || active(data().current)) return;
    return run('正在读取导出文件…', async current => {
      data().preview = null; data().error = '';
      state.importPreview = null; state.importSelection = null; state.importSelectedIds = [];
      await invoke('cancel_previews', { sessionId: state.vault.session_id }); render();
      const result = await invoke('pick_import_files', { ...args(), format: 'auto' });
      if (current() && result) { data().preview = validPreview(result); data().current = null; }
    });
  }
  function start(preview, control) {
    if (state.busy || !control.isConnected || data().preview !== preview || state.page !== 'import' || preview.session_id !== state.vault?.session_id || preview.scope !== state.scope) return;
    return run('正在开始导入…', async current => {
      data().preview = null; render();
      const result = await invoke('start_import_job', { ...args(), previewId: preview.preview_id });
      if (current()) accept(result);
    });
  }
  function update(kind, job, control) {
    if (state.busy || (control && !control.isConnected) || data().current?.job_id !== job.job_id || state.page !== 'import' || job.scope !== state.scope) return;
    revision += 1; clearTimer();
    return run(kind === 'cancel_import_job' ? '正在暂停导入…' : '正在读取导入任务…', async current => {
      try {
        const result = await invoke(kind, { ...args(), jobId: job.job_id });
        if (current()) accept(result, job.job_id);
      } catch (error) {
        if (current()) data().error = '操作结果暂时无法确认，请重新读取进度；已保存的消息会保留。';
        throw error;
      }
    });
  }
  function discardPreview(preview, control) {
    if (state.busy || !control.isConnected || data().preview !== preview) return;
    data().preview = null;
    return run('已取消，尚未导入', () => invoke('cancel_previews', { sessionId: state.vault.session_id }));
  }
  function fileDetails(preview) {
    const box = $('details', { class: 'import-job-details' }, $('summary', {}, '查看文件与导入覆盖'));
    for (const file of preview.files || []) box.append(paragraph(`${file.file_name} · ${Math.ceil(file.byte_count / 1024)} KiB`));
    for (const note of preview.coverage?.notes || []) box.append(paragraph(note));
    const ds = preview.coverage?.deepseek;
    if (ds) box.append(paragraph('DeepSeek 的所有有效分支都会保留；未提供当前分支时不会猜测。隐藏推理、账号资料和附件原件不导入。'));
    return box;
  }
  function pane() {
    const value = data();
    const box = $('section', { class: 'panel import-job', 'aria-label': '导入平台历史' }, $('h2', {}, '把以前的对话带进来'));
    const job = value.current;
    if (job) {
      const titles = { running: '正在导入', cancelling: '正在暂停', cancelled: '已暂停', interrupted: '可以继续上次导入', completed: '导入完成', failed: '导入尚未完成' };
      box.append($('h3', {}, titles[job.state]), paragraph(`已处理 ${job.events_processed} / ${job.events_total} 条消息`),
        $('progress', { max: Math.max(1, job.events_total), value: job.events_processed, 'aria-label': '导入进度' }),
        paragraph(job.message || '已保存的消息会保留，重复内容会自动跳过。'));
      if (value.error) box.append(hint(value.error, true));
      const controls = $('div', { class: 'button-row import-job-actions' });
      if (job.state === 'running') controls.append(button('暂停导入', event => update('cancel_import_job', job, event.currentTarget)));
      if (job.can_resume && !active(job)) controls.append(button('继续导入', event => update('resume_import_job', job, event.currentTarget), true));
      if (value.error) controls.append(button('重新读取进度', event => update('import_job_status', job, event.currentTarget)));
      if (!active(job)) controls.append(button('查看本批会话', event => { if (state.busy || !event.currentTarget.isConnected || data() !== value || active(value.current) || value.current?.job_id !== job.job_id) return; ui.viewImported(job); }, job.state === 'completed'), button('导入其他文件', event => { if (state.busy || !event.currentTarget.isConnected || data() !== value || active(value.current) || value.current?.job_id !== job.job_id) return; revision += 1; clearTimer(); value.current = null; render(); }));
      box.append(controls);
      if (active(job)) box.append(paragraph('可以随时暂停。已写入的内容保留，之后从进度处继续；暂停后即可查看资料。'));
      box.append($('details', {}, $('summary', {}, '查看数量'), line('导出文件', job.files_total), line('会话', job.conversations_total), line('新增记录', job.events_added), line('已存在记录', job.events_duplicates)));
      return box;
    }
    const preview = value.preview;
    if (preview) {
      box.append(paragraph(`${preview.files.length} 个文件 · ${preview.conversations.filter(item => item.event_count > 0).length} 个有消息的会话 · ${preview.event_count} 条消息`),
        line('保存到', state.vault.display_name), line('资料分类', ({ personal: '个人资料', work: '工作资料' })[preview.scope] || preview.scope));
      if (preview.redacted_event_count) box.append(hint(`${preview.redacted_event_count} 条消息已检测到敏感字段并遮蔽，请仍检查文字样本。`));
      if (preview.warning) box.append(paragraph(preview.warning));
      box.append(fileDetails(preview));
      const samples = $('details', {}, $('summary', {}, '查看文字样本'));
      for (const sample of preview.samples || []) samples.append($('div', { class: 'sample' }, $('strong', {}, sample.role === 'user' ? '用户原话' : sample.role === 'assistant' ? 'AI 回复' : '其他记录'), paragraph(sample.content)));
      box.append(samples);
      const approve = button(`导入全部 ${preview.event_count} 条消息`, () => start(preview, approve), true);
      box.append($('div', { class: 'button-row import-job-actions' }, approve, button('取消', event => discardPreview(preview, event.currentTarget))));
      return box;
    }
    box.append(paragraph('选择下载好的 JSON 或 ZIP，可以同时选择多份，不需要解压。检查数量后一次导入，重复内容会跳过。'), button('选择导出文件', choose, true));
    const guide = $('details', { class: 'official-export-guide' }, $('summary', {}, '还没有导出文件？从 DeepSeek 获取全部历史'),
      paragraph('打开 DeepSeek：头像 → 系统设置 → 数据管理 → 导出所有历史对话。生成后点击下载，把下载得到的文件一起选进来；网站可能提供两份。'),
      button('打开 DeepSeek', () => run('正在打开浏览器…', async () => { await invoke('open_deepseek', {}); showNotice('在 DeepSeek 中完成官方导出，下载后回到这里选择文件'); })),
      paragraph('登录、网站生成时间和浏览器下载由 DeepSeek 与浏览器处理。这里只导入对话，账号信息与附件原件不会作为记忆保存。'));
    box.append(guide);
    box.append(button('查看导入记录', history, false, 'small'));
    if (value.loaded && !value.history.length) box.append(paragraph('还没有导入记录。'));
    for (const previous of value.history.filter(item => !active(item))) box.append($('div', { class: 'import-history-row' }, paragraph(`${previous.created_at ? new Date(previous.created_at).toLocaleString() + ' · ' : ''}${previous.state === 'completed' ? '已完成' : '尚未完成'} · ${previous.files_total} 个文件 · 已处理 ${previous.events_processed} / ${previous.events_total} 条`), button('查看这次导入', event => { if (state.busy || !event.currentTarget.isConnected || data() !== value || active(value.current) || !value.history.includes(previous)) return; revision += 1; clearTimer(); return run('正在读取导入任务…', async current => { const fresh = await invoke('import_job_status', { ...args(), jobId: previous.job_id }); if (current()) accept(fresh, previous.job_id); }); }, false, 'small')));
    return box;
  }
  return { pane, history, restore, readResults, active: () => active(data().current), clearPreview: () => { if (state.importJobs) state.importJobs.preview = null; }, clearTimer };
}
