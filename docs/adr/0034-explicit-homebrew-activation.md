# ADR 0034：Homebrew 安装与用户级激活分离

## 状态

Accepted，2026-07-21。取代 [ADR 0033](./0033-native-homebrew-bottles.md)；保留其原生 Bottle、
固定 SHA-256、双架构发行与无 Developer ID 边界，只改变用户级运行资源的激活方式。

## 背景

Homebrew Formula 的 `post_install` 在受限沙箱和临时 `HOME` 中运行。Agent Ferry 的 LaunchAgent 与
Chrome Native Host manifest 必须写入真实登录用户的 `~/Library`，并通过该用户的 `launchctl`
会话加载。即使 Formula 恢复真实 HOME，Homebrew 沙箱仍会拒绝这些写入；强行绕过会让安装看似成功、
实际没有可用后台服务，并扩大包管理器脚本的权限边界。

## 决策

1. `brew install` 和 `brew upgrade` 只管理 Bottle 中的三个程序以及 Homebrew keg，不执行用户级
   `post_install`；
2. CLI 提供 `aferry activate`，由用户在正常终端环境中明确执行；
3. `activate` 只使用同一安装目录内的 `agentferryd` 与 `agentferry-host`，写入固定正式 Chrome
   Item ID，安装并重载当前用户 LaunchAgent；不下载代码、不使用 `sudo`、不读取 Agent 凭据；
4. 激活步骤必须幂等。任一步失败后再次执行同一命令即可修复，不要求用户理解底层两个管理命令；
5. 安装和每次升级后都提示执行 `aferry activate`。升级激活负责把运行资源切换到新 keg；
6. RC 必须先证明日志出现 `Pouring ...bottle.tar.gz`，再在隔离 HOME 中显式激活，并验证服务、
   Native Host、日志和双阶段卸载。

## 后果

- 普通安装从一条命令变为安装与激活两条清晰命令，但两者都无需管理员权限；
- Formula 不再依赖 Homebrew 沙箱之外的用户会话行为，Bottle 安装失败与 Ferry 激活失败可以分别
  诊断；
- Chrome 扩展不会在仅安装二进制后立即连通，必须完成一次 `aferry activate`；
- 升级后应重新激活，避免 LaunchAgent 或 Native Host 继续引用即将被 Homebrew 清理的旧 keg；
- 未来若 Homebrew 提供可靠、明确授权的用户服务机制，可以新增 ADR 评估恢复自动激活，但不得以
  绕过沙箱为实现手段。
