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
    return "尚未检测到 Agent Ferry Core。执行下方命令后重新检查。";
  }
  return message || "无法连接 Agent Ferry Core。";
}

function App() {
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });
  const [copied, setCopied] = useState(false);

  const checkConnection = useCallback(async () => {
    setConnection({ kind: "checking" });
    try {
      if (typeof browser === "undefined" || !browser.runtime?.sendNativeMessage) {
        throw new Error("尚未在 Chrome 扩展环境中运行。");
      }
      const response = await browser.runtime.sendNativeMessage(NATIVE_HOST_NAME, {
        protocol_version: PROTOCOL_VERSION,
        request_id: crypto.randomUUID(),
        command: { type: "status" },
      }) as StatusResponse;
      if (response.protocol_version !== PROTOCOL_VERSION) throw new Error("协议版本不兼容，请升级 Agent Ferry Core。");
      if (response.error) throw new Error(response.error.message);
      if (response.result?.daemon !== "ready") throw new Error("Agent Ferry 后台服务尚未就绪。");
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

  return <main className="onboarding-shell">
    <header className="brand">
      <img src="/icons/icon-128.png" alt=""/>
      <div><span>AGENT FERRY</span><h1>把网页交给你的 Agent</h1></div>
    </header>

    <section className={`connection-card ${connection.kind}`} aria-live="polite">
      <span className="status-dot"/>
      <div>
        <strong>{connection.kind === "ready" ? "本地连接已就绪" : connection.kind === "checking" ? "正在检查本地连接" : "还差一步"}</strong>
        <p>{connection.kind === "ready" ? `Agent Ferry Core ${connection.version} 已运行，可以开始发送网页。` : connection.kind === "checking" ? "正在检查 Chrome、Native Host 与 agentferryd。" : connection.detail}</p>
      </div>
    </section>

    <section className="setup-card">
      <div className="step-number">1</div>
      <div className="step-content">
        <h2>安装轻量 Core</h2>
        <p>在已经安装 Homebrew 的 macOS Terminal 中执行。无需 sudo、Rust 或 Node.js，也不会替你安装 Claude Code、Codex 或 OpenCode。</p>
        <div className="command-row">
          <code>{INSTALL_COMMAND}</code>
          <button type="button" onClick={() => void copyCommand()}>{copied ? "已复制" : "复制"}</button>
        </div>
      </div>
    </section>

    <section className="setup-card">
      <div className="step-number">2</div>
      <div className="step-content">
        <h2>确认连接</h2>
        <p>安装并执行激活命令后，会注册 Native Host 并启动 agentferryd。完成后回到这里重新检查。</p>
        <button className="check-button" type="button" disabled={connection.kind === "checking"} onClick={() => void checkConnection()}>{connection.kind === "checking" ? "正在检查…" : "重新检查"}</button>
      </div>
    </section>

    <section className="setup-card">
      <div className="step-number">3</div>
      <div className="step-content">
        <h2>选择页面、Agent 与运行位置</h2>
        <p>打开任意 http(s) 页面，再点击工具栏里的 Agent Ferry。正文只会在你确认可见 Prompt 和目的地并点击开始后提取。</p>
      </div>
    </section>

    <footer>
      <a href={PRIVACY_URL} target="_blank" rel="noreferrer">隐私政策</a>
      <span>·</span>
      <a href={SUPPORT_URL} target="_blank" rel="noreferrer">帮助与反馈</a>
    </footer>
  </main>;
}

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode><App/></React.StrictMode>);
