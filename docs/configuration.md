# 适配器配置

适配器只读取自身状态目录中的 `config.toml`。默认位置：

```text
%LOCALAPPDATA%\maa-mcp-adapter\config.toml
```

它不会复制、迁移或修改 `maa-cli` 的配置档。

## 完整示例

```toml
maa_executable = 'C:\Tools\MAA\maa.exe'
state_directory = 'D:\AppData\maa-mcp-adapter'
retention_days = 7
redaction_patterns = [
  'account-123456',
  'device-serial-value',
  'private-token-value',
]
confirmation = "elicitation"
maa_gui_config = 'C:\MAA\config\gui.json'

[emulator]
path = 'D:\Program Files\Netease\MuMu Player 12\shell\MuMuPlayer.exe'
address = "127.0.0.1:16384"
startup_wait_seconds = 60
stop_after_run = false
```

Windows 路径建议使用 TOML 单引号字面字符串，以免对反斜杠重复转义。所有路径必须是绝对路径，不能包含 `.` 或 `..` 路径段。

## 字段

### `maa_executable`

可选。显式指定用户已经安装的 `maa.exe` 或 `maa-cli.exe`。

- 必须是存在的绝对文件路径。
- 配置后优先于 `PATH`。
- 未配置时依次在 `PATH` 中查找 `maa.exe`、`maa-cli.exe`。
- 适配器不会下载或更新 MAA。

### `state_directory`

可选。覆盖适配器自己的状态目录。默认是 `%LOCALAPPDATA%\maa-mcp-adapter`。

目录内容包括：

```text
config.toml
plans\<plan_id>.json
runs\<run_id>\task.toml
runs\<run_id>\record.json
runs\<run_id>\stdout.log
runs\<run_id>\stderr.log
locks\active-run.lock
```

状态目录与 MAA 配置目录不能互相包含，避免适配器任务文件污染用户的 MAA 配置。

### `retention_days`

可选，默认 `7`。Server 启动时只清理超过该期限且明确处于 `Succeeded`、`Failed` 或 `Stopped` 的运行。

以下内容永不自动清理：

- `Running`；
- `Unknown`；
- 记录损坏或无法读取的目录；
- 含有未知文件或子目录的运行目录。

### `redaction_patterns`

可选的字面字符串数组。`get_run_log` 返回内容前，会把每个非空匹配替换为 `[REDACTED]`。

建议加入：

- 账号标识；
- 模拟器地址或设备序列号；
- 本地路径中不希望发送给模型的片段；
- token、cookie 或私有服务标识。

这些模式不是正则表达式，按大小写敏感的字面文本匹配。不要把秘密本身提交到公开仓库；个人配置应保留在默认用户状态目录。

### `confirmation`

可选，默认 `"elicitation"`。

- `"elicitation"`：执行前必须通过 MCP form elicitation 表单确认（默认，推荐）。
- `"none"`：跳过表单，`execute_daily_plan` 在校验通过后直接启动。此时
  `expected_summary` 的逐字校验和客户端工具授权界面（参数中包含完整摘要）
  成为唯一的人工可见关卡。仅建议在单人、可信、明确接受风险的环境使用；
  客户端不支持表单渲染时这是完成执行的替代方式。

### `maa_gui_config`

可选。指向 MAA 图形版的 `gui.json` 绝对路径。配置后 `check_environment`
会**只读**解析并在 `gui_discovery` 字段回报其中的 adb 路径、连接地址和
模拟器路径，作为填写 maa-cli 配置档与 `[emulator]` 的参考。适配器不会
修改该文件。

### `[emulator]`

可选整节。配置后，执行计划前适配器会确保模拟器在线：`address` 可连通则
不做任何事；否则启动 `path` 指向的可执行文件并轮询 `address` 直至可连通
（未配置 `address` 时只做定长等待）。

- `path`：模拟器可执行文件的绝对路径（必填）。
- `address`：ADB 连接地址，如 `"127.0.0.1:16384"`，用于在线探测。
- `startup_wait_seconds`：启动等待上限，默认 `60`。
- `stop_after_run`：默认 `false`。为 `true` 时，运行进入终态后关闭
  **由适配器本次启动**的模拟器进程（PID 与进程创建时间双重匹配才关闭；
  已在运行的模拟器永远不会被关闭）。

## 修改配置后的步骤

1. 停止并重新启动 MCP 客户端中的 `maa` Server。
2. 调用 `check_environment`。
3. 确认 `can_plan = true`。
4. 只有所有能力探测通过且 `can_execute = true` 时才执行计划。

配置解析拒绝未知字段。错误不会自动退回不安全默认值。

## 故障排查：`api.maa.plus` 不可达

症状：`check_environment` 报 `absolute_task_path = false` 且诊断为
"absolute task path dry-run failed"；手动运行 `maa run` 立即报
`Error: Network error`。

原因：`maa-cli` 每次 `run`（含 `--dry-run`）都会先从
`https://api.maa.plus` 热更新关卡活动数据，失败即中止。部分网络环境对该
域名存在干扰。

解决：使用 `maa-cli` 自身的配置项，把热更新 API 指向官方 GitHub 镜像。在
`maa dir config` 所示目录中创建或编辑 `cli.toml`：

```toml
[hot_update]
api_url = "https://raw.githubusercontent.com/MaaAssistantArknights/MaaRelease/main/MaaAssistantArknights/api"
```

验证顺序：

1. `curl` 该镜像下的 `gui/StageActivityV2.json` 返回 200；
2. `maa --batch run <任务文件> --dry-run --profile <配置档>` 退出码 0；
3. `check_environment` 返回 `can_execute = true`。

这是 `maa-cli` 的公开配置，不涉及本适配器；适配器自身从不修改
`maa-cli` 的配置档。
