# 真实环境验收 Case 模板

> 状态：Current
> 事实来源：`docs/runbooks/real-environment-acceptance.md`
> 范围：单个真实环境行为的人工或半自动验收记录

```markdown
# CASE-<编号>：<行为>

- 状态：PENDING_ENV | BLOCKED_AUTHORIZATION | PASSED | FAILED
- RC Commit：<完整 SHA>
- 产物：<文件名与 SHA256；不适用时写 N/A>
- 执行时间：<ISO 8601；未执行时写 N/A>
- 环境：<macOS/Chrome/Agent/Hermes 的非敏感版本摘要>

## 前置条件

- ...

## 步骤

1. ...

## 预期

- ...

## 允许的副作用

- 只读 | 创建临时任务 | 写入测试数据 | 取消测试任务

## 结果与证据

- 未执行原因或实际结果：...
- 非敏感证据引用：...
```

禁止记录 Token、Cookie、Keychain Secret、完整用户正文和生产原始错误栈。
