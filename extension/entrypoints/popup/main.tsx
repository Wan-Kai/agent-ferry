import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import type { CapturedPage, CapturedPageResult } from "../extract-page.content";
import { extractArxivPdf, isArxivPdfUrl, type ArxivPdfProgress } from "../../lib/arxiv-pdf-extractor";
import { MIN_X_HANDOFF_CONTENT_BYTES, prepareHandoffTransfer } from "../../lib/handoff-transfer";
import {
  DEFAULT_PROMPT,
  EMPTY_PROMPT_TEMPLATE_SETTINGS,
  deleteTemplate,
  effectivePrompt,
  loadPromptTemplateSettings,
  persistPromptTemplateSettings,
  saveTemplate,
  type PromptTemplateSettings,
} from "../../lib/prompt-templates";
import "./style.css";

const NATIVE_HOST_NAME = "com.agentferry.host";
const PROTOCOL_VERSION = 1;
const SELECTED_AGENT_KEY = "selected_agent_key_v1";
const SELECTED_WORKSPACE_KEY = "selected_workspace_by_agent_v1";

type TargetKind = "remote_hermes" | "local_open_code" | "local_claude_code" | "local_codex_cli" | "local_codex_app";
type TargetState = "ready" | "credential_missing" | "authentication_failed" | "connection_failed" | "incompatible";
type HistoryState = "running" | "completed" | "failed" | "cancelled" | "interrupted";
type View = "send" | "history" | "settings" | "detail";

type HandoffTargetStatus = { id: string; name: string; kind: TargetKind; state: TargetState; capabilities: string[] };
type LocalWorkspaceStatus = { id: string; name: string; path: string; ready: boolean };
type StatusResult = { core_version: string; daemon: "ready" | "not_detected"; targets?: HandoffTargetStatus[]; workspaces?: LocalWorkspaceStatus[] };
type TaskSummary = {
  task_id: string;
  title: string;
  url: string;
  site: string | null;
  extractor: string;
  target_id: string;
  target_name: string;
  workspace_name: string | null;
  workspace_path: string | null;
  state: HistoryState;
  stage: string;
  created_at_ms: number;
  updated_at_ms: number;
  completed_at_ms: number | null;
};
type TaskEvent = { sequence: number; event: string; timestamp_ms: number; text: string | null };
type TaskRecord = {
  summary: TaskSummary;
  prompt: string;
  output: string;
  output_truncated: boolean;
  error: string | null;
  run_id: string | null;
  events: TaskEvent[];
};
type HostResponse<T = StatusResult> = {
  protocol_version: number;
  request_id: string;
  result?: T;
  error?: { code: string; message: string; recoverable: boolean };
};
type HandoffEvent = {
  protocol_version: number;
  request_id: string;
  task_id: string;
  sequence: number;
  event: "submitted" | "running" | "output_delta" | "tool_started" | "tool_completed" | "completed" | "failed" | "cancelled";
  text?: string;
};
type HandoffTransferAck = { protocol_version: number; request_id: string; task_id: string; phase: "begin" | "chunk"; next_index: number };
type ConnectionState = { kind: "checking" } | { kind: "ready"; result: StatusResult } | { kind: "unavailable"; detail: string };
type SubmitState = { kind: "idle" } | { kind: "busy"; label: string } | { kind: "error"; label: string };
type TemplateDraft = { id?: string; name: string; content: string };
type ChromeScriptingApi = { scripting: { executeScript(options: { target: { tabId: number }; files: string[] }): Promise<Array<{ result?: unknown; error?: string }>> } };
type SelectOption = { value: string; title: string; meta?: string; avatar?: string; state?: string; stateKind?: "success" | "danger"; disabled?: boolean };

const ICON_PATHS: Record<string, React.ReactNode> = {
  send: <><path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/></>,
  history: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
  settings: <><path d="M20 7h-9"/><path d="M14 17H5"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/></>,
  globe: <><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></>,
  loader: <path d="M21 12a9 9 0 1 1-6.219-8.56"/>,
  check: <path d="m5 12 4 4L19 6"/>,
  "check-circle": <><circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/></>,
  "x-circle": <><circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/></>,
  folder: <path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>,
  plus: <><path d="M5 12h14"/><path d="M12 5v14"/></>,
  trash: <><path d="M3 6h18"/><path d="M8 6V4h8v2"/><path d="m19 6-1 14H6L5 6"/><path d="M10 11v5"/><path d="M14 11v5"/></>,
  pencil: <><path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L8 18l-4 1 1-4Z"/></>,
  "chevron-down": <path d="m6 9 6 6 6-6"/>,
  "chevron-right": <path d="m9 18 6-6-6-6"/>,
  "arrow-left": <><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></>,
  refresh: <><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></>,
  "file-text": <><path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5Z"/><polyline points="14 2 14 8 20 8"/><line x1="8" x2="16" y1="13" y2="13"/><line x1="8" x2="16" y1="17" y2="17"/></>,
  x: <><path d="M18 6 6 18"/><path d="m6 6 12 12"/></>,
};

