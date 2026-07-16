import React, { useCallback, useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";
import type { CapturedPage } from "../extract-page.content";
import { prepareHandoffTransfer } from "../../lib/handoff-transfer";
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
  kind: "remote_hermes";
  state: "ready" | "credential_missing" | "authentication_failed" | "connection_failed" | "incompatible";
  capabilities: string[];
};
type StatusResult = {
  core_version: string;
  daemon: "ready" | "not_detected";
  targets?: HandoffTargetStatus[];
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
type RunState = { kind: "idle" } | { kind: "capturing" } | { kind: "running" | "done" | "failed"; label: string; output: string };
type TemplateDraft = { id?: string; name: string; content: string };
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

function App() {
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });
  const [selectedTarget, setSelectedTarget] = useState("");
  const [prompt, setPrompt] = useState(DEFAULT_PROMPT);
  const [run, setRun] = useState<RunState>({ kind: "idle" });
  const [templateSettings, setTemplateSettings] = useState<PromptTemplateSettings>(EMPTY_PROMPT_TEMPLATE_SETTINGS);
  const [templateDraft, setTemplateDraft] = useState<TemplateDraft | null>(null);
  const [templateError, setTemplateError] = useState("");

  const readyTargets = useMemo(
    () => connection.kind === "ready" ? (connection.result.targets ?? []).filter((target) => target.state === "ready") : [],
    [connection],
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
        setSelectedTarget((current) => current || firstReady?.id || "");
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

  const startHandoff = useCallback(async () => {
    if (!selectedTarget || !prompt.trim()) return;
    setRun({ kind: "capturing" });
    try {
      const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
      if (!tab.id || !tab.url?.match(/^https?:\/\//)) throw new Error("当前页面不是可提取的 http(s) 页面");
      // 当前首发平台明确是 Chrome MV3；原生命名空间比 polyfill 对 runtime content script 的支持更稳定。
      const chromeApi = (globalThis as typeof globalThis & { chrome: ChromeScriptingApi }).chrome;
      const results = await chromeApi.scripting.executeScript({
        target: { tabId: tab.id },
        files: ["/content-scripts/extract-page.js"],
      });
      const injection = results[0];
      const source = injection?.result as CapturedPage | undefined;
      if (!source) throw new Error(injection?.error || "页面提取没有返回正文");
      const transfer = await prepareHandoffTransfer(source.markdown);
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
          if (message.event === "completed") return { kind: "done", label: "Hermes 已完成", output: text || output };
          if (message.event === "failed" || message.event === "cancelled") return { kind: "failed", label: text || "Hermes 任务未完成", output };
          if (message.event === "output_delta") return { kind: "running", label: "Hermes 正在分析", output: output + text };
          if (message.event === "tool_started") return { kind: "running", label: `Hermes 正在使用工具${text ? `：${text}` : ""}`, output };
          return { kind: "running", label: message.event === "submitted" ? "已提交，等待 Hermes" : "Hermes 正在分析", output };
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
  }, [prompt, selectedTarget]);

  return (
    <main>
      <header><div className="mark" aria-hidden="true">AF</div><div><p className="eyebrow">AGENT FERRY</p><h1>交给你的 Agent</h1></div></header>
      <section className={`status status-${connection.kind}`} aria-live="polite"><span className="status-dot" /><div><p className="status-title">{connection.kind === "checking" ? "正在检查本地连接" : connection.kind === "ready" ? "本地通路已就绪" : "暂时无法连接"}</p><p className="status-detail">{connection.kind === "checking" ? "Chrome → Native Host → agentferryd" : connection.kind === "ready" ? `Agent Ferry ${connection.result.core_version}` : connection.detail}</p></div></section>
      {connection.kind === "ready" && <section className="targets" aria-label="可用目标"><div className="section-heading"><p>REMOTE HERMES</p><span>{connection.result.targets?.length ?? 0}</span></div>{(connection.result.targets?.length ?? 0) === 0 ? <div className="empty-target"><p>尚未配置远程目标</p><code>aferry connection add hermes</code></div> : connection.result.targets?.map((target) => <label className={`target ${target.state !== "ready" ? "target-disabled" : ""}`} key={target.id}><input type="radio" name="target" value={target.id} checked={selectedTarget === target.id} disabled={target.state !== "ready"} onChange={() => setSelectedTarget(target.id)} /><span className={`target-dot target-${target.state}`} /><div><p className="target-name">{target.name}</p><p className="target-detail">{targetStateLabel(target.state)}{target.state === "ready" && target.capabilities.includes("run.events_sse") ? " · 实时输出" : ""}</p></div></label>)}</section>}
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
      <button className="primary" type="button" disabled={connection.kind !== "ready" || readyTargets.length === 0 || !selectedTarget || !prompt.trim() || run.kind === "capturing" || run.kind === "running"} onClick={() => void startHandoff()}>{run.kind === "capturing" ? "正在提取页面…" : run.kind === "running" ? "Hermes 正在处理…" : "发送当前页面"}</button>
      {run.kind !== "idle" && run.kind !== "capturing" && <section className={`run run-${run.kind}`} aria-live="polite"><p className="run-label">{run.label}</p>{run.output && <pre>{run.output}</pre>}</section>}
      <p className="footnote">关闭弹窗不会取消已提交的远端任务</p>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
