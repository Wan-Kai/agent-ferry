import React, { useCallback, useEffect, useMemo, useState } from "react";
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

type HandoffTargetStatus = {
  id: string;
  name: string;
  kind: "remote_hermes" | "local_open_code" | "local_claude_code" | "local_codex_cli" | "local_codex_app";
  state: "ready" | "credential_missing" | "authentication_failed" | "connection_failed" | "incompatible";
  capabilities: string[];
};
type LocalWorkspaceStatus = { id: string; name: string; path: string; ready: boolean };
type StatusResult = {
  core_version: string;
  daemon: "ready" | "not_detected";
  targets?: HandoffTargetStatus[];
  workspaces?: LocalWorkspaceStatus[];
};
type HostResponse = {
  protocol_version: number;
  request_id: string;
  result?: StatusResult;
  error?: { code: string; message: string; recoverable: boolean };
};
type HandoffEvent = {
  protocol_version: number;
  request_id: string;
  task_id: string;
  sequence: number;
  event: "submitted" | "running" | "output_delta" | "tool_started" | "tool_completed" | "completed" | "failed" | "cancelled";
  run_id?: string;
  text?: string;
};
type HandoffTransferAck = {
  protocol_version: number;
  request_id: string;
  task_id: string;
  phase: "begin" | "chunk";
  next_index: number;
};
type ConnectionState =
  | { kind: "checking" }
  | { kind: "ready"; result: StatusResult }
  | { kind: "unavailable"; detail: string };
type RunState = { kind: "idle" } | { kind: "capturing"; label: string } | { kind: "running" | "done" | "failed"; label: string; output: string };
type TemplateDraft = { id?: string; name: string; content: string };
type WorkspaceDraft = { name: string; path: string };
type ChromeScriptingApi = {
  scripting: {
    executeScript(options: { target: { tabId: number }; files: string[] }): Promise<Array<{ result?: unknown; error?: string }>>;
  };
};

function unavailableDetail(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("host not found") || message.includes("Specified native messaging host")) {
    return "未找到 Chrome Native Host。先运行 aferry setup，按提示完成注册。";
  }
  if (message.includes("forbidden")) return "当前扩展不在 Native Host allowlist 中，请重新执行注册命令。";
  return `本地连接不可用：${message}`;
}

function targetStateLabel(state: HandoffTargetStatus["state"]): string {
  return ({ ready: "可用", credential_missing: "凭据缺失", authentication_failed: "认证失败", connection_failed: "无法连接", incompatible: "能力不兼容" })[state];
}

function pdfProgressLabel(progress: ArxivPdfProgress): string {
  if (progress.stage === "opening") return "PDF 下载完成，正在读取文档结构…";
  if (progress.stage === "extracting") {
    return `正在提取 PDF 文本 ${progress.completed_pages}/${progress.total_pages} 页`;
  }
  const loaded = (progress.loaded_bytes / 1024 / 1024).toFixed(1);
  if (progress.total_bytes) {
    return `正在下载 PDF ${loaded}/${(progress.total_bytes / 1024 / 1024).toFixed(1)} MiB`;
  }
  return `正在下载 PDF ${loaded} MiB`;
}

function targetAgent(target?: HandoffTargetStatus): { label: string; avatar: string; local: boolean } {
  if (target?.kind === "local_open_code") return { label: "OpenCode", avatar: "OC", local: true };
  if (target?.kind === "local_claude_code") return { label: "Claude Code", avatar: "CC", local: true };
  if (target?.kind === "local_codex_cli") return { label: "Codex CLI", avatar: "CX", local: true };
  if (target?.kind === "local_codex_app") return { label: "Codex App", avatar: "CA", local: true };
  return { label: "Hermes", avatar: "H", local: false };
}

