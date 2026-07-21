import React, { useCallback, useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import "./style.css";

const NATIVE_HOST_NAME = "com.agentferry.host";
const PROTOCOL_VERSION = 1;
const INSTALL_COMMAND = "brew install Wan-Kai/tap/agent-ferry\naferry activate";
const PRIVACY_URL = "https://github.com/Wan-Kai/agent-ferry/blob/main/PRIVACY.md";
const SUPPORT_URL = "https://github.com/Wan-Kai/agent-ferry/issues";

type ConnectionState =
  | { kind: "checking" }
  | { kind: "ready"; version: string }
  | { kind: "unavailable"; detail: string };

type StatusResponse = {
  protocol_version: number;
  result?: { core_version: string; daemon: "ready" | "not_detected" };
  error?: { message: string };
};

function connectionError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (/native messaging host|specified native messaging host|not found/i.test(message)) {
    return "尚未检测到 Core。复制安装命令，在终端运行后重新检查。";
  }
  return message || "本地通道没有响应，请确认 Core 已完成激活。";
}

function CopyIcon() {
  return <svg viewBox="0 0 20 20" aria-hidden="true">
    <rect x="6.5" y="6.5" width="9" height="9" rx="2" />
    <path d="M13.5 6.5V5A2.5 2.5 0 0 0 11 2.5H5A2.5 2.5 0 0 0 2.5 5v6A2.5 2.5 0 0 0 5 13.5h1.5" />
  </svg>;
}

function RefreshIcon() {
  return <svg viewBox="0 0 20 20" aria-hidden="true">
    <path d="M16 6.5V3m0 3.5h-3.5M15.3 6A6.5 6.5 0 1 0 16 13" />
  </svg>;
}

function CheckIcon() {
  return <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m4 10.2 3.7 3.7L16 5.7" /></svg>;
}

function RouteMap({ connection }: { connection: ConnectionState }) {
  return <div className={`route-map route-${connection.kind}`} aria-hidden="true">
    <div className="route-source">
      <div className="browser-bar"><i /><i /><i /></div>
      <div className="page-lines"><b /><span /><span /><span /></div>
      <small>当前网页</small>
    </div>

    <div className="route-track route-track-in">
      <span className="route-packet"><i /></span>
    </div>

    <div className="route-core">
      <img src="/icons/icon-128.png" alt="" />
      <span>Ferry Core</span>
      <small>{connection.kind === "ready" ? "在线" : connection.kind === "checking" ? "连接中" : "待连接"}</small>
    </div>

    <div className="route-track route-track-out" />

    <div className="route-agents">
      <img src="/icons/agents/claude.svg" alt="" />
      <img src="/icons/agents/codex.svg" alt="" />
      <img src="/icons/agents/opencode.svg" alt="" />
      <img src="/icons/agents/hermes.svg" alt="" />
      <small>你的 Agents</small>
    </div>
  </div>;
}

