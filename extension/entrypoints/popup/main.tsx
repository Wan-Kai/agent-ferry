import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import type { CapturedPage, CapturedPageResult } from "../extract-page";
import { extractArxivPdf, isArxivPdfUrl, type ArxivPdfProgress } from "../../lib/arxiv-pdf-extractor";
import {
  buildAgentProducts,
  buildRunLocations,
  migrateLocationSelections,
  migrateSelectedProduct,
  targetStateLabel,
  type AgentProductId,
} from "../../lib/agent-selection";
import { MIN_X_HANDOFF_CONTENT_BYTES, prepareHandoffTransfer } from "../../lib/handoff-transfer";
import { filterArchiveTasks, selectRecentCompleted, taskStateGroup, type ArchiveStateFilter } from "../../lib/history-view";
import {
  DEFAULT_PROMPT,
  EMPTY_PROMPT_TEMPLATE_SETTINGS,
  MAX_PROMPT_LENGTH,
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
const SELECTED_AGENT_PRODUCT_KEY = "selected_agent_product_v2";
const SELECTED_LOCATION_BY_AGENT_KEY = "selected_location_by_agent_v2";
const PINNED_TASKS_KEY = "pinned_history_tasks_v1";
const RECENT_COMPLETED_LIMIT = 6;

type TargetKind = "remote_hermes" | "local_open_code" | "local_claude_code" | "local_codex_cli" | "local_codex_app";
type TargetState = "ready" | "credential_missing" | "authentication_failed" | "connection_failed" | "incompatible";
type HistoryState = "running" | "completed" | "failed" | "cancelled" | "interrupted";
type View = "send" | "history" | "archive" | "settings" | "detail";

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
type SelectOption = { value: string; title: string; meta?: string; triggerMeta?: string; icon?: string; leadingIcon?: string; state?: string; stateKind?: "success" | "danger"; disabled?: boolean };

const ICON_PATHS: Record<string, React.ReactNode> = {
  send: <><path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/></>,
  history: <><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></>,
  settings: <><path d="M20 7h-9"/><path d="M14 17H5"/><circle cx="17" cy="17" r="3"/><circle cx="7" cy="7" r="3"/></>,
  globe: <><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></>,
  cloud: <><path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/></>,
  monitor: <><rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/></>,
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
  archive: <><rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/></>,
  pin: <><path d="M12 17v5"/><path d="M5 17h14"/><path d="m6 3 1 7-3 4h16l-3-4 1-7Z"/></>,
  search: <><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></>,
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
  const optionHeight = kind === "run-location" ? 62 : 52;
  return <div className={`custom-select ${kind}-select ${open ? "open" : ""}`} ref={root} style={{ "--menu-space": open ? `${Math.min(options.length * optionHeight, 232) + 6}px` : "0px" } as React.CSSProperties}>
    <button className="select-trigger" type="button" role="combobox" aria-expanded={open} aria-haspopup="listbox" aria-label={label} onClick={() => setOpen((current) => !current)} onKeyDown={(event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setOpen(true);
      }
      if (event.key === "Escape") setOpen(false);
    }}>
      {selected?.icon && <img className="product-icon" src={selected.icon} alt=""/>}
      {selected?.leadingIcon && <span className="location-icon"><Icon name={selected.leadingIcon}/></span>}
      <span className="select-trigger-copy"><span className="select-trigger-title">{selected?.title ?? "请选择"}</span>{selected && ("triggerMeta" in selected ? selected.triggerMeta : selected.meta) && <span className="select-trigger-meta">{"triggerMeta" in selected ? selected.triggerMeta : selected.meta}</span>}</span>
      <Icon name="chevron-down" className="select-chevron" />
    </button>
    {open && <div className="select-menu" role="listbox">{options.map((option, index) => <button className="select-option" type="button" role="option" aria-selected={option.value === value} disabled={option.disabled} key={option.value} ref={(node) => { optionRefs.current[index] = node; }} onClick={() => selectOption(option)} onKeyDown={(event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); moveFocus(index, event.key === "ArrowDown" ? 1 : -1); }
      if (event.key === "Home" || event.key === "End") { event.preventDefault(); const candidates = options.map((candidate, candidateIndex) => ({ candidate, candidateIndex })).filter(({ candidate }) => !candidate.disabled); const target = event.key === "Home" ? candidates[0] : candidates.at(-1); if (target) optionRefs.current[target.candidateIndex]?.focus(); }
      if (event.key === "Escape") { event.preventDefault(); setOpen(false); root.current?.querySelector<HTMLButtonElement>(".select-trigger")?.focus(); }
    }}>
      {option.icon && <span className="option-product-icon"><img src={option.icon} alt=""/></span>}
      {option.leadingIcon && <span className="option-location-icon"><Icon name={option.leadingIcon}/></span>}
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
  const copy = view === "send" ? ["Agent Ferry", "让网页进入 Agent 的上下文"] : view === "history" ? ["历史", "任务会在后台继续运行"] : view === "archive" ? ["任务档案", "搜索全部历史记录"] : view === "detail" ? ["任务详情", "执行阶段与 Agent 输出"] : ["设置", "连接、Agent 与 Prompt"];
  const isSubView = view === "detail" || view === "archive";
  return <header className={`app-header ${view === "send" ? "send-header" : ""}`}>
    {isSubView ? <><button className="back-button" type="button" aria-label="返回上一页" onClick={onBack}><Icon name="arrow-left" /></button><div className="header-title-group"><h1 className="screen-title">{copy[0]}</h1><p className="screen-caption">{copy[1]}</p></div></> : <div className={`brand ${view === "send" ? "send-brand" : ""}`}><div className="brand-mark"><img src="/icons/icon-48.png" alt="" /></div><div className={view === "send" ? "brand-copy" : "header-title-group"}><h1 className={view === "send" ? "brand-name" : "screen-title"}>{copy[0]}</h1><p className={view === "send" ? "brand-caption" : "screen-caption"}>{copy[1]}</p></div></div>}
    <button className={`connection-status connection-${connection.kind}`} type="button" aria-label="查看本地连接设置" onClick={onSettings}><span className="status-dot"/><span className="status-tooltip">{connection.kind === "ready" ? `本地连接已就绪 · v${connection.result.core_version}` : connection.kind === "checking" ? "正在检查本地连接" : connection.detail}</span></button>
  </header>;
}

function BottomNav({ view, running, onNavigate }: { view: View; running: number; onNavigate(view: View): void }) {
  return <nav className="bottom-nav" aria-label="主要导航">
    {(["send", "history", "settings"] as const).map((item) => <button className={`nav-item ${(view === item || (view === "detail" || view === "archive") && item === "history") ? "active" : ""}`} type="button" key={item} onClick={() => onNavigate(item)}>
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

const historyGroup = taskStateGroup;

function App() {
  const [view, setView] = useState<View>("send");
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });
  const [page, setPage] = useState({ title: "正在读取当前页面", domain: "", url: "", favicon: "" });
  const [faviconFailed, setFaviconFailed] = useState(false);
  const [selectedProduct, setSelectedProduct] = useState<AgentProductId | "">("");
  const [locationByProduct, setLocationByProduct] = useState<Partial<Record<AgentProductId, string>>>({});
  const [templateSettings, setTemplateSettings] = useState<PromptTemplateSettings>(EMPTY_PROMPT_TEMPLATE_SETTINGS);
  const [finalPrompt, setFinalPrompt] = useState(DEFAULT_PROMPT);
  const [submit, setSubmit] = useState<SubmitState>({ kind: "idle" });
  const [tasks, setTasks] = useState<TaskSummary[]>([]);
  const [historyFilter, setHistoryFilter] = useState<"running" | "completed" | "failed">("running");
  const [pinnedTaskIds, setPinnedTaskIds] = useState<Set<string>>(new Set());
  const [archiveQuery, setArchiveQuery] = useState("");
  const [archiveState, setArchiveState] = useState<ArchiveStateFilter>("all");
  const [archiveTarget, setArchiveTarget] = useState("");
  const [selectedTaskId, setSelectedTaskId] = useState("");
  const [detailReturnView, setDetailReturnView] = useState<"history" | "archive">("history");
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
      const stored = await browser.storage.local.get([
        SELECTED_AGENT_PRODUCT_KEY,
        SELECTED_LOCATION_BY_AGENT_KEY,
        SELECTED_AGENT_KEY,
        SELECTED_WORKSPACE_KEY,
      ]);
      const preferred = typeof stored[SELECTED_AGENT_PRODUCT_KEY] === "string"
        ? stored[SELECTED_AGENT_PRODUCT_KEY] as string
        : typeof stored[SELECTED_AGENT_KEY] === "string" ? stored[SELECTED_AGENT_KEY] as string : "";
      const product = migrateSelectedProduct(preferred, result.targets ?? []);
      const fallback = buildAgentProducts(result.targets ?? []).find((candidate) => candidate.ready)?.id ?? "hermes";
      const resolvedProduct = product || fallback;
      setSelectedProduct(resolvedProduct);
      await browser.storage.local.set({ [SELECTED_AGENT_PRODUCT_KEY]: resolvedProduct });
      const storedLocations = stored[SELECTED_LOCATION_BY_AGENT_KEY];
      if (storedLocations && typeof storedLocations === "object") {
        setLocationByProduct(storedLocations as Partial<Record<AgentProductId, string>>);
      } else {
        const legacyWorkspaces = stored[SELECTED_WORKSPACE_KEY] && typeof stored[SELECTED_WORKSPACE_KEY] === "object"
          ? stored[SELECTED_WORKSPACE_KEY] as Record<string, string>
          : {};
        const migrated = migrateLocationSelections(
          typeof stored[SELECTED_AGENT_KEY] === "string" ? stored[SELECTED_AGENT_KEY] as string : "",
          legacyWorkspaces,
          result.targets ?? [],
        );
        setLocationByProduct(migrated);
        await browser.storage.local.set({ [SELECTED_LOCATION_BY_AGENT_KEY]: migrated });
      }
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
      setPage({ title: tab.title || "当前页面", domain, url, favicon: tab.favIconUrl ?? "" });
      setFaviconFailed(false);
    });
    void loadPromptTemplateSettings(browser.storage.local).then((settings) => {
      setTemplateSettings(settings);
      setFinalPrompt(effectivePrompt(settings));
    }).catch((error: unknown) => setTemplateError(String(error)));
    void browser.storage.local.get(PINNED_TASKS_KEY).then((stored) => {
      const ids = stored[PINNED_TASKS_KEY];
      if (Array.isArray(ids)) setPinnedTaskIds(new Set(ids.filter((id): id is string => typeof id === "string")));
    });
  }, []);
  useEffect(() => { void refreshHistory(); }, [refreshHistory]);
  useEffect(() => {
    if (connection.kind !== "ready") return;
    const timer = window.setInterval(() => void refreshHistory(), view === "history" || view === "archive" ? 1500 : 5000);
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
  const products = useMemo(() => buildAgentProducts(targets), [targets]);
  const runLocations = useMemo(() => selectedProduct ? buildRunLocations(selectedProduct, targets, workspaces) : [], [selectedProduct, targets, workspaces]);
  const selectedLocationId = selectedProduct && runLocations.some((location) => location.id === locationByProduct[selectedProduct])
    ? locationByProduct[selectedProduct] ?? ""
    : runLocations.find((location) => !location.disabled)?.id ?? "";
  const selectedLocation = runLocations.find((location) => location.id === selectedLocationId);
  const selectedTarget = selectedLocation?.targetId
    ? targets.find((target) => target.id === selectedLocation.targetId && target.state === "ready")
    : undefined;
  const agentOptions: SelectOption[] = products.map((product) => ({
    value: product.id,
    title: product.title,
    icon: product.icon,
    state: product.ready
      ? "可用"
      : (() => {
          const detected = buildRunLocations(product.id, targets, workspaces).find((location) => location.targetId);
          return detected ? targetStateLabel(detected.state) : "未检测到";
        })(),
    stateKind: product.ready ? "success" : "danger",
    // 产品是用户理解任务去向的第一层，即使暂不可用也应允许进入查看具体原因；只在运行位置层阻止提交。
    disabled: false,
  }));
  const locationOptions: SelectOption[] = runLocations.map((location) => ({
    value: location.id,
    title: location.title,
    meta: location.meta,
    triggerMeta: "",
    leadingIcon: location.locality === "remote" ? "cloud" : "monitor",
    state: targetStateLabel(location.state),
    stateKind: location.disabled ? "danger" : "success",
    disabled: location.disabled,
  }));
  const templateOptions: SelectOption[] = [{ value: "", title: "默认分析" }, ...templateSettings.templates.map((template) => ({ value: template.id, title: template.name }))];
  const selectedProductTitle = products.find((product) => product.id === selectedProduct)?.title ?? "Agent";
  const finalPromptBytes = new TextEncoder().encode(finalPrompt).byteLength;
  const promptInvalid = !finalPrompt.trim() || finalPromptBytes > MAX_PROMPT_LENGTH;
  const sendButtonLabel = selectedTarget && selectedLocation ? "开始任务" : "请选择可用的运行位置";
  const dataTransferNote = selectedTarget && selectedLocation
    ? selectedLocation.locality === "remote"
      ? `点击后将提取当前页正文和上方任务指令，并发送到你配置的 ${selectedProductTitle} · ${selectedLocation.title}。`
      : `点击后将提取当前页正文和上方任务指令，并交给本机 ${selectedProductTitle} 在 ${selectedLocation.title} 中处理。`
    : "请选择 Agent 和运行位置；页面正文只会在你点击开始后提取和发送。";
  const runningCount = tasks.filter((task) => task.state === "running").length;
  const completedCount = tasks.filter((task) => task.state === "completed").length;
  const recentCompleted = useMemo(() => selectRecentCompleted(tasks, pinnedTaskIds, RECENT_COMPLETED_LIMIT), [pinnedTaskIds, tasks]);
  const recentTasks = historyFilter === "completed" ? recentCompleted : tasks.filter((task) => historyGroup(task.state) === historyFilter);
  const archiveTargets = useMemo(() => [...new Set(tasks.map((task) => task.target_name))].sort((left, right) => left.localeCompare(right, "zh-CN")), [tasks]);
  const archiveTasks = useMemo(() => filterArchiveTasks(tasks, archiveQuery, archiveState, archiveTarget), [archiveQuery, archiveState, archiveTarget, tasks]);

  const changeProduct = async (value: string) => {
    const product = value as AgentProductId;
    setSelectedProduct(product);
    const locations = buildRunLocations(product, targets, workspaces);
    const nextLocation = locationByProduct[product] && locations.some((location) => location.id === locationByProduct[product] && !location.disabled)
      ? locationByProduct[product]
      : locations.find((location) => !location.disabled)?.id;
    const next = nextLocation ? { ...locationByProduct, [product]: nextLocation } : locationByProduct;
    setLocationByProduct(next);
    await browser.storage.local.set({ [SELECTED_AGENT_PRODUCT_KEY]: product, [SELECTED_LOCATION_BY_AGENT_KEY]: next });
  };
  const changeLocation = async (value: string) => {
    if (!selectedProduct) return;
    const next = { ...locationByProduct, [selectedProduct]: value };
    setLocationByProduct(next);
    await browser.storage.local.set({ [SELECTED_LOCATION_BY_AGENT_KEY]: next });
  };
  const selectTemplate = async (id: string) => {
    const next = { ...templateSettings, selected_template_id: id || null };
    setTemplateSettings(next);
    setFinalPrompt(effectivePrompt(next));
    await persistPromptTemplateSettings(browser.storage.local, next);
  };
  const togglePinned = async (taskId: string) => {
    const next = new Set(pinnedTaskIds);
    if (next.has(taskId)) next.delete(taskId); else next.add(taskId);
    setPinnedTaskIds(next);
    await browser.storage.local.set({ [PINNED_TASKS_KEY]: [...next] });
  };
  const openTask = (taskId: string, returnView: "history" | "archive") => {
    setSelectedTaskId(taskId);
    setTaskDetail(null);
    setDetailReturnView(returnView);
    setView("detail");
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
        const results = await chromeApi.scripting.executeScript({ target: { tabId: tab.id }, files: ["/extract-page.js"] });
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
          setDetailReturnView("history");
          setSubmit({ kind: "idle" });
          setView("detail");
          void refreshHistory();
          port.disconnect();
        }
      });
      port.postMessage({ protocol_version: PROTOCOL_VERSION, request_id: requestId, command: { type: "handoff_begin", task_id: taskId, target_id: selectedTarget.id, prompt: finalPrompt, source: sourceMetadata, total_bytes: transfer.totalBytes, total_chunks: transfer.chunks.length, sha256: transfer.sha256 } });
    } catch (error) {
      setSubmit({ kind: "error", label: error instanceof Error ? error.message : String(error) });
    }
  }, [finalPrompt, refreshHistory, selectedTarget]);

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
    setView(detailReturnView);
  };

  const navigate = (next: View) => { setView(next); if (next !== "detail") setHistoryError(""); };

  return <main className={`app-shell ${view !== "send" ? "tall" : ""}`}>
    <Header view={view} connection={connection} onSettings={() => navigate("settings")} onBack={() => navigate(view === "archive" ? "history" : detailReturnView)}/>
    {view === "send" && <div className="main-content">
      <section className="page-summary" aria-label="当前页面"><div className="page-summary-icon">{page.favicon && !faviconFailed ? <img src={page.favicon} alt="" onError={() => setFaviconFailed(true)}/> : <Icon name="globe"/>}</div><div className="page-summary-copy"><div className="page-summary-title">{page.title}</div><div className="page-summary-meta">{page.domain}</div></div></section>
      <div className="form-stack">
        <div className="field agent-destination-field"><label className="field-label"><span>Agent</span><span className="field-hint">{products.filter((product) => product.ready).length} 个产品可用</span></label><div className="handoff-pair"><div className="handoff-pair-cell product-cell"><span className="pair-label">产品</span><ProductSelect value={selectedProduct} options={agentOptions} label="选择 Agent 产品" kind="agent-product" onChange={(value) => void changeProduct(value)}/></div><div className="handoff-pair-cell location-cell"><span className="pair-label">运行于</span><ProductSelect value={selectedLocationId} options={locationOptions} label="选择运行位置" kind="run-location" onChange={(value) => void changeLocation(value)}/></div></div></div>
        <div className="field prompt-field"><label className="field-label"><span>任务指令</span><span className="field-hint">最终内容将原样发送</span></label><ProductSelect value={templateSettings.selected_template_id ?? ""} options={templateOptions} label="选择 Prompt 模板" kind="prompt" onChange={(value) => void selectTemplate(value)}/><textarea className="textarea-control prompt-editor" aria-label="最终 Prompt" value={finalPrompt} onChange={(event) => setFinalPrompt(event.target.value)}/><div className={`prompt-note ${promptInvalid ? "invalid" : ""}`}>{!finalPrompt.trim() ? "Prompt 不能为空" : finalPromptBytes > MAX_PROMPT_LENGTH ? "Prompt 不能超过 16 KiB" : "本次修改仅用于当前任务"}</div></div>
      </div>
      <p className="data-transfer-note">{dataTransferNote}</p>
      <button className="primary-button send-button" type="button" aria-busy={submit.kind === "busy"} disabled={connection.kind !== "ready" || !selectedTarget || promptInvalid || submit.kind === "busy"} onClick={() => void startHandoff()}>{submit.kind === "busy" ? submit.label : sendButtonLabel}</button>
      {submit.kind === "error" && <p className="submission-error" role="alert">{submit.label}</p>}
    </div>}
    {view === "history" && <div className="scroll-content"><div className="screen-intro screen-intro-row"><div><h2 className="screen-heading">近期任务</h2><p className="screen-description">关注正在执行和刚刚结束的任务。</p></div><button className="archive-shortcut" type="button" onClick={() => navigate("archive")}><Icon name="archive"/><span>任务档案</span></button></div><div className="segment-control" role="tablist" aria-label="任务状态">{(["running", "completed", "failed"] as const).map((filter) => <button className={`segment-button ${historyFilter === filter ? "active" : ""}`} type="button" role="tab" aria-selected={historyFilter === filter} key={filter} onClick={() => setHistoryFilter(filter)}>{filter === "running" ? "进行中" : filter === "completed" ? "已完成" : "失败"} <span>{tasks.filter((task) => historyGroup(task.state) === filter).length}</span></button>)}</div>
      <section className="task-list" aria-live="polite">{historyError ? <div className="empty-state">{historyError}</div> : recentTasks.length === 0 ? <div className="empty-state">这里还没有任务。<br/>从发送页提交当前页面后，会自动出现在这里。</div> : recentTasks.map((task) => { const group = historyGroup(task.state); const pinned = pinnedTaskIds.has(task.task_id); return <article className="task-card" key={task.task_id}><button className="task-card-open" type="button" onClick={() => openTask(task.task_id, "history")}><div className="task-card-top"><div className={`task-status-icon ${group}`}><Icon name={group === "running" ? "loader" : group === "completed" ? "check-circle" : "x-circle"} className={group === "running" ? "spinning" : ""}/></div><div className="task-copy"><div className="task-title">{task.title}</div><div className="task-meta"><span>{task.site || new URL(task.url).hostname}</span><span>·</span><span>{task.target_name}</span></div></div><Icon name="chevron-right" className="small"/></div><div className="task-card-bottom"><span className={`status-label ${group}`}>{task.stage}</span><span>{group === "running" ? durationText(task) : formatRelative(task.updated_at_ms)}</span></div></button><button className={`pin-button ${pinned ? "active" : ""}`} type="button" aria-label={pinned ? `取消关注 ${task.title}` : `关注 ${task.title}`} title={pinned ? "取消关注" : "保留在近期"} onClick={() => void togglePinned(task.task_id)}><Icon name="pin" className="small"/></button></article>; })}
        {historyFilter === "completed" && completedCount > 0 && <button className="archive-more-button" type="button" onClick={() => { setArchiveQuery(""); setArchiveTarget(""); setArchiveState("completed"); navigate("archive"); }}><span><strong>查看更多</strong><small>在任务档案中查看全部 {completedCount} 条已完成记录</small></span><Icon name="chevron-right"/></button>}
      </section>
    </div>}
    {view === "archive" && <div className="scroll-content archive-view"><div className="archive-tools"><label className="archive-search"><Icon name="search"/><span className="sr-only">搜索任务档案</span><input value={archiveQuery} placeholder="搜索标题、网址或 Agent" onChange={(event) => setArchiveQuery(event.target.value)}/>{archiveQuery && <button type="button" aria-label="清空搜索" onClick={() => setArchiveQuery("")}><Icon name="x" className="small"/></button>}</label><div className="archive-filters"><label><span className="sr-only">按状态筛选</span><select value={archiveState} onChange={(event) => setArchiveState(event.target.value as ArchiveStateFilter)}><option value="all">全部状态</option><option value="running">进行中</option><option value="completed">已完成</option><option value="failed">失败</option></select></label><label><span className="sr-only">按 Agent 筛选</span><select value={archiveTarget} onChange={(event) => setArchiveTarget(event.target.value)}><option value="">全部 Agent</option>{archiveTargets.map((target) => <option value={target} key={target}>{target}</option>)}</select></label></div><div className="archive-result-count">{archiveTasks.length} 条记录</div></div>
      <section className="archive-list" aria-live="polite">{historyError ? <div className="empty-state">{historyError}</div> : archiveTasks.length === 0 ? <div className="empty-state">没有找到匹配的任务。<br/>可以调整关键词或筛选条件。</div> : archiveTasks.map((task) => { const group = historyGroup(task.state); return <button className="archive-row" type="button" key={task.task_id} onClick={() => openTask(task.task_id, "archive")}><span className={`archive-state-dot ${group}`}/><span className="archive-row-copy"><strong>{task.title}</strong><small>{task.target_name}<span>·</span>{task.site || new URL(task.url).hostname}<span>·</span>{formatRelative(task.updated_at_ms)}</small></span><span className={`archive-state-label ${group}`}>{group === "running" ? "进行中" : group === "completed" ? "已完成" : "失败"}</span><Icon name="chevron-right" className="small"/></button>; })}</section>
    </div>}
    {view === "detail" && <div className="scroll-content"><article className="detail-content">{!taskDetail ? <div className="empty-state">{historyError || "正在读取任务…"}</div> : <><div className="detail-status"><div className={`task-status-icon ${historyGroup(taskDetail.summary.state)}`}><Icon name={taskDetail.summary.state === "running" ? "loader" : taskDetail.summary.state === "completed" ? "check-circle" : "x-circle"} className={taskDetail.summary.state === "running" ? "spinning" : ""}/></div><div className="detail-status-copy"><div className="detail-status-title">{taskDetail.summary.state === "running" ? "正在运行" : taskDetail.summary.state === "completed" ? "任务已完成" : "任务失败"}</div><div className="detail-status-note">{taskDetail.summary.state === "running" ? taskDetail.summary.stage : `总用时 ${durationText(taskDetail.summary)}`}</div></div><span className="task-meta">{formatRelative(taskDetail.summary.created_at_ms)}</span></div>
      <div className="detail-heading"><h2>{taskDetail.summary.title}</h2><p>{taskDetail.summary.site || taskDetail.summary.url} · {taskDetail.summary.target_name}{taskDetail.summary.workspace_name ? ` · ${taskDetail.summary.workspace_name}` : ""}</p></div>
      <div className="phase-list">{taskDetail.events.filter((event) => event.event !== "output_delta").slice(-12).map((event, index, all) => <div className="phase-row" key={`${event.sequence}-${event.event}`}><span className={`phase-marker ${index < all.length - 1 || taskDetail.summary.state !== "running" ? "done" : "active"}`}><Icon name={event.event === "failed" ? "x" : index < all.length - 1 || taskDetail.summary.state !== "running" ? "check-circle" : "loader"} className="small"/></span><div><div className="phase-name">{event.event === "submitted" ? "提交任务" : event.event === "tool_started" ? "使用工具" : event.event === "completed" ? "完成分析" : event.event === "failed" ? "执行失败" : "Agent 分析"}</div><div className="phase-note">{event.text || taskDetail.summary.stage}</div></div><span className="phase-time">{new Date(event.timestamp_ms).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}</span></div>)}</div>
      {(taskDetail.output || taskDetail.error) && <pre className={`output-panel ${taskDetail.error ? "output-error" : ""}`}>{taskDetail.error || taskDetail.output}{taskDetail.output_truncated ? "\n\n[历史输出已达到保存上限]" : ""}</pre>}
      <div className="detail-actions"><button className="danger-button" type="button" disabled={taskDetail.summary.state === "running"} onClick={() => void deleteHistory()}>删除记录</button><button className="secondary-button" type="button" onClick={() => void togglePinned(taskDetail.summary.task_id)}><Icon name="pin" className="small"/>{pinnedTaskIds.has(taskDetail.summary.task_id) ? "取消关注" : "关注"}</button><button className="primary-button" type="button" onClick={() => navigate(detailReturnView)}>返回</button></div></>}</article></div>}
    {view === "settings" && <div className="scroll-content"><section className="settings-section"><div className="section-heading-row"><div><h2 className="section-heading">本地连接</h2><p className="section-help">浏览器与 Agent Ferry 后台服务的连接状态。</p></div></div><div className="connection-card"><div className={`task-status-icon ${connection.kind === "ready" ? "completed" : "failed"}`}><Icon name={connection.kind === "ready" ? "check-circle" : connection.kind === "checking" ? "loader" : "x-circle"} className={connection.kind === "checking" ? "spinning" : ""}/></div><div className="connection-copy"><div className="connection-title">{connection.kind === "ready" ? "本地连接已就绪" : connection.kind === "checking" ? "正在检查本地连接" : "本地连接不可用"}</div><div className="connection-meta">{connection.kind === "ready" ? `agentferryd v${connection.result.core_version} · 协议兼容` : connection.kind === "checking" ? "Chrome → Native Host → agentferryd" : connection.detail}</div></div><button className="text-button" type="button" onClick={() => void checkConnection()}><Icon name="refresh" className="small"/><span>重新检查</span></button></div></section>
      <section className="settings-section"><div className="section-heading-row"><div><h2 className="section-heading">Agent 与启动目录</h2><p className="section-help">目录名自动取路径最后一段，每个目录提供四个本地 Agent。</p></div></div><div className="workspace-list">{workspaces.map((workspace) => <div className="workspace-card" key={workspace.id}><div className="workspace-icon"><Icon name="folder"/></div><div className="workspace-copy"><div className="workspace-name">{workspace.name}</div><div className="workspace-path" title={workspace.path}>{workspace.path}</div><div className="workspace-agents" aria-label="可用 Agent"><span title="OpenCode"><img src="/icons/agents/opencode.svg" alt=""/>OpenCode</span><span title="Claude Code"><img src="/icons/agents/claude.svg" alt=""/>Claude</span><span title="Codex CLI 与 Codex App"><img src="/icons/agents/codex.svg" alt=""/>Codex</span></div></div><button className="row-action danger" type="button" disabled={workspaceBusy} aria-label={`删除启动目录 ${workspace.name}`} onClick={() => void updateWorkspace({ type: "workspace_remove", identifier: workspace.id })}><Icon name="trash"/></button></div>)}</div><div className="add-path-form"><input className="text-control" value={workspacePath} placeholder="/Users/name/projects/repository" onChange={(event) => setWorkspacePath(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.nextElementSibling instanceof HTMLButtonElement && event.currentTarget.nextElementSibling.click(); }}/><button className="secondary-button" type="button" disabled={workspaceBusy || !workspacePath.trim()} onClick={() => { const path = workspacePath.trim().replace(/\/+$/, ""); const name = path.split("/").filter(Boolean).at(-1) || path; void updateWorkspace({ type: "workspace_add", name, path }); }}><Icon name="plus"/><span>添加</span></button></div>{workspaceError && <p className="field-error visible">{workspaceError}</p>}</section>
      <section className="settings-section"><div className="section-heading-row"><div><h2 className="section-heading">Prompt 模板</h2><p className="section-help">模板在这里管理；发送页可基于模板修改本次任务指令。</p></div><button className="text-button" type="button" onClick={() => setTemplateDraft({ name: "", content: DEFAULT_PROMPT })}><Icon name="plus" className="small"/><span>新建</span></button></div><div className="template-list"><div className="template-row"><div className="template-icon"><Icon name="file-text"/></div><div className="template-copy"><div className="template-name">默认分析</div><div className="template-meta">系统默认 · {DEFAULT_PROMPT.length} 字</div></div></div>{templateSettings.templates.map((template) => <div className="template-row" key={template.id}><div className="template-icon"><Icon name="file-text"/></div><div className="template-copy"><div className="template-name">{template.name}</div><div className="template-meta">{template.content.length} 字</div></div><button className="row-action" type="button" aria-label={`编辑模板 ${template.name}`} onClick={() => setTemplateDraft(template)}><Icon name="pencil"/></button><button className="row-action danger" type="button" aria-label={`删除模板 ${template.name}`} onClick={() => void removeTemplate(template.id)}><Icon name="trash"/></button></div>)}</div>{templateError && <p className="field-error visible">{templateError}</p>}</section>
    </div>}
    <BottomNav view={view} running={runningCount} onNavigate={navigate}/>
    {templateDraft && <div className="dialog-backdrop" role="presentation"><div className="template-dialog" role="dialog" aria-modal="true" aria-labelledby="template-dialog-title"><div className="dialog-header"><h2 className="dialog-title" id="template-dialog-title">{templateDraft.id ? "编辑 Prompt 模板" : "新建 Prompt 模板"}</h2><button className="row-action" type="button" aria-label="关闭" onClick={() => setTemplateDraft(null)}><Icon name="x"/></button></div><div className="dialog-body"><label className="field"><span className="field-label">模板名称</span><input className="text-control" maxLength={80} value={templateDraft.name} onChange={(event) => setTemplateDraft({ ...templateDraft, name: event.target.value })}/></label><label className="field"><span className="field-label">Prompt 内容</span><textarea className="textarea-control" value={templateDraft.content} onChange={(event) => setTemplateDraft({ ...templateDraft, content: event.target.value })}/></label></div><div className="dialog-actions"><button className="secondary-button" type="button" onClick={() => setTemplateDraft(null)}>取消</button><button className="primary-button" type="button" onClick={() => void commitTemplate()}>保存模板</button></div></div></div>}
  </main>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App/></React.StrictMode>);
