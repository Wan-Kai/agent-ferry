import React, { useCallback, useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";
import type { CapturedPage } from "../extract-page.content";
import "./style.css";

const NATIVE_HOST_NAME = "com.agentferry.host";
const PROTOCOL_VERSION = 1;
const DEFAULT_PROMPT = "请分析这篇内容，提炼核心观点、关键证据和可执行的启发，并将值得长期保留的信息自行沉淀到你的文档或记忆中。";

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
type ConnectionState =
  | { kind: "checking" }
  | { kind: "ready"; result: StatusResult }
  | { kind: "unavailable"; detail: string };
type RunState = { kind: "idle" } | { kind: "capturing" } | { kind: "running" | "done" | "failed"; label: string; output: string };
type ChromeScriptingApi = {
  scripting: {
    executeScript(options: { target: { tabId: number }; files: string[] }): Promise<Array<{ result?: unknown }>>;
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
      const source = results[0]?.result as CapturedPage | undefined;
      if (!source) throw new Error("页面提取没有返回正文");

      const requestId = crypto.randomUUID();
      const taskId = crypto.randomUUID();
      const port = browser.runtime.connectNative(NATIVE_HOST_NAME);
      let lastSequence = -1;
      setRun({ kind: "running", label: "正在提交给 Hermes…", output: "" });
      port.onMessage.addListener((message: HostResponse | HandoffEvent) => {
        if (message.request_id !== requestId) return;
        if ("error" in message && message.error) {
          setRun({ kind: "failed", label: message.error.message, output: "" });
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
        command: { type: "handoff", task_id: taskId, target_id: selectedTarget, prompt: prompt.trim(), source },
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
      {connection.kind === "ready" && <label className="prompt-label">交接指令<textarea value={prompt} onChange={(event) => setPrompt(event.target.value)} rows={4} /></label>}
      {connection.kind === "unavailable" && <button className="secondary" type="button" onClick={() => void checkConnection()}>重新检查</button>}
      <button className="primary" type="button" disabled={connection.kind !== "ready" || readyTargets.length === 0 || !selectedTarget || !prompt.trim() || run.kind === "capturing" || run.kind === "running"} onClick={() => void startHandoff()}>{run.kind === "capturing" ? "正在提取页面…" : run.kind === "running" ? "Hermes 正在处理…" : "发送当前页面"}</button>
      {run.kind !== "idle" && run.kind !== "capturing" && <section className={`run run-${run.kind}`} aria-live="polite"><p className="run-label">{run.label}</p>{run.output && <pre>{run.output}</pre>}</section>}
      <p className="footnote">关闭弹窗不会取消已提交的远端任务</p>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
