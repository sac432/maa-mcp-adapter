# maa-mcp-adapter

Unofficial MCP adapter for MAA.

让 Claude Code、Codex、Cursor 等任何支持 MCP 的 AI Agent，在明确的安全边界内
一句话完成 [MAA](https://maa.plus)（明日方舟小助手）日常：自动开模拟器、
按你保存的 MAA 设置执行日常、跑完关闭——每一步真正动手前都有可核对的确认。
目前只是初版功能，最终目的是一句话让agent通过maa完成所有maa能完成的事以及maa做不到的事（例如maa 搜索攻略+ maa自动挑战指定关卡）

> [!IMPORTANT]
> 本项目是独立的非官方适配器，与 MAA Team 无关联，未获 MAA Team 认可或背书。
> 本项目不包含、分发或修改 MAA、MaaCore、`maa-cli` 及其资源。
> 使用本项目操作游戏账号的风险由使用者自行承担。

## ✨ 特性

- **一句话日常**：`帮我跑日常` → 自动启动模拟器 → 执行 → 关闭游戏与模拟器；
- **默认计划来自你，不来自 LLM**：不指定任务时，计划从你 MAA 图形版保存的
  任务队列确定性派生——Agent 无法擅自加刷图、开加急；
- **确认即所得**：执行前展示完整中文摘要（首行即消耗品上限）与计划哈希，
  服务端逐字核对后才启动；任何改动都会使确认失效；
- **一键初始化**：`setup_environment` 从 MAA 配置自动发现 adb 地址与模拟器，
  经确认后生成全部所需配置，写入即时生效（热加载，无需重启）；
- **可审计、可反馈**：受限脱敏的运行日志；`collect_diagnostics` 一键生成
  可直接粘贴到 Issue 的隐私安全诊断报告。

## 🔒 安全工作流

Agent 不能从一句模糊请求直接启动 MAA。标准流程：

```text
用户意图
→ Agent 逐项询问缺失信息（或从 MAA 保存设置派生默认计划）
→ 创建/更新 Draft → 生成完整中文摘要和计划哈希
→ Agent 原样携带摘要与哈希调用执行（服务端逐字核对，无法伪造）
→ MCP 客户端显示二次确认表单 → 用户接受准确摘要
→ Server 再次核对 revision、策略、摘要和哈希
→ dry-run → 启动唯一后台任务
```

固定限制（服务端强制，任何 Agent 无法覆盖）：

- 只允许 `StartUp`、`Fight`、`Recruit`、`Infrast`、`Mall`、`Award`、`CloseDown`；
- 碎石恒为 `0`，禁止账号切换和付费购买；
- 理智药、刷图次数和公招消耗必须是有限显式值；
- 不接受任意 `maa-cli` 参数、Shell 命令或自定义任务映射；
- 同一时间最多一个活动运行；停止前必须同时匹配 PID 与进程创建时间；
- 日志每次最多返回 200 行、64 KiB，并应用配置的脱敏模式。

单人可信环境可通过 `confirmation = "none"` 显式跳过表单一步（其余校验不变，
摘要仍随调用参数展示在客户端授权界面）。完整威胁模型见
[安全模型](docs/security-model.md)。

## 📋 前置条件

| 依赖 | 说明 |
|---|---|
| Windows 10/11 | 当前仅支持 Windows |
| [maa-cli](https://github.com/MaaAssistantArknights/maa-cli) | 已安装并完成 `maa install`（MaaCore 与资源）；本适配器不代装 |
| 安卓模拟器 | MuMu 12 / 雷电等，已能被 MAA 正常连接 |
| MAA 图形版（推荐） | 已保存过连接与日常任务配置——一键初始化和默认计划都从这里读取 |
| 支持 MCP 的 Agent 客户端 | 执行确认需要客户端支持 MCP form elicitation（详见下文已知限制） |

<details>
<summary>maa-cli 快速安装（三条命令）</summary>

```powershell
winget install maa-cli
```

```powershell
# 官方要求在 maa install 前安装 VC++ 运行库 如果已经安装则可以跳过
winget install "Microsoft.VCRedist.2015+.x64" --override "/repair /passive /norestart" --uninstall-previous --accept-package-agreements --force
```

```powershell
maa install
```

maa-cli 本体放哪都可以：要么在 `PATH` 中，要么在本适配器
`config.toml` 里用 `maa_executable` 填绝对路径。其他安装方式见
[maa-cli 官方安装文档](https://github.com/MaaAssistantArknights/maa-cli/blob/main/crates/maa-cli/docs/zh-CN/install.md)。

</details>

## 🚀 安装

**方式一：下载发布包**（推荐）

从 [Releases](https://github.com/sac432/maa-mcp-adapter/releases) 下载
`maa-mcp-adapter-v0.1.0-windows-x86_64.zip`，解压到固定目录（如
`C:\Tools\maa-mcp-adapter\`），并用包内 `maa-mcp-adapter.exe.sha256` 校验：

```powershell
Get-FileHash C:\Tools\maa-mcp-adapter\maa-mcp-adapter.exe -Algorithm SHA256
```

**方式二：源码构建**（需要 Rust stable 1.97+）

```powershell
git clone https://github.com/sac432/maa-mcp-adapter.git
Set-Location maa-mcp-adapter
cargo build --release --locked
```

二进制位于 `target\release\maa-mcp-adapter.exe`。

## 🔌 接入你的 Agent

几乎所有支持 MCP 的环境都接受同一形态的注册（把路径换成你的实际位置）：

```json
{
  "mcpServers": {
    "maa": {
      "command": "C:\\Tools\\maa-mcp-adapter\\maa-mcp-adapter.exe"
    }
  }
}
```

常见客户端：

- **Claude Code**：`claude mcp add maa -s user -- C:\Tools\maa-mcp-adapter\maa-mcp-adapter.exe`
- **Codex**：`~/.codex/config.toml` 的 `[mcp_servers.maa]`
- **Cursor / Cline / Windsurf 等**：在各自的 MCP 配置文件里粘贴上面的 JSON

详细步骤（含 elicitation 权限设置）见
[客户端配置](docs/client-setup.md)。接入成功的标志：客户端列出 **12 个工具**。

## ⚡ 快速开始

**第 1 步：一键初始化**（新用户仅需一次，或者直接开始，agent会自动帮你初始化）。对 Agent 说：

> 帮我初始化 MAA 环境，我的 MAA 装在 `D:\MAA`。

Agent 会调用 `setup_environment`：自动发现 adb 地址与模拟器 → **表单展示
将写入的全部内容** → 你确认 → 生成 maa-cli 配置档与适配器配置，即时生效。

**第 2 步：跑日常**。对 Agent 说：

> 帮我跑一次默认日常。

计划从你 MAA 保存的任务队列派生（刷图需要你显式说明关卡与次数才会加入），
执行前弹出确认表单——**第一行就是本次消耗品上限**，核对后确认即可。
运行在后台独立进程中进行，关掉客户端也不影响；随时可以问状态、看日志、
喊停。

## 🛠 十二个 MCP Tools

| Tool | 用途 |
|---|---|
| `check_environment` | 检查 `maa-cli`、能力和配置档 |
| `setup_environment` | 从 MAA 图形版配置发现启动信息，经确认后一键初始化 |
| `get_daily_options` | 返回安全任务类型、字段和固定限制 |
| `create_daily_plan` | 创建 Draft；不传任务时从 MAA 保存设置派生默认计划 |
| `update_daily_plan` | 按 revision 修改仍可编辑的计划 |
| `preview_daily_plan` | 验证并生成摘要与计划哈希 |
| `cancel_daily_plan` | 取消尚未运行的计划 |
| `execute_daily_plan` | 逐字核对摘要与哈希、表单确认后启动 |
| `get_run_status` | 查询并核对运行状态 |
| `get_run_log` | 读取受限、脱敏的日志尾部 |
| `stop_run` | 精确停止记录中的 Windows Job |
| `collect_diagnostics` | 生成脱敏诊断报告并指向 GitHub 反馈 |

## ⚙️ 配置

适配器配置位于 `%LOCALAPPDATA%\maa-mcp-adapter\config.toml`，
按修改时间**热加载**（保存即生效）。要点：

- `confirmation`：`"elicitation"`（默认，表单确认）/ `"none"`（显式免表单）；
- `maa_gui_config`：指向 MAA 图形版 `gui.json`，驱动发现与默认计划派生；
- `[emulator]`：模拟器自动启停（含 MuMu/雷电 CLI 参数、adb 就绪探测、
  开机静置缓冲；只会关闭由适配器本次启动的模拟器）；
- `redaction_patterns`：日志脱敏模式。

全部字段说明见[适配器配置](docs/configuration.md)。

## 🧭 故障排查与反馈

- **`can_execute = false` 且提示 dry-run 失败**：多为 `api.maa.plus`
  网络受阻，按[文档](docs/configuration.md#故障排查apimaaplus-不可达)
  给 maa-cli 配置官方 GitHub 镜像即可；
- **其他问题**：让 Agent 调用 `collect_diagnostics`，把生成的脱敏报告
  （自动隐去用户目录与你配置的敏感词）粘贴到
  [Issues](https://github.com/sac432/maa-mcp-adapter/issues/new)。
  请勿在公开 Issue 中提交账号标识、原始日志或真实配置。

## ⚠️ 已知限制

- 仅支持 Windows；真实 `maa-cli` 兼容性按版本人工验收（当前：`v0.7.5` /
  MaaCore `v6.14.2`）；
- 执行确认依赖客户端正确实现 MCP form elicitation：Claude Code 终端 CLI
  已验证可用；部分图形界面客户端声明支持但不渲染表单（会表现为"自动拒绝"），
  此类环境可评估后使用 `confirmation = "none"`；
- MAA 的"刷到理智尽"模式不会被派生为默认计划——刷图必须显式给出关卡与
  有限次数，这是设计决定而非缺陷；
- 项目处于 `0.1.0` 实验阶段，完整验收进度见
  [发布清单](docs/release-checklist.md)。

## 🧪 开发与验证

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

自动测试只使用仓库内构建的 fake CLI，不操作真实游戏、模拟器或账号。

## 📄 许可证

本项目以 [Apache License 2.0](LICENSE) 发布。MAA、MaaCore、`maa-cli`
及其资源受各自许可证与权利人约束；本项目的许可证不授予分发它们的权利，
详见 [NOTICE](NOTICE)。
