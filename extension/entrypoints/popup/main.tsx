import React, { useCallback, useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import "./style.css";

const NATIVE_HOST_NAME = "com.agentferry.host";
const PROTOCOL_VERSION = 1;

type StatusResult = {
  core_version: string;
  daemon: "ready" | "not_detected";
  native_host: "ready" | "not_detected";
  chrome_extension: "ready" | "not_detected";
  capabilities: string[];
};

type HostResponse = {
  protocol_version: number;
  request_id: string;
  result?: StatusResult;
  error?: {
    code: string;
    message: string;
    recoverable: boolean;
  };
};

type ConnectionState =
  | { kind: "checking" }
  | { kind: "ready"; result: StatusResult }
  | { kind: "unavailable"; detail: string };

function unavailableDetail(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes("host not found") || message.includes("Specified native messaging host")) {
    return "未找到 Chrome Native Host。先运行 aferry setup，按提示完成注册。";
  }
  if (message.includes("Access to the specified native messaging host is forbidden")) {
    return "当前扩展不在 Native Host allowlist 中。请用本扩展 ID 重新执行注册命令。";
  }
  return `本地连接不可用：${message}`;
}

function App() {
  const [connection, setConnection] = useState<ConnectionState>({ kind: "checking" });

  const checkConnection = useCallback(async () => {
    setConnection({ kind: "checking" });
    try {
      const response = (await browser.runtime.sendNativeMessage(NATIVE_HOST_NAME, {
        protocol_version: PROTOCOL_VERSION,
        request_id: crypto.randomUUID(),
        command: { type: "status" },
      })) as HostResponse;

      if (response.protocol_version !== PROTOCOL_VERSION) {
        setConnection({
          kind: "unavailable",
          detail: `协议版本不兼容：扩展为 ${PROTOCOL_VERSION}，本地服务为 ${response.protocol_version}。请升级 Agent Ferry。`,
        });
      } else if (response.error) {
        setConnection({ kind: "unavailable", detail: response.error.message });
      } else if (response.result?.daemon === "ready") {
        setConnection({ kind: "ready", result: response.result });
      } else {
        setConnection({ kind: "unavailable", detail: "daemon 尚未就绪，请启动 agentferryd。" });
      }
    } catch (error) {
      setConnection({ kind: "unavailable", detail: unavailableDetail(error) });
    }
  }, []);

  useEffect(() => {
    void checkConnection();
  }, [checkConnection]);

  return (
    <main>
      <header>
        <div className="mark" aria-hidden="true">AF</div>
        <div>
          <p className="eyebrow">AGENT FERRY</p>
          <h1>交给你的 Agent</h1>
        </div>
      </header>

      <section className={`status status-${connection.kind}`} aria-live="polite">
        <span className="status-dot" />
        <div>
          <p className="status-title">
            {connection.kind === "checking" && "正在检查本地连接"}
            {connection.kind === "ready" && "本地通路已就绪"}
            {connection.kind === "unavailable" && "暂时无法连接"}
          </p>
          <p className="status-detail">
            {connection.kind === "checking" && "Chrome → Native Host → agentferryd"}
            {connection.kind === "ready" && `Agent Ferry ${connection.result.core_version}`}
            {connection.kind === "unavailable" && connection.detail}
          </p>
        </div>
      </section>

      {connection.kind === "unavailable" && (
        <button className="secondary" type="button" onClick={() => void checkConnection()}>
          重新检查
        </button>
      )}

      <button className="primary" type="button" disabled={connection.kind !== "ready"}>
        发送当前页面
      </button>
      <p className="footnote">页面提取将在下一阶段开放</p>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
