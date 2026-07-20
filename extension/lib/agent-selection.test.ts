import { describe, expect, it } from "vitest";
import {
  buildAgentProducts,
  buildRunLocations,
  migrateLocationSelections,
  migrateSelectedProduct,
  type AgentTarget,
  type AgentWorkspace,
} from "./agent-selection";

const targets: AgentTarget[] = [
  { id: "remote-hermes", name: "ktoon-hermes", kind: "remote_hermes", state: "ready", capabilities: [] },
  { id: "claude-workspace-a", name: "Claude Code · agent-ferry", kind: "local_claude_code", state: "ready", capabilities: [] },
  { id: "opencode-workspace-a", name: "OpenCode · agent-ferry", kind: "local_open_code", state: "incompatible", capabilities: [] },
];

const workspaces: AgentWorkspace[] = [
  { id: "workspace-a", name: "agent-ferry", path: "/Users/name/agent-ferry", ready: true },
];

describe("Agent 产品与运行位置", () => {
  it("产品选择不混入本地或远端信息", () => {
    expect(buildAgentProducts(targets).map(({ id, title, ready }) => ({ id, title, ready }))).toEqual([
      { id: "hermes", title: "Hermes", ready: true },
      { id: "claude_code", title: "Claude Code", ready: true },
      { id: "open_code", title: "OpenCode", ready: false },
      { id: "codex_cli", title: "Codex CLI", ready: false },
      { id: "codex_app", title: "Codex App", ready: false },
    ]);
  });

  it("Hermes 同时展示远端实例和未检测到的本机实例", () => {
    expect(buildRunLocations("hermes", targets, workspaces)).toEqual([
      {
        id: "remote-hermes",
        title: "ktoon-hermes",
        meta: "远端 · 实时输出",
        locality: "remote",
        state: "ready",
        disabled: false,
        targetId: "remote-hermes",
      },
      {
        id: "local-hermes-unavailable",
        title: "Local Hermes",
        meta: "本机 · 未检测到",
        locality: "local",
        state: "not_detected",
        disabled: true,
        targetId: null,
      },
    ]);
  });

  it("Claude Code 的运行位置使用工作区名称和完整路径", () => {
    expect(buildRunLocations("claude_code", targets, workspaces)).toEqual([
      {
        id: "claude-workspace-a",
        title: "agent-ferry",
        meta: "/Users/name/agent-ferry",
        locality: "local",
        state: "ready",
        disabled: false,
        targetId: "claude-workspace-a",
      },
    ]);
  });

  it("可以从旧版选择值迁移产品", () => {
    expect(migrateSelectedProduct("remote-hermes", targets)).toBe("hermes");
    expect(migrateSelectedProduct("local_claude_code", targets)).toBe("claude_code");
  });

  it("升级后保留 Hermes 连接和每个本地 Agent 的启动目录", () => {
    expect(migrateLocationSelections("remote-hermes", { local_claude_code: "workspace-a" }, targets)).toEqual({
      hermes: "remote-hermes",
      claude_code: "claude-workspace-a",
    });
  });
});
