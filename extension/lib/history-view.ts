export type HistoryViewState = "running" | "completed" | "failed" | "cancelled" | "interrupted";

export type HistoryViewTask = {
  task_id: string;
  title: string;
  url: string;
  site: string | null;
  target_name: string;
  state: HistoryViewState;
  updated_at_ms: number;
};

export type ArchiveStateFilter = "all" | "running" | "completed" | "failed";

export function taskStateGroup(state: HistoryViewState): Exclude<ArchiveStateFilter, "all"> {
  if (state === "running") return "running";
  if (state === "completed") return "completed";
  return "failed";
}

/**
 * 已关注任务不应因为出现了更新的记录而离开近期列表，因此它们不占用普通近期任务的名额。
 * 返回结果仍按更新时间排序，避免“关注”动作改变用户刚刚浏览过的时间顺序。
 */
export function selectRecentCompleted<T extends HistoryViewTask>(tasks: T[], pinnedTaskIds: Set<string>, limit: number): T[] {
  const completed = tasks.filter((task) => task.state === "completed").sort((left, right) => right.updated_at_ms - left.updated_at_ms);
  const pinned = completed.filter((task) => pinnedTaskIds.has(task.task_id));
  const recent = completed.filter((task) => !pinnedTaskIds.has(task.task_id)).slice(0, limit);
  return [...pinned, ...recent].sort((left, right) => right.updated_at_ms - left.updated_at_ms);
}

export function filterArchiveTasks<T extends HistoryViewTask>(
  tasks: T[],
  query: string,
  state: ArchiveStateFilter,
  targetName: string,
): T[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  return tasks.filter((task) => {
    if (state !== "all" && taskStateGroup(task.state) !== state) return false;
    if (targetName && task.target_name !== targetName) return false;
    if (!normalizedQuery) return true;
    return [task.title, task.url, task.site ?? "", task.target_name]
      .some((value) => value.toLocaleLowerCase().includes(normalizedQuery));
  });
}
