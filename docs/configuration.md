# 适配器配置

适配器只读取自身状态目录中的 `config.toml`。默认位置：

```text
%LOCALAPPDATA%\maa-mcp-adapter\config.toml
```

它不会修改或覆盖 `maa-cli` 的既有配置档；`setup_environment` 工具仅在用户经表单确认后**新建**配置档（已存在的文件永不触碰）。

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
path = 'D:\Program Files\Netease\MuMu Player 12\nx_main\MuMuManager.exe'
args = ["control", "-v", "0", "launch"]
address = "127.0.0.1:16384"
adb_path = 'D:\Program Files\Netease\MuMu Player 12\shell\adb.exe'
startup_wait_seconds = 120
boot_settle_seconds = 15
stop_after_run = false
stop_args = ["control", "-v", "0", "shutdown"]
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

可选。指向 MAA 图形版的 `gui.json` 绝对路径。适配器对它**只读**，并提供
两个用途：

1. `check_environment` 在 `gui_discovery` 字段回报其中的 adb 路径、连接
   地址和模拟器路径，作为填写 maa-cli 配置档与 `[emulator]` 的参考；
2. `create_daily_plan` 在调用方**不提供任何任务**时，从同目录
   `gui.new.json` 的 `TaskQueue`（MAA"一键长草"保存的任务队列）确定性地
   派生默认计划——"跑默认日常"由此等于"跑 MAA 里保存的日常"，而不是
   任何 LLM 的即兴发挥。

派生规则（只映射启用的白名单任务，数值向安全方向钳制）：

- 开始唤醒 → `start_up`；自动公招 → `recruit`（次数上限截断到 4，
  确认招募取自星级勾选，**加急许可恒为否**）；基建 → `infrast`；
  信用收支 → `mall`（访友/购物開關照搬）；领取奖励 → `award`。
- **不派生刷理智**：MAA 的刷理智是"刷到资源耗尽"模型，与本项目
  "有限显式次数"的安全模型不兼容，刷图必须由用户显式给出关卡与次数。
- 肉鸽、生息演算等白名单外任务一律忽略。

### `[emulator]`

可选整节。配置后，执行计划前适配器会确保模拟器在线：`address` 可连通则
不做任何事；否则启动 `path` 指向的可执行文件并轮询 `address` 直至可连通
（未配置 `address` 时只做定长等待）。

- `path`：模拟器可执行文件的绝对路径（必填）。
- `args`：启动参数数组，默认空。直接启动主程序通常只会打开设备管理
  界面；使用官方 CLI 才能自动开机。MuMu 12 示例：`MuMuManager.exe`
  加 `["control", "-v", "0", "launch"]` 直接开机 0 号设备。
- `address`：ADB 连接地址，如 `"127.0.0.1:16384"`，用于在线探测。
- `adb_path`：可选但强烈建议配置。模拟器端口通常在安卓系统启动完成前
  就已监听，仅测端口会让 MAA 抢跑（连接失败或触控初始化失败）。配置
  后会用 `adb shell getprop sys.boot_completed` 轮询真实启动完成。
- `startup_wait_seconds`：就绪等待总时限，默认 `60`（冷启动建议 `120`）。
- `boot_settle_seconds`：判定就绪后的额外静置秒数，默认 `10`，给系统
  服务和触控组件留初始化缓冲；已就绪的快路径不静置。
- `stop_after_run`：默认 `false`。为 `true` 时，运行进入终态后关闭
  由适配器本次启动的模拟器。
- `stop_args`：可选。配置后，关闭动作改为以 `path` + `stop_args` 执行
  一次关机命令（适合 MuMuManager 这类"启动器进程会立即退出"的 CLI，
  例如 `["control", "-v", "0", "shutdown"]`）；未配置时按启动记录的
  PID 与进程创建时间双重匹配终止进程。已在运行的模拟器（适配器未
  启动过的）在两种方式下都不会被关闭。

## 新用户一键初始化：`setup_environment`

新用户不必手写上述配置。安装 maa-cli 后，让 Agent 调用
`setup_environment`（可传 `maa_gui_config` 指向 MAA 图形版的
`gui.json`），它会：

1. 从 MAA 配置**只读**发现 adb 路径、连接地址与模拟器安装位置；
2. 生成一份 maa-cli 配置档（默认名 `mcp-daily`，含 `[connection]`）——
   **仅新建，已存在的文件永不覆盖**；
3. 自动定位受支持的模拟器 CLI（MuMuManager / 雷电 ldconsole 等）并把
   `maa_gui_config` 与 `[emulator]` 追加到本适配器的 `config.toml`
   （原文件先备份，合并结果先验证再原子写入；已有的节不重复添加）。

全部写入内容会先在确认表单中逐字展示，用户同意后才落盘。完成后重启
`maa` Server 并调用 `check_environment` 验证 `can_execute = true`。

## 修改配置后的步骤

配置按 `config.toml` 的修改时间**热加载**：`setup_environment` 或手动编辑
保存后，下一个工具调用即生效，无需重启 MCP Server。验证顺序：

1. 调用 `check_environment`。
2. 确认 `can_plan = true`。
3. 只有所有能力探测通过且 `can_execute = true` 时才执行计划。

配置解析拒绝未知字段。错误不会自动退回不安全默认值：配置损坏时所有
工具返回 `CONFIG_INVALID`，修复文件后自动恢复。仅当修改
`state_directory`（迁移状态目录）时仍建议重启 Server，避免运行中任务
的记录与新目录脱节。

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

这是 `maa-cli` 的公开配置，不涉及本适配器；适配器自身从不修改 `maa-cli` 的既有配置档。