function App() {
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });
  const [selectedTarget, setSelectedTarget] = useState("");
  const [prompt, setPrompt] = useState(DEFAULT_PROMPT);
  const [run, setRun] = useState<RunState>({ kind: "idle" });
  const [templateSettings, setTemplateSettings] = useState<PromptTemplateSettings>(EMPTY_PROMPT_TEMPLATE_SETTINGS);
  const [templateDraft, setTemplateDraft] = useState<TemplateDraft | null>(null);
  const [templateError, setTemplateError] = useState("");
  const [workspaceDraft, setWorkspaceDraft] = useState<WorkspaceDraft>({ name: "", path: "" });
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [workspaceError, setWorkspaceError] = useState("");

  const readyTargets = useMemo(
    () => connection.kind === "ready" ? (connection.result.targets ?? []).filter((target) => target.state === "ready") : [],
    [connection],
  );
  const selectedTargetStatus = useMemo(
    () => connection.kind === "ready" ? connection.result.targets?.find((target) => target.id === selectedTarget) : undefined,
    [connection, selectedTarget],
  );

  const checkConnection = useCallback(async () => {
    setConnection({ kind: "checking" });
    try {
      const response = (await browser.runtime.sendNativeMessage(NATIVE_HOST_NAME, {
        protocol_version: PROTOCOL_VERSION,
        request_id: crypto.randomUUID(),
        command: { type: "status" },
      })) as HostResponse;
      if (response.protocol_version !== PROTOCOL_VERSION) {
        setConnection({ kind: "unavailable", detail: "协议版本不兼容，请升级 Agent Ferry。" });
      } else if (response.error) {
        setConnection({ kind: "unavailable", detail: response.error.message });
      } else if (response.result?.daemon === "ready") {
        setConnection({ kind: "ready", result: response.result });
        const firstReady = response.result.targets?.find((target) => target.state === "ready");
        setSelectedTarget((current) => response.result?.targets?.some((target) => target.id === current && target.state === "ready") ? current : firstReady?.id || "");
      } else {
        setConnection({ kind: "unavailable", detail: "daemon 尚未就绪，请启动 agentferryd。" });
      }
    } catch (error) {
      setConnection({ kind: "unavailable", detail: unavailableDetail(error) });
    }
  }, []);

  useEffect(() => { void checkConnection(); }, [checkConnection]);
  useEffect(() => {
    void loadPromptTemplateSettings(browser.storage.local)
      .then((settings) => {
        setTemplateSettings(settings);
        setPrompt(effectivePrompt(settings));
      })
      .catch((error: unknown) => setTemplateError(`无法读取模板：${error instanceof Error ? error.message : String(error)}`));
  }, []);

  const selectTemplate = useCallback(async (id: string) => {
    const next = { ...templateSettings, selected_template_id: id || null };
    setTemplateSettings(next);
    setPrompt(effectivePrompt(next));
    setTemplateDraft(null);
    setTemplateError("");
    try {
      await persistPromptTemplateSettings(browser.storage.local, next);
    } catch (error) {
      setTemplateError(`无法保存模板选择：${error instanceof Error ? error.message : String(error)}`);
    }
  }, [templateSettings]);

  const commitTemplate = useCallback(async () => {
    if (!templateDraft) return;
    try {
      const next = saveTemplate(templateSettings, templateDraft, crypto.randomUUID());
      await persistPromptTemplateSettings(browser.storage.local, next);
      setTemplateSettings(next);
      if (templateDraft.id === next.selected_template_id) setPrompt(effectivePrompt(next));
      setTemplateDraft(null);
      setTemplateError("");
    } catch (error) {
      setTemplateError(error instanceof Error ? error.message : String(error));
    }
  }, [templateDraft, templateSettings]);

  const removeSelectedTemplate = useCallback(async () => {
    const id = templateSettings.selected_template_id;
    if (!id) return;
    const next = deleteTemplate(templateSettings, id);
    try {
      await persistPromptTemplateSettings(browser.storage.local, next);
      setTemplateSettings(next);
      setPrompt(DEFAULT_PROMPT);
      setTemplateDraft(null);
      setTemplateError("");
    } catch (error) {
      setTemplateError(`无法删除模板：${error instanceof Error ? error.message : String(error)}`);
    }
  }, [templateSettings]);

  const updateWorkspaces = useCallback(async (command: { type: "workspace_add"; name: string; path: string } | { type: "workspace_remove"; identifier: string }) => {
    setWorkspaceBusy(true);
    setWorkspaceError("");
    try {
      const response = (await browser.runtime.sendNativeMessage(NATIVE_HOST_NAME, {
        protocol_version: PROTOCOL_VERSION,
        request_id: crypto.randomUUID(),
        command,
      })) as HostResponse;
      if (response.error) throw new Error(response.error.message);
      if (!response.result) throw new Error("本地服务没有返回更新后的启动目录");
      setConnection({ kind: "ready", result: response.result });
      const firstReady = response.result.targets?.find((target) => target.state === "ready");
      setSelectedTarget((current) => response.result?.targets?.some((target) => target.id === current && target.state === "ready") ? current : firstReady?.id || "");
      if (command.type === "workspace_add") setWorkspaceDraft({ name: "", path: "" });
    } catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
    } finally {
      setWorkspaceBusy(false);
    }
  }, []);

  const startHandoff = useCallback(async () => {
    if (!selectedTarget || !prompt.trim()) return;
    const agentLabel = targetAgent(selectedTargetStatus).label;
    setRun({ kind: "capturing", label: "正在读取当前页面…" });
    try {
      const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
      if (!tab.id || !tab.url?.match(/^https?:\/\//)) throw new Error("当前页面不是可提取的 http(s) 页面");
      // 当前首发平台明确是 Chrome MV3；原生命名空间比 polyfill 对 runtime content script 的支持更稳定。
      const chromeApi = (globalThis as typeof globalThis & { chrome: ChromeScriptingApi }).chrome;
      let source: CapturedPage;
      if (isArxivPdfUrl(tab.url)) {
        source = await extractArxivPdf(tab.url, fetch, (progress) => {
          setRun({ kind: "capturing", label: pdfProgressLabel(progress) });
        });
      } else {
        const results = await chromeApi.scripting.executeScript({
          target: { tabId: tab.id },
          files: ["/content-scripts/extract-page.js"],
        });
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
      let lastSequence = -1;
      let expectedAckNext = 0;
      let expectedTransferPhase: "begin" | "chunk" | "events" = "begin";
      let endSent = false;
      setRun({ kind: "running", label: `正在传输正文 0/${transfer.chunks.length}`, output: "" });
      const sendChunk = (index: number) => {
        const data = transfer.chunks[index];
        if (data === undefined) throw new Error("本地正文分块索引越界");
        expectedTransferPhase = "chunk";
        expectedAckNext = index + 1;
        port.postMessage({
          protocol_version: PROTOCOL_VERSION,
          request_id: requestId,
          command: { type: "handoff_chunk", task_id: taskId, index, data },
        });
      };
      port.onMessage.addListener((message: HostResponse | HandoffEvent | HandoffTransferAck) => {
        if (message.request_id !== requestId) return;
        if ("error" in message && message.error) {
          setRun({ kind: "failed", label: message.error.message, output: "" });
          port.disconnect();
          return;
        }
        if ("phase" in message) {
          if (message.task_id !== taskId) {
            setRun({ kind: "failed", label: "正文传输 task_id 不一致", output: "" });
            port.disconnect();
            return;
          }
          const validBegin = expectedTransferPhase === "begin" && message.phase === "begin" && message.next_index === 0;
          const validChunk = expectedTransferPhase === "chunk" && message.phase === "chunk" && message.next_index === expectedAckNext;
          if (!validBegin && !validChunk) {
            setRun({ kind: "failed", label: "正文传输 ACK 顺序无效", output: "" });
            port.disconnect();
            return;
          }
          if (message.next_index < transfer.chunks.length) {
            setRun({ kind: "running", label: `正在传输正文 ${message.next_index}/${transfer.chunks.length}`, output: "" });
            sendChunk(message.next_index);
          } else if (!endSent) {
            endSent = true;
            expectedTransferPhase = "events";
            setRun({ kind: "running", label: "正文完整性校验中…", output: "" });
            port.postMessage({
              protocol_version: PROTOCOL_VERSION,
              request_id: requestId,
              command: { type: "handoff_end", task_id: taskId },
            });
          }
          return;
        }
        if (!("task_id" in message) || message.task_id !== taskId || message.sequence <= lastSequence) return;
        lastSequence = message.sequence;
        const text = message.text ?? "";
        setRun((current) => {
          const output = "output" in current ? current.output : "";
          if (message.event === "completed") return { kind: "done", label: `${agentLabel} 已完成`, output: text || output };
          if (message.event === "failed" || message.event === "cancelled") return { kind: "failed", label: text || `${agentLabel} 任务未完成`, output };
          if (message.event === "output_delta") return { kind: "running", label: `${agentLabel} 正在分析`, output: output + text };
          if (message.event === "tool_started") return { kind: "running", label: `${agentLabel} 正在使用工具${text ? `：${text}` : ""}`, output };
          return { kind: "running", label: message.event === "submitted" ? `已提交，等待 ${agentLabel}` : `${agentLabel} 正在分析`, output };
        });
      });
      port.onDisconnect.addListener(() => {
        if (browser.runtime.lastError) {
          setRun((current) => current.kind === "running" ? { kind: "failed", label: unavailableDetail(browser.runtime.lastError), output: current.output } : current);
        }
      });
      port.postMessage({
        protocol_version: PROTOCOL_VERSION,
        request_id: requestId,
        command: {
          type: "handoff_begin",
          task_id: taskId,
          target_id: selectedTarget,
          prompt,
          source: sourceMetadata,
          total_bytes: transfer.totalBytes,
          total_chunks: transfer.chunks.length,
          sha256: transfer.sha256,
        },
      });
    } catch (error) {
      setRun({ kind: "failed", label: error instanceof Error ? error.message : String(error), output: "" });
    }
  }, [prompt, selectedTarget, selectedTargetStatus]);

  return (
    <main>
      <header><div className="mark" aria-hidden="true">AF</div><div><p className="eyebrow">AGENT FERRY</p><h1>交给你的 Agent</h1></div></header>
      <section className={`status status-${connection.kind}`} aria-live="polite"><span className="status-dot" /><div><p className="status-title">{connection.kind === "checking" ? "正在检查本地连接" : connection.kind === "ready" ? "本地通路已就绪" : "暂时无法连接"}</p><p className="status-detail">{connection.kind === "checking" ? "Chrome → Native Host → agentferryd" : connection.kind === "ready" ? `Agent Ferry ${connection.result.core_version}` : connection.detail}</p></div></section>
      {connection.kind === "ready" && <section className="targets" aria-label="发送到"><div className="section-heading"><p>发送到</p><span>{readyTargets.length} 个可用</span></div>{(connection.result.targets?.length ?? 0) === 0 ? <div className="empty-target"><p>尚未配置可用目标</p><code>请在下方添加本地 Agent 启动目录</code></div> : connection.result.targets?.map((target) => {
        const selected = selectedTarget === target.id;
        const agent = targetAgent(target);
        const realtime = target.capabilities.includes("run.events_sse") || target.capabilities.includes("run.events");
        return <label className={`target target-${agent.local ? "local" : "remote"} ${selected ? "target-selected" : ""} ${target.state !== "ready" ? "target-disabled" : ""}`} key={target.id}>
          <input className="target-radio" type="radio" name="target" value={target.id} checked={selected} disabled={target.state !== "ready"} onChange={() => setSelectedTarget(target.id)} />
          <span className="target-avatar" aria-hidden="true">{agent.avatar}</span>
          <span className="target-copy"><span className="target-name">{target.name}</span><span className="target-meta"><span>{agent.local ? "本机" : "远程"}</span><span className={`target-state target-state-${target.state}`}>{targetStateLabel(target.state)}</span>{target.state === "ready" && realtime && <span>实时输出</span>}</span></span>
          <span className="target-check" aria-hidden="true" />
        </label>;
      })}</section>}
      {connection.kind === "ready" && <details className="workspace-settings">
        <summary><span><strong>本地 Agent 启动目录</strong><small>OpenCode、Claude Code 与 Codex</small></span><span className="summary-action">配置</span></summary>
        <div className="workspace-panel">
          <p className="workspace-help">每个目录会生成对应的本地 Agent 目标。Hermes 不使用这里的配置。</p>
          <div className="workspace-list">
            {(connection.result.workspaces ?? []).map((workspace) => <div className="workspace-item" key={workspace.id}>
              <span className={`workspace-route ${workspace.ready ? "" : "workspace-route-broken"}`} aria-hidden="true">4 AGENTS</span>
              <span className="workspace-copy"><strong>{workspace.name}</strong><code title={workspace.path}>{workspace.path}</code></span>
              <button className="workspace-remove" type="button" disabled={workspaceBusy} aria-label={`删除启动目录 ${workspace.name}`} onClick={() => void updateWorkspaces({ type: "workspace_remove", identifier: workspace.id })}>移除</button>
            </div>)}
          </div>
          <div className="workspace-form">
            <label>名称<input value={workspaceDraft.name} maxLength={128} placeholder="例如：agent-ferry" onChange={(event) => setWorkspaceDraft({ ...workspaceDraft, name: event.target.value })} /></label>
            <label>绝对路径<input value={workspaceDraft.path} placeholder="/Users/name/projects/app" onChange={(event) => setWorkspaceDraft({ ...workspaceDraft, path: event.target.value })} /></label>
            <button className="workspace-add" type="button" disabled={workspaceBusy || !workspaceDraft.name.trim() || !workspaceDraft.path.trim()} onClick={() => void updateWorkspaces({ type: "workspace_add", name: workspaceDraft.name, path: workspaceDraft.path })}>{workspaceBusy ? "正在保存…" : "添加启动目录"}</button>
          </div>
          {workspaceError && <p className="workspace-error" role="alert">{workspaceError}</p>}
        </div>
      </details>}
      {connection.kind === "ready" && <section className="prompt-section">
        <div className="template-row">
          <label>Prompt 模板<select value={templateSettings.selected_template_id ?? ""} onChange={(event) => void selectTemplate(event.target.value)}><option value="">默认 Prompt</option>{templateSettings.templates.map((template) => <option value={template.id} key={template.id}>{template.name}</option>)}</select></label>
          <div className="template-actions">
            <button className="text-button" type="button" onClick={() => { setTemplateDraft({ name: "", content: prompt }); setTemplateError(""); }}>新建</button>
            <button className="text-button" type="button" disabled={!templateSettings.selected_template_id} onClick={() => { const selected = templateSettings.templates.find((template) => template.id === templateSettings.selected_template_id); if (selected) setTemplateDraft({ ...selected }); }}>编辑</button>
            <button className="text-button danger" type="button" disabled={!templateSettings.selected_template_id} onClick={() => void removeSelectedTemplate()}>删除</button>
          </div>
        </div>
        {templateDraft && <div className="template-editor">
          <label>模板名称<input value={templateDraft.name} maxLength={80} onChange={(event) => setTemplateDraft({ ...templateDraft, name: event.target.value })} /></label>
          <label>模板内容<textarea value={templateDraft.content} rows={3} onChange={(event) => setTemplateDraft({ ...templateDraft, content: event.target.value })} /></label>
          <div className="editor-actions"><button className="text-button" type="button" onClick={() => setTemplateDraft(null)}>取消</button><button className="small-primary" type="button" onClick={() => void commitTemplate()}>保存模板</button></div>
        </div>}
        {templateError && <p className="template-error" role="alert">{templateError}</p>}
        <label className="prompt-label">最终 Prompt（将原样发送）<textarea value={prompt} onChange={(event) => setPrompt(event.target.value)} rows={4} /></label>
      </section>}
      {connection.kind === "unavailable" && <button className="secondary" type="button" onClick={() => void checkConnection()}>重新检查</button>}
      <button className="primary" type="button" disabled={connection.kind !== "ready" || readyTargets.length === 0 || !selectedTarget || !prompt.trim() || run.kind === "capturing" || run.kind === "running"} onClick={() => void startHandoff()}>{run.kind === "capturing" ? "正在提取页面…" : run.kind === "running" ? "Agent 正在处理…" : "发送当前页面"}</button>
      {run.kind !== "idle" && <section className={`run run-${run.kind}`} aria-live="polite"><p className="run-label">{run.label}</p>{"output" in run && run.output && <pre>{run.output}</pre>}</section>}
      <p className="footnote">{run.kind === "capturing" ? "提取完成前请保持弹窗打开" : "关闭弹窗不会取消已提交的任务"}</p>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
