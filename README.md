# maa-mcp-adapter

Unofficial MCP adapter for MAA.

一个面向 Windows 的实验性 STDIO MCP Server，让 Claude Code、Codex 等 Agent 在明确的安全边界内规划、确认、启动和管理 MAA 日常任务。

> [!IMPORTANT]
> 本项目是独立的非官方适配器，与 MAA Team 无关联，未获 MAA Team 认可或背书。本项目不包含、分发或修改 MAA、MaaCore、`maa-cli` 及其资源。

## 当前状态

项目处于 `0.1.0` MVP 发布准备阶段。fake CLI 自动化测试已覆盖完整协议和失败路径；真实 `maa-cli v0.7.5` / MaaCore `v6.14.2` 上已通过 capability probe、不启动游戏的 dry-run，以及一次经 Claude Code CLI form elicitation 确认、在真机模拟器上完整成功的真实安全日常。发布前仍需：

- 使用支持 MCP form elicitation 的 Codex 客户端完成最终确认与 fake MAA 执行验收；
- 分别验证 Claude Code 与 Codex 关闭、重启后的运行恢复。

在完成这些人工验收前，请勿把本项目用于无人值守的真实账号。

## 安全工作流

Agent 不能从一句模糊请求直接启动 MAA。标准流程是：

```text
用户意图
→ Agent 逐项询问缺失信息
→ 创建/更新 Draft
→ 生成完整中文摘要和计划哈希
→ Agent 原样携带摘要与哈希调用执行（服务端逐字核对）
→ MCP 客户端显示二次确认表单
→ 用户接受准确摘要
→ Server 再次核对 revision、策略、摘要和哈希
→ dry-run
→ 启动唯一后台任务
```

单人可信环境可通过 `confirmation = "none"` 显式跳过表单一步（其余校验不变，
摘要仍随调用参数展示在客户端授权界面），详见[适配器配置](docs/configuration.md)。

固定限制包括：

- 只允许 `StartUp`、`Fight`、`Recruit`、`Infrast`、`Mall`、`Award`、`CloseDown`。
- 碎石恒为 `0`，禁止账号切换和付费购买。
- 理智药、刷图次数和公招消耗必须是有限显式值。
- 不接受任意 `maa-cli` 参数、Shell 命令或自定义任务映射。
- 同一时间最多一个活动运行。
- 停止前必须同时匹配 PID 和 Windows 进程创建时间。
- 日志每次最多返回 200 行、64 KiB，并应用配置的脱敏模式。

完整威胁模型见 [安全模型](docs/security-model.md)。

## 十个 MCP Tools

| Tool | 用途 |
|---|---|
| `check_environment` | 检查 `maa-cli`、能力和配置档 |
| `get_daily_options` | 返回安全任务类型、字段和固定限制 |
| `create_daily_plan` | 从意图或结构化任务创建 Draft |
| `update_daily_plan` | 按 revision 修改仍可编辑的计划 |
| `preview_daily_plan` | 验证并生成摘要与计划哈希 |
| `cancel_daily_plan` | 取消尚未运行的计划 |
| `execute_daily_plan` | 通过 MCP elicitation 确认后启动 |
| `get_run_status` | 查询并核对运行状态 |
| `get_run_log` | 读取受限、脱敏的日志尾部 |
| `stop_run` | 精确停止记录中的 Windows Job |

## 构建

要求：

- Windows 10/11；
- Rust stable 1.97 或更高；
- 用户已自行安装兼容的 MAA CLI。

```powershell
git clone https://github.com/sac432/maa-mcp-adapter.git
Set-Location maa-mcp-adapter
cargo build --release --locked
```

二进制位于 `target\release\maa-mcp-adapter.exe`。适配器通过 `config.toml` 找到用户已有的 `maa.exe` 或 `maa-cli.exe`，不会复制 MAA 配置档。

## 配置与客户端

- [适配器配置](docs/configuration.md)
- [Claude Code 与 Codex 配置](docs/client-setup.md)
- [安全模型](docs/security-model.md)
- [0.1.0 发布验收清单](docs/release-checklist.md)

连接后先让 Agent 调用 `check_environment`。只有 `can_execute = true` 时才应继续创建真实执行计划。

## 开发验证

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

自动测试只使用仓库内构建的 fake CLI，不操作真实游戏、模拟器或账号。

## 许可证

本项目以 [Apache License 2.0](LICENSE) 发布。MAA 及其相关组件受各自许可证约束；本项目的许可证不授予分发 MAA 的权利。
