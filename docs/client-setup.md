# Claude Code 与 Codex 配置

以下示例假设发布二进制位于：

```text
C:\Tools\maa-mcp-adapter\maa-mcp-adapter.exe
```

先完成 [适配器配置](configuration.md)，再连接客户端。

## Codex

Codex CLI、Codex IDE 扩展和 Codex 桌面应用共享 `config.toml`。可在用户配置 `~/.codex/config.toml` 中加入：

```toml
[mcp_servers.maa]
enabled = true
required = false
command = 'C:\Tools\maa-mcp-adapter\maa-mcp-adapter.exe'
startup_timeout_sec = 10.0
tool_timeout_sec = 120.0
```

也可使用 CLI：

```powershell
codex mcp add maa -- 'C:\Tools\maa-mcp-adapter\maa-mcp-adapter.exe'
codex mcp list
```

若使用 granular approval policy，必须允许 `mcp_elicitations`，否则 `execute_daily_plan` 会以 `CLIENT_ELICITATION_UNSUPPORTED` 或客户端拒绝安全失败：

```toml
approval_policy = { granular = { mcp_elicitations = true } }
```

重新启动 Codex；在 TUI 使用 `/mcp`，或在桌面/IDE 的 MCP Server 设置中确认 `maa` 已连接并显示十二个 Tools。

卸载：

```powershell
codex mcp remove maa
```

如果当前版本的子命令不同，先运行 `codex mcp --help`。Codex 当前 MCP 配置说明见 [官方文档](https://learn.chatgpt.com/docs/extend/mcp)。

## Claude Code

用户级安装：

```powershell
claude mcp add --transport stdio --scope user maa -- 'C:\Tools\maa-mcp-adapter\maa-mcp-adapter.exe'
claude mcp list
claude mcp get maa
```

项目级配置可在项目根目录创建 `.mcp.json`：

```json
{
  "mcpServers": {
    "maa": {
      "type": "stdio",
      "command": "C:\\Tools\\maa-mcp-adapter\\maa-mcp-adapter.exe",
      "args": [],
      "env": {}
    }
  }
}
```

JSON 中的 Windows 反斜杠必须写成 `\\`。项目级 Server 首次使用时，Claude Code 会要求信任。

在 Claude Code 中输入 `/mcp`，确认 `maa` 已连接。Claude Code 会自动显示 MCP form elicitation；无需额外开关。官方安装、scope、移除和 elicitation 说明见 [Claude Code MCP 文档](https://code.claude.com/docs/en/mcp)。

卸载：

```powershell
claude mcp remove maa
```

如果使用项目级 `.mcp.json`，也可从文件中删除 `maa` 条目并重新启动 Claude Code。

## 首次验收

在两个客户端分别完成：

1. 查看 MCP Server，确认恰好出现十二个 Tools。
2. 输入：“帮我启动游戏刷日常。”
3. 确认 Agent 对配置档、任务类型、关卡、次数、理智药、公招和奖励等不明确项逐项提问。
4. 检查最终中文摘要明确写出碎石 `0`、禁止账号切换、禁止付费购买和是否关闭游戏。
5. 在 MCP 二次确认表单中先选择拒绝，确认未创建进程。
6. 再发起执行并接受准确摘要。
7. 调用 `get_run_status`、`get_run_log` 和 `stop_run`。
8. 关闭客户端后确认任务继续；重启客户端并确认状态和日志仍可查询。

## 常见问题

### Server 未连接

- 确认路径指向发布的 `.exe`，不是仓库目录。
- 从 PowerShell 直接运行二进制；正常情况下它会等待 STDIO MCP 输入，不打印普通日志。
- 检查客户端 MCP 状态面板中的启动错误。

### `CLIENT_ELICITATION_UNSUPPORTED`

客户端没有声明 MCP form elicitation，或策略拒绝该请求。不要绕过确认；升级客户端、允许 elicitation 后重新连接。

### `MAA_CLI_UNAVAILABLE`

在适配器 `config.toml` 设置 `maa_executable` 的绝对路径，或把用户已有的 `maa.exe`/`maa-cli.exe` 加入 `PATH`，然后重新调用 `check_environment`。