function Icon({ name, className = "" }: { name: string; className?: string }) {
  return <svg className={`icon ${className}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{ICON_PATHS[name]}</svg>;
}

function ProductSelect({ value, options, label, kind, onChange }: { value: string; options: SelectOption[]; label: string; kind: string; onChange(value: string): void }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const selected = options.find((option) => option.value === value) ?? options.find((option) => !option.disabled);
  useEffect(() => {
    const close = (event: PointerEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", close);
    return () => document.removeEventListener("pointerdown", close);
  }, []);
  useEffect(() => {
    if (!open) return;
    const selectedIndex = options.findIndex((option) => option.value === selected?.value && !option.disabled);
    window.requestAnimationFrame(() => optionRefs.current[selectedIndex >= 0 ? selectedIndex : options.findIndex((option) => !option.disabled)]?.focus());
  }, [open, options, selected?.value]);
  const moveFocus = (currentIndex: number, direction: number) => {
    let next = currentIndex;
    for (let count = 0; count < options.length; count += 1) {
      next = (next + direction + options.length) % options.length;
      if (!options[next]?.disabled) { optionRefs.current[next]?.focus(); return; }
    }
  };
  const selectOption = (option: SelectOption) => {
    onChange(option.value);
    setOpen(false);
    // 选项会随菜单卸载，主动回焦可避免键盘用户的焦点落回页面根节点。
    window.requestAnimationFrame(() => root.current?.querySelector<HTMLButtonElement>(".select-trigger")?.focus());
  };
  return <div className={`custom-select ${kind}-select ${open ? "open" : ""}`} ref={root} style={{ "--menu-space": open ? `${Math.min(options.length * 52, 232) + 6}px` : "0px" } as React.CSSProperties}>
    <button className="select-trigger" type="button" role="combobox" aria-expanded={open} aria-haspopup="listbox" aria-label={label} onClick={() => setOpen((current) => !current)} onKeyDown={(event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setOpen(true);
      }
      if (event.key === "Escape") setOpen(false);
    }}>
      <span className="select-trigger-copy"><span className="select-trigger-title">{selected?.title ?? "请选择"}</span>{selected?.meta && <span className="select-trigger-meta">{selected.meta}</span>}</span>
      <Icon name="chevron-down" className="select-chevron" />
    </button>
    {open && <div className="select-menu" role="listbox">{options.map((option, index) => <button className="select-option" type="button" role="option" aria-selected={option.value === value} disabled={option.disabled} key={option.value} ref={(node) => { optionRefs.current[index] = node; }} onClick={() => selectOption(option)} onKeyDown={(event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); moveFocus(index, event.key === "ArrowDown" ? 1 : -1); }
      if (event.key === "Home" || event.key === "End") { event.preventDefault(); const candidates = options.map((candidate, candidateIndex) => ({ candidate, candidateIndex })).filter(({ candidate }) => !candidate.disabled); const target = event.key === "Home" ? candidates[0] : candidates.at(-1); if (target) optionRefs.current[target.candidateIndex]?.focus(); }
      if (event.key === "Escape") { event.preventDefault(); setOpen(false); root.current?.querySelector<HTMLButtonElement>(".select-trigger")?.focus(); }
    }}>
      {option.avatar && <span className="option-avatar">{option.avatar}</span>}
      <span className="option-copy"><span className="option-title">{option.title}</span>{option.meta && <span className="option-meta">{option.meta}</span>}</span>
      {option.state && <span className={`option-state ${option.stateKind ?? ""}`}>{option.state}</span>}
      <span className="option-check">{option.value === value && <Icon name="check" className="small" />}</span>
    </button>)}</div>}
  </div>;
}

function unavailableDetail(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("host not found") || message.includes("Specified native messaging host")) return "未找到本地连接，请运行 aferry setup。";
  if (message.includes("forbidden")) return "当前浏览器尚未完成连接授权，请重新运行 aferry setup。";
  return `本地连接不可用：${message}`;
}

function targetStateLabel(state: TargetState): string {
  return ({ ready: "可用", credential_missing: "凭据缺失", authentication_failed: "认证失败", connection_failed: "无法连接", incompatible: "能力不兼容" })[state];
}

function targetLabel(kind: TargetKind): { label: string; avatar: string; local: boolean } {
  if (kind === "local_open_code") return { label: "OpenCode", avatar: "OC", local: true };
  if (kind === "local_claude_code") return { label: "Claude Code", avatar: "CC", local: true };
  if (kind === "local_codex_cli") return { label: "Codex CLI", avatar: "CX", local: true };
  if (kind === "local_codex_app") return { label: "Codex App", avatar: "CA", local: true };
  return { label: "Hermes", avatar: "H", local: false };
}

function pdfProgressLabel(progress: ArxivPdfProgress): string {
  if (progress.stage === "opening") return "正在读取 PDF 文档结构…";
  if (progress.stage === "extracting") return `正在提取 PDF · ${progress.completed_pages}/${progress.total_pages} 页`;
  const loaded = (progress.loaded_bytes / 1024 / 1024).toFixed(1);
  return progress.total_bytes ? `正在下载 PDF · ${loaded}/${(progress.total_bytes / 1024 / 1024).toFixed(1)} MiB` : `正在下载 PDF · ${loaded} MiB`;
}

async function nativeRequest<T>(command: Record<string, unknown>): Promise<T> {
  const response = await browser.runtime.sendNativeMessage(NATIVE_HOST_NAME, { protocol_version: PROTOCOL_VERSION, request_id: crypto.randomUUID(), command }) as HostResponse<T>;
  if (response.protocol_version !== PROTOCOL_VERSION) throw new Error("协议版本不兼容，请升级 Agent Ferry");
  if (response.error) throw new Error(response.error.message);
  if (response.result === undefined) throw new Error("本地服务没有返回结果");
  return response.result;
}

function Header({ view, connection, onSettings, onBack }: { view: View; connection: ConnectionState; onSettings(): void; onBack(): void }) {
  const copy = view === "send" ? ["Agent Ferry", "发送当前页面"] : view === "history" ? ["历史", "任务会在后台继续运行"] : view === "detail" ? ["任务详情", "执行阶段与 Agent 输出"] : ["设置", "连接、Agent 与 Prompt"];
  return <header className="app-header">
    {view === "detail" ? <><button className="back-button" type="button" aria-label="返回历史" onClick={onBack}><Icon name="arrow-left" /></button><div className="header-title-group"><h1 className="screen-title">{copy[0]}</h1><p className="screen-caption">{copy[1]}</p></div></> : <div className="brand"><div className="brand-mark">AF</div><div className={view === "send" ? "brand-copy" : "header-title-group"}><h1 className={view === "send" ? "brand-name" : "screen-title"}>{copy[0]}</h1><p className={view === "send" ? "brand-caption" : "screen-caption"}>{copy[1]}</p></div></div>}
    <button className={`connection-status connection-${connection.kind}`} type="button" aria-label="查看本地连接设置" onClick={onSettings}><span className="status-dot"/><span className="status-tooltip">{connection.kind === "ready" ? `本地连接已就绪 · v${connection.result.core_version}` : connection.kind === "checking" ? "正在检查本地连接" : connection.detail}</span></button>
  </header>;
}

function BottomNav({ view, running, onNavigate }: { view: View; running: number; onNavigate(view: View): void }) {
  return <nav className="bottom-nav" aria-label="主要导航">
    {(["send", "history", "settings"] as const).map((item) => <button className={`nav-item ${(view === item || view === "detail" && item === "history") ? "active" : ""}`} type="button" key={item} onClick={() => onNavigate(item)}>
      <Icon name={item}/><span>{item === "send" ? "发送" : item === "history" ? "历史" : "设置"}</span>{item === "history" && running > 0 && <span className="nav-badge">{running}</span>}
    </button>)}
  </nav>;
}

function formatRelative(timestamp: number): string {
  const delta = Math.max(0, Date.now() - timestamp);
  if (delta < 60_000) return "刚刚";
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)} 分钟前`;
  const date = new Date(timestamp);
  return date.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" }) + " " + date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

function durationText(task: TaskSummary): string {
  const end = task.completed_at_ms ?? Date.now();
  const seconds = Math.max(0, Math.round((end - task.created_at_ms) / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  return `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
}

function historyGroup(state: HistoryState): "running" | "completed" | "failed" {
  if (state === "running") return "running";
  if (state === "completed") return "completed";
  return "failed";
}

function App() {
  const [view, setView] = useState<View>("send");
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });
  const [page, setPage] = useState({ title: "正在读取当前页面", domain: "", url: "" });
  const [selectedAgent, setSelectedAgent] = useState("");
  const [workspaceByAgent, setWorkspaceByAgent] = useState<Record<string, string>>({});
  const [templateSettings, setTemplateSettings] = useState<PromptTemplateSettings>(EMPTY_PROMPT_TEMPLATE_SETTINGS);
  const [submit, setSubmit] = useState<SubmitState>({ kind: "idle" });
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [historyFilter, setHistoryFilter] = useState<"running" | "completed" | "failed">("running");
  const [selectedTaskId, setSelectedTaskId] = useState("");
  const [taskDetail, setTaskDetail] = useState<TaskRecord | null>(null);
  const [historyError, setHistoryError] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [workspaceError, setWorkspaceError] = useState("");
  const [templateDraft, setTemplateDraft] = useState<TemplateDraft | null>(null);
  const [templateError, setTemplateError] = useState("");

  const checkConnection = useCallback(async () => {
    setConnection({ kind: "checking" });
    try {
      const result = await nativeRequest<StatusResult>({ type: "status" });
      if (result.daemon !== "ready") throw new Error("Agent Ferry 后台服务尚未启动");
      setConnection({ kind: "ready", result });
      const stored = await browser.storage.local.get([SELECTED_AGENT_KEY, SELECTED_WORKSPACE_KEY]);
      const readyTargets = (result.targets ?? []).filter((target) => target.state === "ready");
      const fallback = readyTargets[0];
      const preferred = typeof stored[SELECTED_AGENT_KEY] === "string" ? stored[SELECTED_AGENT_KEY] as string : "";
      const possible = readyTargets.some((target) => target.id === preferred || target.kind === preferred);
      setSelectedAgent(possible ? preferred : fallback?.kind === "remote_hermes" ? fallback.id : fallback?.kind ?? "");
      if (stored[SELECTED_WORKSPACE_KEY] && typeof stored[SELECTED_WORKSPACE_KEY] === "object") setWorkspaceByAgent(stored[SELECTED_WORKSPACE_KEY] as Record<string, string>);
    } catch (error) {
      setConnection({ kind: "unavailable", detail: unavailableDetail(error) });
    }
  }, []);

  const refreshHistory = useCallback(async () => {
    if (connection.kind !== "ready") return;
    try {
      const result = await nativeRequest<{ tasks: TaskSummary[] }>({ type: "history_list", state: null, limit: 200 });
      setTasks(result.tasks);
      setHistoryError("");
    } catch (error) {
      setHistoryError(error instanceof Error ? error.message : String(error));
    }
  }, [connection.kind]);

  const refreshDetail = useCallback(async () => {
    if (!selectedTaskId || connection.kind !== "ready") return;
    try {
      const result = await nativeRequest<{ task: TaskRecord | null }>({ type: "history_get", task_id: selectedTaskId });
      setTaskDetail(result.task);
      setHistoryError(result.task ? "" : "未找到这个任务");
    } catch (error) {
      setHistoryError(error instanceof Error ? error.message : String(error));
    }
  }, [connection.kind, selectedTaskId]);

  useEffect(() => { void checkConnection(); }, [checkConnection]);
  useEffect(() => {
    void browser.tabs.query({ active: true, currentWindow: true }).then(([tab]) => {
      const url = tab.url ?? "";
      let domain = "当前页面不可提取";
      try { domain = new URL(url).hostname; } catch { /* 非 http(s) 页面保留提示。 */ }
      setPage({ title: tab.title || "当前页面", domain, url });
    });
    void loadPromptTemplateSettings(browser.storage.local).then(setTemplateSettings).catch((error: unknown) => setTemplateError(String(error)));
  }, []);
  useEffect(() => { void refreshHistory(); }, [refreshHistory]);
  useEffect(() => {
    if (connection.kind !== "ready") return;
    const timer = window.setInterval(() => void refreshHistory(), view === "history" ? 1500 : 5000);
    return () => window.clearInterval(timer);
  }, [connection.kind, refreshHistory, view]);
  useEffect(() => {
    if (view !== "detail") return;
    void refreshDetail();
    const timer = window.setInterval(() => void refreshDetail(), 1200);
    return () => window.clearInterval(timer);
  }, [refreshDetail, view]);

  const status = connection.kind === "ready" ? connection.result : null;
  const targets = status?.targets ?? [];
  const workspaces = status?.workspaces ?? [];
  const agentOptions = useMemo<SelectOption[]>(() => {
    const localKinds: TargetKind[] = ["local_open_code", "local_claude_code", "local_codex_cli", "local_codex_app"];
    const local = localKinds.flatMap((kind) => {
      const candidates = targets.filter((target) => target.kind === kind);
      if (!candidates.length) return [];
      const representative = candidates.find((target) => target.state === "ready") ?? candidates[0];
      const info = targetLabel(kind);
      return [{ value: kind, title: `本地 ${info.label}`, meta: "本机 · 实时输出", avatar: info.avatar, state: targetStateLabel(representative.state), stateKind: representative.state === "ready" ? "success" as const : "danger" as const, disabled: !candidates.some((target) => target.state === "ready") }];
    });
    const remote = targets.filter((target) => target.kind === "remote_hermes").map((target) => ({ value: target.id, title: target.name, meta: "远程 · 实时输出", avatar: "H", state: targetStateLabel(target.state), stateKind: target.state === "ready" ? "success" as const : "danger" as const, disabled: target.state !== "ready" }));
    return [...remote, ...local];
  }, [targets]);
  const isLocal = selectedAgent.startsWith("local_");
  const workspaceId = isLocal ? workspaceByAgent[selectedAgent] || workspaces.find((workspace) => workspace.ready)?.id || "" : "";
  const selectedTarget = useMemo(() => {
    if (!selectedAgent) return undefined;
    if (!isLocal) return targets.find((target) => target.id === selectedAgent && target.state === "ready");
    return targets.find((target) => target.kind === selectedAgent && target.id.endsWith(workspaceId) && target.state === "ready");
  }, [isLocal, selectedAgent, targets, workspaceId]);
  const workspaceOptions = workspaces.map((workspace) => ({ value: workspace.id, title: workspace.name, meta: workspace.path, disabled: !workspace.ready }));
  const templateOptions: SelectOption[] = [{ value: "", title: "默认分析" }, ...templateSettings.templates.map((template) => ({ value: template.id, title: template.name }))];
  const runningCount = tasks.filter((task) => task.state === "running").length;

  const changeAgent = async (value: string) => {
    setSelectedAgent(value);
    await browser.storage.local.set({ [SELECTED_AGENT_KEY]: value });
  };
  const changeWorkspace = async (value: string) => {
    const next = { ...workspaceByAgent, [selectedAgent]: value };
    setWorkspaceByAgent(next);
    await browser.storage.local.set({ [SELECTED_WORKSPACE_KEY]: next });
  };
  const selectTemplate = async (id: string) => {
    const next = { ...templateSettings, selected_template_id: id || null };
    setTemplateSettings(next);
    await persistPromptTemplateSettings(browser.storage.local, next);
  };

  const startHandoff = useCallback(async () => {
    if (!selectedTarget) return;
    setSubmit({ kind: "busy", label: "正在读取当前页面…" });
    try {
      const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
      if (!tab.id || !tab.url?.match(/^https?:\/\//)) throw new Error("当前页面不是可提取的 http(s) 页面");
      const chromeApi = (globalThis as typeof globalThis & { chrome: ChromeScriptingApi }).chrome;
      let source: CapturedPage;
      if (isArxivPdfUrl(tab.url)) {
        source = await extractArxivPdf(tab.url, fetch, (progress) => setSubmit({ kind: "busy", label: pdfProgressLabel(progress) }));
      } else {
        const results = await chromeApi.scripting.executeScript({ target: { tabId: tab.id }, files: ["/content-scripts/extract-page.js"] });
        const injection = results[0];
        const captureResult = injection?.result as CapturedPageResult | undefined;
        if (!captureResult) throw new Error(injection?.error || "页面提取没有返回正文");
        if ("error" in captureResult) throw new Error(captureResult.error);
        source = captureResult;
      }
      const transfer = await prepareHandoffTransfer(source.markdown, source.extractor === "x-thread" ? MIN_X_HANDOFF_CONTENT_BYTES : undefined);
      const { markdown: _markdown, ...sourceMetadata } = source;
      const requestId = crypto.randomUUID();
      const taskId = crypto.randomUUID();
      const port = browser.runtime.connectNative(NATIVE_HOST_NAME);
      let expectedAck = 0;
      let phase: "begin" | "chunk" | "events" = "begin";
      let endSent = false;
      const sendChunk = (index: number) => port.postMessage({ protocol_version: PROTOCOL_VERSION, request_id: requestId, command: { type: "handoff_chunk", task_id: taskId, index, data: transfer.chunks[index] } });
      port.onMessage.addListener((message: HostResponse | HandoffEvent | HandoffTransferAck) => {
        if (message.request_id !== requestId) return;
        if ("error" in message && message.error) {
          setSubmit({ kind: "error", label: message.error.message });
          port.disconnect();
          return;
        }
        if ("phase" in message) {
          const valid = phase === "begin" ? message.phase === "begin" && message.next_index === 0 : phase === "chunk" && message.phase === "chunk" && message.next_index === expectedAck;
          if (!valid) {
            setSubmit({ kind: "error", label: "正文传输顺序无效，请重新发送" });
            port.disconnect();
            return;
          }
          if (message.next_index < transfer.chunks.length) {
            phase = "chunk";
            expectedAck = message.next_index + 1;
            setSubmit({ kind: "busy", label: `正在传输正文 · ${message.next_index}/${transfer.chunks.length}` });
            sendChunk(message.next_index);
          } else if (!endSent) {
            endSent = true;
            phase = "events";
            setSubmit({ kind: "busy", label: "正在创建任务…" });
            port.postMessage({ protocol_version: PROTOCOL_VERSION, request_id: requestId, command: { type: "handoff_end", task_id: taskId } });
          }
          return;
        }
        if ("event" in message && message.task_id === taskId) {
          setSelectedTaskId(taskId);
          setTaskDetail(null);
          setSubmit({ kind: "idle" });
          setView("detail");
          void refreshHistory();
          port.disconnect();
        }
      });
      port.postMessage({ protocol_version: PROTOCOL_VERSION, request_id: requestId, command: { type: "handoff_begin", task_id: taskId, target_id: selectedTarget.id, prompt: effectivePrompt(templateSettings), source: sourceMetadata, total_bytes: transfer.totalBytes, total_chunks: transfer.chunks.length, sha256: transfer.sha256 } });
    } catch (error) {
      setSubmit({ kind: "error", label: error instanceof Error ? error.message : String(error) });
    }
  }, [refreshHistory, selectedTarget, templateSettings]);

  const updateWorkspace = async (command: Record<string, unknown>) => {
    setWorkspaceBusy(true);
    setWorkspaceError("");
    try {
      const result = await nativeRequest<StatusResult>(command);
      setConnection({ kind: "ready", result });
      setWorkspacePath("");
    } catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
    } finally {
      setWorkspaceBusy(false);
    }
  };
  const commitTemplate = async () => {
    if (!templateDraft) return;
    try {
      const next = saveTemplate(templateSettings, templateDraft, crypto.randomUUID());
      await persistPromptTemplateSettings(browser.storage.local, next);
      setTemplateSettings(next);
      setTemplateDraft(null);
      setTemplateError("");
    } catch (error) { setTemplateError(error instanceof Error ? error.message : String(error)); }
  };
  const removeTemplate = async (id: string) => {
    const next = deleteTemplate(templateSettings, id);
    await persistPromptTemplateSettings(browser.storage.local, next);
    setTemplateSettings(next);
  };
  const deleteHistory = async () => {
    if (!selectedTaskId || !window.confirm("删除这条任务记录？")) return;
    await nativeRequest<{ deleted: boolean }>({ type: "history_delete", task_id: selectedTaskId });
    setSelectedTaskId("");
    setTaskDetail(null);
    await refreshHistory();
    setView("history");
  };

  const navigate = (next: View) => { setView(next); if (next !== "detail") setHistoryError(""); };

  return <main className={`app-shell ${view !== "send" ? "tall" : ""}`}>
    <Header view={view} connection={connection} onSettings={() => navigate("settings")} onBack={() => navigate("history")}/>
    {view === "send" && <div className="main-content">
      <section className="page-summary" aria-label="当前页面"><div className="page-summary-icon"><Icon name="globe"/></div><div className="page-summary-copy"><div className="page-summary-title">{page.title}</div><div className="page-summary-meta">{page.domain}</div></div></section>
      <div className="form-stack">
        <div className="field"><label className="field-label"><span>Agent</span><span className="field-hint">{targets.filter((target) => target.state === "ready").length} 个可用</span></label><ProductSelect value={selectedAgent} options={agentOptions} label="选择 Agent" kind="agent" onChange={(value) => void changeAgent(value)}/></div>
        {isLocal && <div className="field workspace-field"><label className="field-label"><span>启动目录</span><span className="field-hint">本地 Agent</span></label>{workspaceOptions.length ? <ProductSelect value={workspaceId} options={workspaceOptions} label="选择本地 Agent 启动目录" kind="workspace" onChange={(value) => void changeWorkspace(value)}/> : <div className="field-empty"><span><strong>未配置启动目录</strong><small>请先在设置中添加</small></span><button type="button" onClick={() => navigate("settings")}>去设置</button></div>}</div>}
        <div className="field"><label className="field-label"><span>Prompt</span><span className="field-hint">仅发送所选模板</span></label><div className="control-row"><ProductSelect value={templateSettings.selected_template_id ?? ""} options={templateOptions} label="选择 Prompt 模板" kind="prompt" onChange={(value) => void selectTemplate(value)}/><button className="icon-button" type="button" aria-label="编辑 Prompt 模板" title="编辑 Prompt 模板" onClick={() => navigate("settings")}><Icon name="pencil"/></button></div></div>
      </div>
      <button className="primary-button send-button" type="button" aria-busy={submit.kind === "busy"} disabled={connection.kind !== "ready" || !selectedTarget || submit.kind === "busy"} onClick={() => void startHandoff()}>{submit.kind === "busy" ? submit.label : "发送当前页面"}</button>
      {submit.kind === "error" && <p className="submission-error" role="alert">{submit.label}</p>}
    </div>}
    {view === "history" && <div className="scroll-content"><div className="screen-intro"><h2 className="screen-heading">任务历史</h2><p className="screen-description">查看正在执行、已完成和失败的页面分析任务。</p></div><div className="segment-control" role="tablist" aria-label="任务状态">{(["running", "completed", "failed"] as const).map((filter) => <button className={`segment-button ${historyFilter === filter ? "active" : ""}`} type="button" role="tab" aria-selected={historyFilter === filter} key={filter} onClick={() => setHistoryFilter(filter)}>{filter === "running" ? "进行中" : filter === "completed" ? "已完成" : "失败"} <span>{tasks.filter((task) => historyGroup(task.state) === filter).length}</span></button>)}</div>
      <section className="task-list" aria-live="polite">{historyError ? <div className="empty-state">{historyError}</div> : tasks.filter((task) => historyGroup(task.state) === historyFilter).length === 0 ? <div className="empty-state">这里还没有任务。<br/>从发送页提交当前页面后，会自动出现在这里。</div> : tasks.filter((task) => historyGroup(task.state) === historyFilter).map((task) => { const group = historyGroup(task.state); return <button className="task-card" type="button" key={task.task_id} onClick={() => { setSelectedTaskId(task.task_id); setTaskDetail(null); setView("detail"); }}><div className="task-card-top"><div className={`task-status-icon ${group}`}><Icon name={group === "running" ? "loader" : group === "completed" ? "check-circle" : "x-circle"} className={group === "running" ? "spinning" : ""}/></div><div className="task-copy"><div className="task-title">{task.title}</div><div className="task-meta"><span>{task.site || new URL(task.url).hostname}</span><span>·</span><span>{task.target_name}</span></div></div><Icon name="chevron-right" className="small"/></div><div className="task-card-bottom"><span className={`status-label ${group}`}>{task.stage}</span><span>{group === "running" ? durationText(task) : formatRelative(task.updated_at_ms)}</span></div></button>; })}</section>
    </div>}
    {view === "detail" && <div className="scroll-content"><article className="detail-content">{!taskDetail ? <div className="empty-state">{historyError || "正在读取任务…"}</div> : <><div className="detail-status"><div className={`task-status-icon ${historyGroup(taskDetail.summary.state)}`}><Icon name={taskDetail.summary.state === "running" ? "loader" : taskDetail.summary.state === "completed" ? "check-circle" : "x-circle"} className={taskDetail.summary.state === "running" ? "spinning" : ""}/></div><div className="detail-status-copy"><div className="detail-status-title">{taskDetail.summary.state === "running" ? "正在运行" : taskDetail.summary.state === "completed" ? "任务已完成" : "任务失败"}</div><div className="detail-status-note">{taskDetail.summary.state === "running" ? taskDetail.summary.stage : `总用时 ${durationText(taskDetail.summary)}`}</div></div><span className="task-meta">{formatRelative(taskDetail.summary.created_at_ms)}</span></div>
      <div className="detail-heading"><h2>{taskDetail.summary.title}</h2><p>{taskDetail.summary.site || taskDetail.summary.url} · {taskDetail.summary.target_name}{taskDetail.summary.workspace_name ? ` · ${taskDetail.summary.workspace_name}` : ""}</p></div>
      <div className="phase-list">{taskDetail.events.filter((event) => event.event !== "output_delta").slice(-12).map((event, index, all) => <div className="phase-row" key={`${event.sequence}-${event.event}`}><span className={`phase-marker ${index < all.length - 1 || taskDetail.summary.state !== "running" ? "done" : "active"}`}><Icon name={event.event === "failed" ? "x" : index < all.length - 1 || taskDetail.summary.state !== "running" ? "check-circle" : "loader"} className="small"/></span><div><div className="phase-name">{event.event === "submitted" ? "提交任务" : event.event === "tool_started" ? "使用工具" : event.event === "completed" ? "完成分析" : event.event === "failed" ? "执行失败" : "Agent 分析"}</div><div className="phase-note">{event.text || taskDetail.summary.stage}</div></div><span className="phase-time">{new Date(event.timestamp_ms).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</span></div>)}</div>
      {(taskDetail.output || taskDetail.error) && <pre className={`output-panel ${taskDetail.error ? "output-error" : ""}`}>{taskDetail.error || taskDetail.output}{taskDetail.output_truncated ? "\n\n[历史输出已达到保存上限]" : ""}</pre>}
      <div className="detail-actions"><button className="danger-button" type="button" disabled={taskDetail.summary.state === "running"} onClick={() => void deleteHistory()}>删除记录</button><button className="primary-button" type="button" onClick={() => navigate("history")}>返回历史</button></div></>}</article></div>}
    {view === "settings" && <div className="scroll-content"><section className="settings-section"><div className="section-heading-row"><div><h2 className="section-heading">本地连接</h2><p className="section-help">浏览器与 Agent Ferry 后台服务的连接状态。</p></div></div><div className="connection-card"><div className={`task-status-icon ${connection.kind === "ready" ? "completed" : "failed"}`}><Icon name={connection.kind === "ready" ? "check-circle" : connection.kind === "checking" ? "loader" : "x-circle"} className={connection.kind === "checking" ? "spinning" : ""}/></div><div className="connection-copy"><div className="connection-title">{connection.kind === "ready" ? "本地连接已就绪" : connection.kind === "checking" ? "正在检查本地连接" : "本地连接不可用"}</div><div className="connection-meta">{connection.kind === "ready" ? `agentferryd v${connection.result.core_version} · 协议兼容` : connection.kind === "checking" ? "Chrome → Native Host → agentferryd" : connection.detail}</div></div><button className="text-button" type="button" onClick={() => void checkConnection()}><Icon name="refresh" className="small"/><span>重新检查</span></button></div></section>
      <section className="settings-section"><div className="section-heading-row"><div><h2 className="section-heading">Agent 与启动目录</h2><p className="section-help">目录名自动取路径最后一段，每个目录提供四个本地 Agent。</p></div></div><div className="workspace-list">{workspaces.map((workspace) => <div className="workspace-card" key={workspace.id}><div className="workspace-icon"><Icon name="folder"/></div><div className="workspace-copy"><div className="workspace-name">{workspace.name}</div><div className="workspace-path" title={workspace.path}>{workspace.path}</div><div className="workspace-agents">OpenCode · Claude Code · Codex CLI · Codex App</div></div><button className="row-action danger" type="button" disabled={workspaceBusy} aria-label={`删除启动目录 ${workspace.name}`} onClick={() => void updateWorkspace({ type: "workspace_remove", identifier: workspace.id })}><Icon name="trash"/></button></div>)}</div><div className="add-path-form"><input className="text-control" value={workspacePath} placeholder="/Users/name/projects/repository" onChange={(event) => setWorkspacePath(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.nextElementSibling instanceof HTMLButtonElement && event.currentTarget.nextElementSibling.click(); }}/><button className="secondary-button" type="button" disabled={workspaceBusy || !workspacePath.trim()} onClick={() => { const path = workspacePath.trim().replace(/\/+$/, ""); const name = path.split("/").filter(Boolean).at(-1) || path; void updateWorkspace({ type: "workspace_add", name, path }); }}><Icon name="plus"/><span>添加</span></button></div>{workspaceError && <p className="field-error visible">{workspaceError}</p>}</section>
      <section className="settings-section"><div className="section-heading-row"><div><h2 className="section-heading">Prompt 模板</h2><p className="section-help">发送页仅显示模板名称，内容在这里配置。</p></div><button className="text-button" type="button" onClick={() => setTemplateDraft({ name: "", content: DEFAULT_PROMPT })}><Icon name="plus" className="small"/><span>新建</span></button></div><div className="template-list"><div className="template-row"><div className="template-icon"><Icon name="file-text"/></div><div className="template-copy"><div className="template-name">默认分析</div><div className="template-meta">系统默认 · {DEFAULT_PROMPT.length} 字</div></div></div>{templateSettings.templates.map((template) => <div className="template-row" key={template.id}><div className="template-icon"><Icon name="file-text"/></div><div className="template-copy"><div className="template-name">{template.name}</div><div className="template-meta">{template.content.length} 字</div></div><button className="row-action" type="button" aria-label={`编辑模板 ${template.name}`} onClick={() => setTemplateDraft(template)}><Icon name="pencil"/></button><button className="row-action danger" type="button" aria-label={`删除模板 ${template.name}`} onClick={() => void removeTemplate(template.id)}><Icon name="trash"/></button></div>)}</div>{templateError && <p className="field-error visible">{templateError}</p>}</section>
    </div>}
    <BottomNav view={view} running={runningCount} onNavigate={navigate}/>
    {templateDraft && <div className="dialog-backdrop" role="presentation"><div className="template-dialog" role="dialog" aria-modal="true" aria-labelledby="template-dialog-title"><div className="dialog-header"><h2 className="dialog-title" id="template-dialog-title">{templateDraft.id ? "编辑 Prompt 模板" : "新建 Prompt 模板"}</h2><button className="row-action" type="button" aria-label="关闭" onClick={() => setTemplateDraft(null)}><Icon name="x"/></button></div><div className="dialog-body"><label className="field"><span className="field-label">模板名称</span><input className="text-control" maxLength={80} value={templateDraft.name} onChange={(event) => setTemplateDraft({ ...templateDraft, name: event.target.value })}/></label><label className="field"><span className="field-label">Prompt 内容</span><textarea className="textarea-control" value={templateDraft.content} onChange={(event) => setTemplateDraft({ ...templateDraft, content: event.target.value })}/></label></div><div className="dialog-actions"><button className="secondary-button" type="button" onClick={() => setTemplateDraft(null)}>取消</button><button className="primary-button" type="button" onClick={() => void commitTemplate()}>保存模板</button></div></div></div>}
  </main>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App/></React.StrictMode>);