function App() {
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });
  const [copied, setCopied] = useState(false);

  const checkConnection = useCallback(async () => {
    setConnection({ kind: "checking" });
    try {
      if (typeof browser === "undefined" || !browser.runtime?.sendNativeMessage) {
        throw new Error("当前页面不在 Chrome 扩展环境中运行。");
      }
      const response = await browser.runtime.sendNativeMessage(NATIVE_HOST_NAME, {
        protocol_version: PROTOCOL_VERSION,
        request_id: crypto.randomUUID(),
        command: { type: "status" },
      }) as StatusResponse;
      if (response.protocol_version !== PROTOCOL_VERSION) throw new Error("Core 协议版本不兼容，请先升级 Agent Ferry。");
      if (response.error) throw new Error(response.error.message);
      if (response.result?.daemon !== "ready") throw new Error("Core 已安装，但后台服务尚未启动。请重新运行 aferry activate。");
      setConnection({ kind: "ready", version: response.result.core_version });
    } catch (error) {
      setConnection({ kind: "unavailable", detail: connectionError(error) });
    }
  }, []);

  useEffect(() => { void checkConnection(); }, [checkConnection]);

  const copyCommand = async () => {
    await navigator.clipboard.writeText(INSTALL_COMMAND);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  };

  const statusTitle = connection.kind === "ready"
    ? "航线已接通"
    : connection.kind === "checking"
      ? "正在确认本地通道"
      : "还差一次本地连接";
  const statusDetail = connection.kind === "ready"
    ? `Core ${connection.version} 正在运行，网页现在可以交给你的 Agent。`
    : connection.kind === "checking"
      ? "正在检查 Chrome、Native Host 与后台服务。"
      : connection.detail;

  return <div className="onboarding-page">
    <header className="topbar">
      <a className="wordmark" href="#top" aria-label="Agent Ferry 首页">
        <img src="/icons/icon-48.png" alt="" />
        <span>Agent Ferry</span>
      </a>
      <div className="topbar-route"><span>Browser</span><i /><span>Agent</span></div>
    </header>

    <main id="top" className="onboarding-shell">
      <section className="intro-panel">
        <div className="intro-copy">
          <p className="eyebrow">YOUR PAGE, IN THEIR CONTEXT</p>
          <h1>从这一页，<br />驶向你的 Agent。</h1>
          <p className="lead">把正在阅读的论文、帖子或文档，连同你的任务指令，直接交给本地 Agent 或远端 Hermes。</p>
        </div>

        <RouteMap connection={connection} />

        <div className="privacy-note">
          <span className="privacy-mark"><i /></span>
          <p><strong>点击发送之前，我们不会读取网页正文。</strong><br />内容只前往你选择的 Agent，不经过 Agent Ferry 云端。</p>
        </div>
      </section>

      <section className="setup-panel" aria-label="连接 Agent Ferry">
        <div className={`connection-status ${connection.kind}`} aria-live="polite">
          <span className="status-beacon"><i /></span>
          <div>
            <p className="status-label">LOCAL CHANNEL</p>
            <h2>{statusTitle}</h2>
            <p>{statusDetail}</p>
          </div>
        </div>

        <ol className="setup-list">
          <li className={connection.kind === "ready" ? "step-complete" : connection.kind === "unavailable" ? "step-current" : "step-upcoming"}>
            <div className="step-marker">{connection.kind === "ready" ? <CheckIcon /> : "1"}</div>
            <div className="step-content">
              <div className="step-heading">
                <h3>安装轻量 Core</h3>
                <span>macOS · Homebrew</span>
              </div>
              <p>在终端执行两行命令。无需 sudo、Rust 或 Node.js，也不会安装任何 Agent。</p>
              <div className="terminal">
                <div className="terminal-chrome"><span>Terminal</span><i /><i /><i /></div>
                <pre><code><span>brew install</span> Wan-Kai/tap/agent-ferry{"\n"}<span>aferry activate</span></code></pre>
                <button className={copied ? "copied" : ""} type="button" onClick={() => void copyCommand()}>
                  {copied ? <CheckIcon /> : <CopyIcon />}
                  {copied ? "已复制" : "复制命令"}
                </button>
              </div>
            </div>
          </li>

          <li className={connection.kind === "ready" ? "step-complete" : connection.kind === "checking" ? "step-current" : "step-upcoming"}>
            <div className="step-marker">{connection.kind === "ready" ? <CheckIcon /> : "2"}</div>
            <div className="step-content">
              <div className="step-heading"><h3>确认通道</h3></div>
              <p>{connection.kind === "ready" ? "Chrome 已经可以安全地连接本地 Core。" : "命令运行完成后，回到这里确认连接。"}</p>
              <button className="check-button" type="button" disabled={connection.kind === "checking"} onClick={() => void checkConnection()}>
                <RefreshIcon />
                {connection.kind === "checking" ? "正在检查…" : connection.kind === "ready" ? "再次检查" : "检查连接"}
              </button>
            </div>
          </li>

          <li className={connection.kind === "ready" ? "step-current" : "step-upcoming"}>
            <div className="step-marker">3</div>
            <div className="step-content">
              <div className="step-heading"><h3>打开网页，选择目的地</h3></div>
              <p>固定工具栏图标，在任意网页中打开 Agent Ferry，确认 Agent、运行位置和最终任务指令。</p>
              {connection.kind === "ready" && <div className="ready-callout"><i /><span>一切就绪，可以关闭本页开始使用</span></div>}
            </div>
          </li>
        </ol>
      </section>
    </main>

    <footer>
      <span>Agent Ferry 不托管你的网页或 Agent 凭据</span>
      <nav aria-label="帮助链接">
        <a href={PRIVACY_URL} target="_blank" rel="noreferrer">隐私政策</a>
        <a href={SUPPORT_URL} target="_blank" rel="noreferrer">帮助与反馈</a>
      </nav>
    </footer>
  </div>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
