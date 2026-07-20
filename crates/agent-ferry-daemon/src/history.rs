//! [INPUT] daemon 已归一化的任务创建信息和 `HandoffEvent`。
//! [OUTPUT] 当前用户私有、原子写入、可查询和可删除的有界 JSON 任务历史。
//! [POS] 本模块是 daemon 内部 Repository，不承担 Agent Session 恢复或远程记忆职责。
//! [INVARIANTS] 不保存捕获正文与凭据；输出、事件和记录数均有上限；运行中记录不能删除；
//! daemon 重启只把遗留任务标记为 interrupted，不能伪造继续执行。
//! [PROTOCOL] 字段与隐私边界由 ADR 0030 定义；对外 wire 类型位于 agent-ferry-protocol。

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_ferry_core::AgentFerryPaths;
use agent_ferry_protocol::{
    HandoffEvent, HandoffEventKind, SourceDocument, TaskHistoryEvent, TaskHistoryRecord,
    TaskHistoryState, TaskHistorySummary,
};

const MAX_HISTORY_OUTPUT_BYTES: usize = 768 * 1024;
const MAX_HISTORY_EVENTS: usize = 500;
const MAX_HISTORY_RECORDS: usize = 500;

#[derive(Debug, Clone)]
pub(crate) struct TargetSnapshot {
    pub name: String,
    pub workspace_name: Option<String>,
    pub workspace_path: Option<String>,
}

#[derive(Debug)]
pub(crate) struct HistoryRepository {
    directory: PathBuf,
    records: HashMap<String, TaskHistoryRecord>,
    last_persisted_ms: HashMap<String, u64>,
}

impl HistoryRepository {
    pub(crate) fn open(paths: &AgentFerryPaths) -> io::Result<Self> {
        fs::create_dir_all(&paths.history_dir)?;
        fs::set_permissions(&paths.history_dir, fs::Permissions::from_mode(0o700))?;
        let mut records = HashMap::new();
        for entry in fs::read_dir(&paths.history_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<TaskHistoryRecord>(&bytes) else {
                continue;
            };
            records.insert(record.summary.task_id.clone(), record);
        }
        let mut repository = Self {
            directory: paths.history_dir.clone(),
            last_persisted_ms: records
                .iter()
                .map(|(task_id, record)| (task_id.clone(), record.summary.updated_at_ms))
                .collect(),
            records,
        };
        repository.mark_abandoned_tasks_interrupted()?;
        Ok(repository)
    }

