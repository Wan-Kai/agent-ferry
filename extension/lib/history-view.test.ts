import { describe, expect, it } from "vitest";
import { filterArchiveTasks, selectRecentCompleted, type HistoryViewTask } from "./history-view";

function task(id: string, state: HistoryViewTask["state"], updated: number, title = id, target = "Hermes"): HistoryViewTask {
  return { task_id: id, title, url: `https://example.com/${id}`, site: "example.com", target_name: target, state, updated_at_ms: updated };
}

describe("selectRecentCompleted", () => {
  it("只截取普通的近期已完成任务", () => {
    const tasks = [task("new", "completed", 3), task("middle", "completed", 2), task("old", "completed", 1), task("running", "running", 4)];
    expect(selectRecentCompleted(tasks, new Set(), 2).map((item) => item.task_id)).toEqual(["new", "middle"]);
  });

  it("已关注任务不占用近期任务名额", () => {
    const tasks = [task("new", "completed", 3), task("middle", "completed", 2), task("old", "completed", 1)];
    expect(selectRecentCompleted(tasks, new Set(["old"]), 2).map((item) => item.task_id)).toEqual(["new", "middle", "old"]);
  });
});

describe("filterArchiveTasks", () => {
  const tasks = [
    task("paper", "completed", 3, "Attention Is All You Need", "Hermes"),
    task("clip", "running", 2, "ClipSeek", "OpenCode"),
    task("failed", "interrupted", 1, "旧任务", "Hermes"),
  ];

  it("可以组合搜索词、状态与 Agent", () => {
    expect(filterArchiveTasks(tasks, "attention", "completed", "Hermes").map((item) => item.task_id)).toEqual(["paper"]);
  });

  it("将中断和取消任务归入失败筛选", () => {
    expect(filterArchiveTasks(tasks, "", "failed", "").map((item) => item.task_id)).toEqual(["failed"]);
  });
});
