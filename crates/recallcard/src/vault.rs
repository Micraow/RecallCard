use crate::model::*;
use chrono::{Datelike, Utc};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub struct Vault {
    root: PathBuf,
}
pub struct WriteGuard(File);
impl Drop for WriteGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl Vault {
    pub fn init(root: &Path) -> Result<Self> {
        reject_root_symlink(root)?;
        fs::create_dir_all(root).map_err(err)?;
        let root = fs::canonicalize(root).map_err(err)?;
        for name in [
            "events",
            "memories",
            "objects",
            "control",
            "control/dream-receipts",
            "control/suppressions",
            "generated/views",
            "generated/bootstrap",
            ".index",
        ] {
            let dir = root.join(name);
            reject_symlink(&dir)?;
            fs::create_dir_all(dir).map_err(err)?;
        }
        let vault = Self { root };
        let _lock = vault.lock()?;
        let marker = vault.root.join("control/schema-version.json");
        if marker.exists() {
            vault.check_marker()?;
        } else {
            vault.write_new(
                &marker,
                &json!({"schema_version": SCHEMA_VERSION, "application": "RecallCard"}),
            )?;
        }
        let ignore = vault.root.join(".gitignore");
        if !ignore.exists() {
            vault.write_bytes(&ignore, b".index/\ngenerated/\n.env\n*.tmp\n")?;
        }
        if !vault.root.join(".git").exists() {
            let out = Command::new("git")
                .arg("-C")
                .arg(&vault.root)
                .args(["init", "--quiet"])
                .output()
                .map_err(|e| format!("无法初始化 Git：{e}"))?;
            if !out.status.success() {
                return Err("Git 初始化失败".into());
            }
        }
        Ok(vault)
    }
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_checked(root, true)
    }
    /// 桌面打开/启动恢复只接受完整的已有目录；缺损时绝不补建或替换。
    pub fn open_existing(root: &Path) -> Result<Self> {
        Self::open_checked(root, false)
    }
    fn open_checked(root: &Path, repair_directories: bool) -> Result<Self> {
        reject_root_symlink(root)?;
        let root = fs::canonicalize(root).map_err(err)?;
        let vault = Self { root };
        vault.check_marker()?;
        for name in [
            "events",
            "memories",
            "control",
            "objects",
            "control/dream-receipts",
            "control/suppressions",
        ] {
            let path = vault.root.join(name);
            reject_symlink(&path)?;
            if repair_directories && !path.exists() {
                fs::create_dir_all(&path).map_err(err)?;
            }
            if !path.is_dir() {
                return Err(format!("Vault 目录不是目录：{name}"));
            }
        }
        Ok(vault)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn state_dir(&self) -> Result<PathBuf> {
        let base = std::env::var_os("RECALLCARD_STATE_DIR")
            .map(PathBuf::from)
            .or_else(dirs::state_dir)
            .or_else(dirs::data_local_dir)
            .ok_or("无法确定本机状态目录，请设置 RECALLCARD_STATE_DIR")?;
        fs::create_dir_all(&base).map_err(err)?;
        let base = fs::canonicalize(base).map_err(err)?;
        let path = base
            .join("recallcard")
            .join(hash(self.root.to_string_lossy().as_bytes()));
        reject_symlink(&path)?;
        fs::create_dir_all(&path).map_err(err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(err)?;
        }
        Ok(path)
    }
    fn check_marker(&self) -> Result<()> {
        let v: Value = read_json(&self.root.join("control/schema-version.json"))?;
        if v["schema_version"] != SCHEMA_VERSION || v["application"] != "RecallCard" {
            return Err("这不是受支持的 RecallCard Vault".into());
        }
        Ok(())
    }
    pub fn capture(&self, input: EventInput) -> Result<Event> {
        self.capture_batch(vec![input])?
            .pop()
            .ok_or_else(|| "事件未写入".into())
    }

    /// 整批先脱敏、校验并规划修订，再共享写锁和去重索引追加。
    /// 文件系统失败可能留下已持久化的前缀；重试相同来源版本安全去重。
    pub fn capture_batch(&self, inputs: Vec<EventInput>) -> Result<Vec<Event>> {
        self.capture_batch_with_progress(inputs, || true, |_, _, _| Ok(()))
    }

    pub fn capture_batch_with_progress<C, F>(
        &self,
        inputs: Vec<EventInput>,
        should_continue: C,
        progress: F,
    ) -> Result<Vec<Event>>
    where
        C: FnMut() -> bool,
        F: FnMut(usize, &Event, bool) -> Result<()>,
    {
        self.capture_batch_with_callbacks(inputs, should_continue, |_| Ok(()), progress)
    }

    /// on_existing 在写锁内收到已有正本，供任务保存初始去重基线。
    /// progress 在每条事件持久化后收到（已处理数量、事件、本次是否新增）。
    /// 回调不可重新取得 Vault 锁；取消只停止后续写入，绝不撤回已写 Event。
    pub fn capture_batch_with_callbacks<C, I, F>(
        &self,
        mut inputs: Vec<EventInput>,
        mut should_continue: C,
        on_existing: I,
        mut progress: F,
    ) -> Result<Vec<Event>>
    where
        C: FnMut() -> bool,
        I: FnOnce(&[Event]) -> Result<()>,
        F: FnMut(usize, &Event, bool) -> Result<()>,
    {
        use std::collections::BTreeMap;
        for input in &mut inputs {
            crate::capture::redact_event(input)?;
        }
        if !should_continue() {
            return Ok(Vec::new());
        }
        let _lock = self.lock()?;
        let mut events = self.events()?;
        on_existing(&events)?;
        let existing_count = events.len();
        let mut versions = BTreeMap::new();
        let mut latest = BTreeMap::new();
        let mut identities = BTreeMap::new();
        // 与旧版 capture 一致：忽略自动 revision_of 比较输入，重复任何旧版本也不回滚。
        fn version_key(input: &EventInput) -> Result<String> {
            let mut normalized = input.clone();
            normalized.revision_of = None;
            normalized.id()
        }
        for (index, event) in events.iter().enumerate() {
            versions.entry(version_key(&event.data)?).or_insert(index);
            latest.insert(event.data.revision_key(), index);
            identities.insert(event.id.clone(), event.data.revision_key());
        }
        let mut plan = Vec::with_capacity(inputs.len());
        for mut input in inputs {
            let version = version_key(&input)?;
            if let Some(index) = versions.get(&version).copied() {
                plan.push((index, false));
                continue;
            }
            let source = input.revision_key();
            if input.revision_of.is_none() {
                input.revision_of = latest.get(&source).map(|index| events[*index].id.clone());
            }
            if input
                .revision_of
                .as_ref()
                .is_some_and(|id| identities.get(id) != Some(&source))
            {
                return Err("修订必须引用同一资料范围内同一来源的已有事件；整批尚未写入".into());
            }
            let event = Event {
                schema_version: SCHEMA_VERSION,
                id: input.id()?,
                captured_at: Utc::now(),
                data: input,
            };
            let index = events.len();
            identities.insert(event.id.clone(), source.clone());
            versions.insert(version, index);
            if latest.get(&source).is_none_or(|old| {
                (events[*old].captured_at, &events[*old].id) <= (event.captured_at, &event.id)
            }) {
                latest.insert(source, index);
            }
            events.push(event);
            plan.push((index, true));
        }
        let mut result = Vec::with_capacity(plan.len());
        for (index, added) in plan {
            if !should_continue() {
                break;
            }
            let event = &events[index];
            if added {
                debug_assert!(index >= existing_count);
                let path = self.event_path(event);
                reject_symlink(path.parent().ok_or("事件路径无父目录")?)?;
                fs::create_dir_all(path.parent().ok_or("事件路径无父目录")?).map_err(err)?;
                let mut bytes = serde_json::to_vec(event).map_err(err)?;
                bytes.push(b'\n');
                self.staged(&path, &bytes)?
                    .persist_noclobber(&path)
                    .map_err(err)?;
                sync_parent(&path)?;
            }
            result.push(event.clone());
            progress(result.len(), event, added)?;
        }
        Ok(result)
    }
    pub(crate) fn event_path(&self, event: &Event) -> PathBuf {
        let time = event.data.occurred_at.unwrap_or(event.captured_at);
        let session = hash(event.data.session_key().as_bytes());
        self.root.join(format!(
            "events/{:04}/{:02}/{}/{}.jsonl",
            time.year(),
            time.month(),
            &session[..24],
            event.id
        ))
    }
    pub fn event(&self, id: &str) -> Result<Event> {
        validate_id(id, "evt_")?;
        self.events()?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| format!("找不到原始事件 {id}"))
    }
    pub fn memory(&self, id: &str) -> Result<Memory> {
        self.ensure_no_pending_dream()?;
        validate_id(id, "mem_")?;
        let path = self.memory_path(id);
        let text = read_text(&path)?;
        let memory = parse_memory(&text)?;
        memory.validate()?;
        if memory.id != id {
            return Err("记忆文件名与编号不匹配".into());
        }
        Ok(memory)
    }
    pub fn events(&self) -> Result<Vec<Event>> {
        self.ensure_no_pending_dream()?;
        let mut events = Vec::new();
        self.visit_events(|event| {
            events.push(event);
            Ok(())
        })?;
        events.sort_by(|a, b| a.captured_at.cmp(&b.captured_at).then(a.id.cmp(&b.id)));
        Ok(events)
    }
    pub fn memories(&self) -> Result<Vec<Memory>> {
        self.ensure_no_pending_dream()?;
        let mut result = Vec::new();
        for path in files_recursive(&self.root.join("memories"), "md")? {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("记忆文件名无效")?;
            if path.parent() != Some(self.root.join("memories").as_path()) {
                return Err("记忆记录必须直接位于 memories 目录".into());
            }
            result.push(self.memory(id)?);
        }
        result.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }
    pub fn add_memory(&self, input: MemoryInput) -> Result<Memory> {
        let _lock = self.lock()?;
        self.validate_evidence(&input)?;
        let memory = self.new_memory(input);
        self.write_memory(&memory, false)?;
        Ok(memory)
    }
    pub(crate) fn new_memory(&self, input: MemoryInput) -> Memory {
        let now = Utc::now();
        let state = if input.evidence == Evidence::AssistantSuggestion {
            MemoryState::Tentative
        } else {
            MemoryState::Active
        };
        Memory {
            schema_version: SCHEMA_VERSION,
            id: format!("mem_{}", Uuid::new_v4().simple()),
            revision: 1,
            state,
            created_at: now,
            updated_at: now,
            data: input,
        }
    }
    pub fn update_memory(
        &self,
        id: &str,
        expected_revision: u64,
        input: MemoryInput,
    ) -> Result<Memory> {
        let _lock = self.lock()?;
        self.update_memory_locked(id, expected_revision, input)
    }
    /// 调用方必须持有资料库写锁，用于审查快照与写入之间保持原子性。
    pub(crate) fn update_memory_locked(
        &self,
        id: &str,
        expected_revision: u64,
        input: MemoryInput,
    ) -> Result<Memory> {
        let mut memory = self.memory(id)?;
        self.validate_evidence(&input)?;
        if memory.revision != expected_revision {
            return Err(format!(
                "记忆版本冲突：当前版本 {}，请重新读取后再更新",
                memory.revision
            ));
        }
        if !matches!(memory.state, MemoryState::Active | MemoryState::Tentative) {
            return Err("只能修改 active/tentative 记忆；已撤回内容不得隐式恢复".into());
        }
        memory.data = input;
        memory.revision = memory.revision.checked_add(1).ok_or("记忆版本已达到上限")?;
        memory.updated_at = Utc::now();
        self.write_memory(&memory, true)?;
        Ok(memory)
    }
    pub fn set_state(
        &self,
        id: &str,
        expected_revision: u64,
        state: MemoryState,
    ) -> Result<Memory> {
        let _lock = self.lock()?;
        let mut memory = self.memory(id)?;
        if memory.revision != expected_revision {
            return Err("记忆版本冲突，请重新读取".into());
        }
        if state == MemoryState::Active {
            self.validate_evidence(&memory.data)?;
        }
        if memory.state != state {
            memory.state = state;
            memory.revision = memory.revision.checked_add(1).ok_or("记忆版本已达到上限")?;
            memory.updated_at = Utc::now();
            self.write_memory(&memory, true)?;
        }
        Ok(memory)
    }
    pub fn sources(&self, id: &str) -> Result<Vec<Event>> {
        let memory = self.memory(id)?;
        memory
            .data
            .source_refs
            .iter()
            .map(|id| self.event(id))
            .collect()
    }
    pub fn read(&self, id: &str) -> Result<Value> {
        if id.starts_with("evt_") {
            serde_json::to_value(self.event(id)?).map_err(err)
        } else if id.starts_with("mem_") {
            serde_json::to_value(self.memory(id)?).map_err(err)
        } else {
            Err("只能读取 evt_ 或 mem_ 记录编号".into())
        }
    }
    pub fn validate_evidence(&self, input: &MemoryInput) -> Result<()> {
        input.validate()?;
        let sources: Vec<Event> = input
            .source_refs
            .iter()
            .map(|id| self.event(id))
            .collect::<Result<_>>()?;
        if sources.iter().any(|e| !e.data.has_original_evidence()) {
            return Err("注入的上下文不能作为新的记忆证据".into());
        }
        if sources.iter().any(|e| e.data.scope != input.scope) {
            return Err("记忆不能扩大或混合来源 scope".into());
        }
        match input.evidence {
            Evidence::UserExplicit
                if !sources.iter().any(|e| {
                    e.data.role == Role::User
                        && (e.data.parts.is_empty()
                            && matches!(e.data.origin, Origin::Native | Origin::UserInput)
                            || e.data.parts.iter().any(|p| p.origin == Origin::UserInput))
                }) =>
            {
                Err("user_explicit 记忆必须引用用户原话".into())
            }
            Evidence::Observed
                if !sources.iter().any(|e| {
                    matches!(e.data.role, Role::User | Role::Tool)
                        && (e.data.parts.is_empty()
                            && matches!(
                                e.data.origin,
                                Origin::Native | Origin::UserInput | Origin::ToolOutput
                            )
                            || e.data.parts.iter().any(|p| {
                                matches!(p.origin, Origin::UserInput | Origin::ToolOutput)
                            }))
                }) =>
            {
                Err("助手建议不能作为已观察事实".into())
            }
            _ => Ok(()),
        }
    }
    pub fn rebuild_views(&self) -> Result<usize> {
        let _lock = self.lock()?;
        self.ensure_derived()?;
        let mut memories = self.memories()?;
        memories.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let mut out = String::from(
            "# 记忆视图\n\n此文件由 canonical Memory 确定性生成；请修改原始记录后重新生成。\n\n",
        );
        for m in &memories {
            out.push_str(&format!("## {}\n\n- 状态：{:?}\n- 范围：{}\n- 版本：{}\n- 证据性质：{:?}\n- 来源：{}\n\n{}\n\n",m.id,m.state,m.data.scope,m.revision,m.data.evidence,m.data.source_refs.join(", "),m.data.content));
        }
        self.write_bytes(
            &self.root.join("generated/views/memories.md"),
            out.as_bytes(),
        )?;
        Ok(memories.len())
    }
    pub fn ensure_derived(&self) -> Result<()> {
        for name in ["generated/views", "generated/bootstrap", ".index"] {
            let path = self.root.join(name);
            reject_symlink(&path)?;
            fs::create_dir_all(path).map_err(err)?;
        }
        Ok(())
    }
    pub fn doctor(&self) -> Result<Value> {
        let events = self.events()?;
        let memories = self.memories()?;
        for m in &memories {
            self.validate_evidence(&m.data)?;
        }
        for e in &events {
            for id in [&e.data.revision_of, &e.data.reply_to, &e.data.caused_by]
                .into_iter()
                .flatten()
            {
                self.event(id)?;
            }
        }
        Ok(
            json!({"ok":true,"events":events.len(),"memories":memories.len(),"schema_version":SCHEMA_VERSION}),
        )
    }
    pub(crate) fn ensure_no_pending_dream(&self) -> Result<()> {
        let event_journal = self.state_dir()?.join("event-transaction.json");
        reject_symlink(&event_journal)?;
        if event_journal.exists() {
            return Err("存在未完成的事件提交，请恢复对应导入任务".into());
        }
        let path = self.state_dir()?.join("dream-transaction.json");
        reject_symlink(&path)?;
        if path.exists() {
            return Err("存在未完成的 Dream 事务，请先执行 dream recover".into());
        }
        Ok(())
    }
    pub(crate) fn lock(&self) -> Result<WriteGuard> {
        let guard = self.lock_unchecked()?;
        self.ensure_no_pending_dream()?;
        Ok(guard)
    }
    /// 在多次读取间保持同一只读快照；写入忙碌时明确返回错误。
    pub fn read_guard(&self) -> Result<WriteGuard> {
        let path = self.state_dir()?.join("write.lock");
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(err)?;
        file.try_lock_shared()
            .map_err(|e| format!("Vault 正在更新，请稍后重试：{e}"))?;
        let guard = WriteGuard(file);
        self.ensure_no_pending_dream()?;
        Ok(guard)
    }
    pub(crate) fn lock_unchecked(&self) -> Result<WriteGuard> {
        let path = self.state_dir()?.join("write.lock");
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(err)?;
        file.try_lock()
            .map_err(|e| format!("Vault 正在被另一个写入操作使用：{e}"))?;
        Ok(WriteGuard(file))
    }
    pub(crate) fn memory_path(&self, id: &str) -> PathBuf {
        self.root.join("memories").join(format!("{id}.md"))
    }
    pub(crate) fn write_memory(&self, memory: &Memory, replace: bool) -> Result<()> {
        let bytes = render_memory(memory)?;
        let path = self.memory_path(&memory.id);
        if replace {
            self.write_bytes(&path, bytes.as_bytes())
        } else {
            self.staged(&path, bytes.as_bytes())?
                .persist_noclobber(&path)
                .map_err(err)?;
            sync_parent(&path)
        }
    }
    pub(crate) fn write_new<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(value).map_err(err)?;
        bytes.push(b'\n');
        self.staged(path, &bytes)?
            .persist_noclobber(path)
            .map_err(err)?;
        sync_parent(path)
    }
    pub(crate) fn write_replace<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(value).map_err(err)?;
        bytes.push(b'\n');
        self.write_bytes(path, &bytes)
    }
    pub(crate) fn write_bytes(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        persist_replace(self.staged(path, bytes)?, path)?;
        sync_parent(path)
    }
    pub(crate) fn staged(&self, path: &Path, bytes: &[u8]) -> Result<NamedTempFile> {
        reject_symlink(path)?;
        let parent = path.parent().ok_or("目标没有父目录")?;
        if !parent.starts_with(&self.root) && !parent.starts_with(self.state_dir()?) {
            return Err("写入目标超出 Vault/本机状态目录".into());
        }
        let mut temp = NamedTempFile::new_in(parent).map_err(err)?;
        temp.write_all(bytes).map_err(err)?;
        temp.as_file().sync_all().map_err(err)?;
        Ok(temp)
    }
}