    pub(crate) fn create(
        &mut self,
        task_id: &str,
        target_id: String,
        target: TargetSnapshot,
        prompt: String,
        source: &SourceDocument,
    ) -> io::Result<()> {
        if self.records.contains_key(task_id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "task_id 已存在于历史记录中",
            ));
        }
        let now = now_ms();
        let record = TaskHistoryRecord {
            summary: TaskHistorySummary {
                task_id: task_id.to_owned(),
                title: source.title.clone(),
                url: source.url.clone(),
                site: source.site.clone(),
                extractor: source.extractor.clone(),
                target_id,
                target_name: target.name,
                workspace_name: target.workspace_name,
                workspace_path: target.workspace_path,
                state: TaskHistoryState::Running,
                stage: "正在提交任务".to_owned(),
                created_at_ms: now,
                updated_at_ms: now,
                completed_at_ms: None,
            },
            prompt,
            output: String::new(),
            output_truncated: false,
            error: None,
            run_id: None,
            events: Vec::new(),
        };
        self.records.insert(task_id.to_owned(), record);
        self.persist(task_id)?;
        self.last_persisted_ms.insert(task_id.to_owned(), now);
        self.prune_terminal_records();
        Ok(())
    }

    pub(crate) fn apply_event(&mut self, event: &HandoffEvent) -> io::Result<()> {
        let Some(record) = self.records.get_mut(&event.task_id) else {
            return Ok(());
        };
        let now = now_ms();
        record.summary.updated_at_ms = now;
        if let Some(run_id) = &event.run_id {
            record.run_id = Some(run_id.clone());
        }
        match event.event {
            HandoffEventKind::Submitted => "Agent 已启动".clone_into(&mut record.summary.stage),
            HandoffEventKind::Running => "Agent 正在分析".clone_into(&mut record.summary.stage),
            HandoffEventKind::OutputDelta => {
                "Agent 正在分析".clone_into(&mut record.summary.stage);
                if let Some(text) = &event.text {
                    append_bounded(&mut record.output, text, &mut record.output_truncated);
                }
            }
            HandoffEventKind::ToolStarted => {
                record.summary.stage = event.text.as_ref().map_or_else(
                    || "Agent 正在使用工具".to_owned(),
                    |text| format!("正在使用工具：{text}"),
                );
            }
            HandoffEventKind::ToolCompleted => {
                "工具执行完成".clone_into(&mut record.summary.stage);
            }
            HandoffEventKind::Completed => {
                record.summary.state = TaskHistoryState::Completed;
                "已完成".clone_into(&mut record.summary.stage);
                record.summary.completed_at_ms = Some(now);
                if let Some(text) = &event.text {
                    if !text.trim().is_empty() {
                        record.output.clear();
                        record.output_truncated = false;
                        append_bounded(&mut record.output, text, &mut record.output_truncated);
                    }
                }
            }
            HandoffEventKind::Failed => {
                record.summary.state = TaskHistoryState::Failed;
                "执行失败".clone_into(&mut record.summary.stage);
                record.summary.completed_at_ms = Some(now);
                record.error.clone_from(&event.text);
            }
            HandoffEventKind::Cancelled => {
                record.summary.state = TaskHistoryState::Cancelled;
                "已取消".clone_into(&mut record.summary.stage);
                record.summary.completed_at_ms = Some(now);
            }
        }
        if event.event != HandoffEventKind::OutputDelta {
            if record.events.len() >= MAX_HISTORY_EVENTS {
                record.events.remove(0);
            }
            record.events.push(TaskHistoryEvent {
                sequence: event.sequence,
                event: event.event,
                timestamp_ms: now,
                text: event.text.clone(),
            });
        }
        let terminal = event.event == HandoffEventKind::Completed
            || event.event == HandoffEventKind::Failed
            || event.event == HandoffEventKind::Cancelled;
        let last_persisted = self
            .last_persisted_ms
            .get(&event.task_id)
            .copied()
            .unwrap_or_default();
        if terminal
            || event.event != HandoffEventKind::OutputDelta
            || now.saturating_sub(last_persisted) >= 500
        {
            self.persist(&event.task_id)?;
            self.last_persisted_ms.insert(event.task_id.clone(), now);
        }
        Ok(())
    }

    pub(crate) fn list(
        &self,
        state: Option<TaskHistoryState>,
        limit: u16,
    ) -> Vec<TaskHistorySummary> {
        let mut tasks = self
            .records
            .values()
            .filter(|record| state.is_none_or(|state| record.summary.state == state))
            .map(|record| record.summary.clone())
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| {
            let left_active = !left.state.is_terminal();
            let right_active = !right.state.is_terminal();
            right_active
                .cmp(&left_active)
                .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
        });
        tasks.truncate(usize::from(limit.clamp(1, 200)));
        tasks
    }

    pub(crate) fn get(&self, task_id: &str) -> Option<TaskHistoryRecord> {
        self.records.get(task_id).cloned()
    }

    pub(crate) fn fail_if_running(&mut self, task_id: &str, message: &str) -> io::Result<()> {
        let Some(record) = self.records.get(task_id) else {
            return Ok(());
        };
        if record.summary.state != TaskHistoryState::Running {
            return Ok(());
        }
        let sequence = record
            .events
            .last()
            .map_or(0, |event| event.sequence.saturating_add(1));
        self.apply_event(&HandoffEvent {
            protocol_version: agent_ferry_protocol::PROTOCOL_VERSION,
            request_id: "history-internal".to_owned(),
            task_id: task_id.to_owned(),
            sequence,
            event: HandoffEventKind::Failed,
            run_id: None,
            text: Some(message.to_owned()),
        })
    }

    pub(crate) fn delete(&mut self, task_id: &str) -> io::Result<bool> {
        let Some(record) = self.records.get(task_id) else {
            return Ok(false);
        };
        if !record.summary.state.is_terminal() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "进行中的任务不能删除",
            ));
        }
        self.records.remove(task_id);
        self.last_persisted_ms.remove(task_id);
        match fs::remove_file(self.record_path(task_id)) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        }
    }

    fn mark_abandoned_tasks_interrupted(&mut self) -> io::Result<()> {
        let now = now_ms();
        let interrupted = self
            .records
            .iter_mut()
            .filter_map(|(task_id, record)| {
                if record.summary.state != TaskHistoryState::Running {
                    return None;
                }
                record.summary.state = TaskHistoryState::Interrupted;
                "后台服务重启，任务已中断".clone_into(&mut record.summary.stage);
                record.summary.updated_at_ms = now;
                record.summary.completed_at_ms = Some(now);
                record.error = Some("Agent Ferry 后台服务在任务完成前退出".to_owned());
                Some(task_id.clone())
            })
            .collect::<Vec<_>>();
        for task_id in interrupted {
            self.persist(&task_id)?;
            self.last_persisted_ms.insert(task_id, now);
        }
        Ok(())
    }

    fn prune_terminal_records(&mut self) {
        if self.records.len() <= MAX_HISTORY_RECORDS {
            return;
        }
        let mut terminal = self
            .records
            .values()
            .filter(|record| record.summary.state.is_terminal())
            .map(|record| (record.summary.task_id.clone(), record.summary.updated_at_ms))
            .collect::<Vec<_>>();
        terminal.sort_by_key(|(_, updated_at)| *updated_at);
        let remove_count = self.records.len().saturating_sub(MAX_HISTORY_RECORDS);
        for (task_id, _) in terminal.into_iter().take(remove_count) {
            self.records.remove(&task_id);
            self.last_persisted_ms.remove(&task_id);
            let _ = fs::remove_file(self.record_path(&task_id));
        }
    }

    fn persist(&self, task_id: &str) -> io::Result<()> {
        let record = self
            .records
            .get(task_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "历史任务不存在"))?;
        let bytes = serde_json::to_vec(record).map_err(io::Error::other)?;
        let temporary = self.directory.join(format!(".{task_id}.tmp"));
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(temporary, self.record_path(task_id))
    }

    fn record_path(&self, task_id: &str) -> PathBuf {
        self.directory.join(format!("{task_id}.json"))
    }
}

fn append_bounded(output: &mut String, text: &str, truncated: &mut bool) {
    if *truncated || text.is_empty() {
        return;
    }
    let remaining = MAX_HISTORY_OUTPUT_BYTES.saturating_sub(output.len());
    if text.len() <= remaining {
        output.push_str(text);
        return;
    }
    let mut boundary = remaining.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.push_str(&text[..boundary]);
    *truncated = true;
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