#[cfg(not(windows))]
fn persist_replace(temp: NamedTempFile, path: &Path) -> Result<()> {
    temp.persist(path).map_err(err)?;
    Ok(())
}

#[cfg(windows)]
fn persist_replace(mut temp: NamedTempFile, path: &Path) -> Result<()> {
    // Windows 上短暂占用或尚未关闭的删除共享句柄可能阻止替换。
    // 始终重试同一份已同步的临时文件，不删除旧记录，不放宽 ACL。
    for attempt in 0..=32 {
        match temp.persist(path) {
            Ok(_) => return Ok(()),
            Err(failure)
                if attempt < 32 && matches!(failure.error.raw_os_error(), Some(5 | 32 | 33)) =>
            {
                temp = failure.file;
                std::thread::sleep(std::time::Duration::from_millis(1 << attempt.min(5)));
                reject_symlink(path)?;
            }
            Err(failure) => return Err(err(failure)),
        }
    }
    unreachable!("最后一次替换必定返回成功或错误")
}

pub fn render_memory(memory: &Memory) -> Result<String> {
    let mut metadata = serde_json::to_value(memory).map_err(err)?;
    metadata
        .as_object_mut()
        .ok_or("记忆元数据无效")?
        .remove("content");
    Ok(format!(
        "---\n{}---\n\n{}\n",
        serde_yaml::to_string(&metadata).map_err(err)?,
        memory.data.content
    ))
}
pub fn parse_memory(text: &str) -> Result<Memory> {
    let rest = text
        .strip_prefix("---\n")
        .ok_or("Memory 缺少 YAML frontmatter")?;
    let (head, body) = rest
        .split_once("\n---\n")
        .ok_or("Memory frontmatter 未闭合")?;
    let mut value: Value =
        serde_yaml::from_str(head).map_err(|e| format!("Memory YAML 无效：{e}"))?;
    value
        .as_object_mut()
        .ok_or("Memory frontmatter 必须是对象")?
        .insert(
            "content".into(),
            Value::String(
                body.strip_prefix('\n')
                    .unwrap_or(body)
                    .strip_suffix('\n')
                    .unwrap_or(body.strip_prefix('\n').unwrap_or(body))
                    .into(),
            ),
        );
    serde_json::from_value(value).map_err(err)
}
pub(crate) fn files_recursive(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    reject_symlink(root)?;
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(err)? {
        let path = entry.map_err(err)?.path();
        reject_symlink(&path)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("文件名不是 UTF-8")?;
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            files.extend(files_recursive(&path, extension)?);
        } else if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some(extension) {
            files.push(path);
        } else {
            return Err(format!("正本目录含有不支持的文件：{}", path.display()));
        }
    }
    files.sort();
    Ok(files)
}
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_str(&read_text(path)?)
        .map_err(|e| format!("记录损坏或包含未解决的 Git 冲突 {}：{e}", path.display()))
}
pub(crate) fn read_text(path: &Path) -> Result<String> {
    reject_symlink(path)?;
    let mut file = File::open(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    if file.metadata().map_err(err)?.len() > 16 * 1024 * 1024 {
        return Err("记录文件超过安全上限 16 MiB".into());
    }
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(err)?;
    Ok(text)
}
pub(crate) fn reject_symlink(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(m) if m.is_symlink() => {
                return Err(format!("不允许 Vault 符号链接：{}", ancestor.display()))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
    }
    Ok(())
}
pub(crate) fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    File::open(path.parent().ok_or("没有父目录")?)
        .map_err(err)?
        .sync_all()
        .map_err(err)?;
    Ok(())
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn reject_root_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_symlink() => Err("Vault 根目录不能是符号链接".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(err(e)),
    }
}
