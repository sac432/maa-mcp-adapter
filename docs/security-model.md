# 安全模型

## 目标

本项目只允许 Agent 在用户明确确认的有限日常范围内操作 MAA。它不是通用 MAA 遥控器、Shell 代理或账号管理工具。

## 信任边界

### 受信任

- 用户选择并安装的适配器二进制；
- 用户显式配置或 `PATH` 中发现的 `maa.exe`/`maa-cli.exe`；
- 当前 Windows 用户可写的适配器状态目录；
- MCP 客户端展示给用户的 form elicitation。

### 不受信任

- Agent 生成的自然语言和 Tool 参数；
- 未知或旧版 `maa-cli` 输出；
- 被外部修改的计划、任务文件、运行记录和日志路径；
- PID 本身；
- 日志内容；
- 用户未确认的客户端自动批准。

## 允许范围

唯一允许生成的 MAA 任务是：

- `StartUp`
- `Fight`
- `Recruit`
- `Infrast`
- `Mall`
- `Award`
- `CloseDown`

领域输入不暴露任意任务名、任意参数映射、Shell 字符串、账号切换或付费购买字段。

## 不可覆盖的策略

- `Fight.stone = 0`
- 禁止账号切换
- 禁止付费购买
- `Mall.force_shopping_if_credit_full = false`
- 保留信用商店黑名单
- 刷图次数、理智药和公招消耗有固定上限
- `CloseDown` 必须显式出现在已确认计划中

这些值在领域校验和任务文件生成两层固定，Agent 不能通过未知字段覆盖。

## 确认绑定

1. 完整计划规范化为稳定结构。
2. 对规范计划计算 SHA-256。
3. `execute_daily_plan` 必须携带 `expected_plan_hash` 和 `expected_summary`；
   后者与服务端生成的摘要逐字比对，不一致返回 `PLAN_SUMMARY_MISMATCH`。
   摘要因此出现在客户端工具授权界面的参数中，Agent 无法伪造展示内容。
4. elicitation 同时展示完整中文摘要和哈希。
5. 用户接受后，Server 重新加载计划。
6. 再次核对 state、revision、安全策略、摘要和哈希。
7. 任一变化返回 `PLAN_CHANGED`，不创建进程。

拒绝、取消、客户端不支持 elicitation 或哈希错误时都不会启动 MAA。

`confirmation = "none"` 是显式的降级开关：跳过第 4 步表单，其余校验不变。
默认关闭；开启意味着用户接受"工具授权界面中的摘要参数"作为唯一人工关卡，
仅适用于单人可信环境。

## 进程与并发

- `maa-cli` 通过直接绝对路径和参数数组启动，不经过 Shell。
- MCP stdin/stdout/stderr 不继承给后台任务。
- `locks/active-run.lock` 使用原子 `create_new`，同一时间最多一个活动运行。
- Windows 运行记录同时保存 PID 和进程创建时间，防止 PID 复用。
- 每个运行使用以 `run_id` 命名的 Job Object；强制停止作用于完整 Job。
- 运行记录持久化失败时，只在身份完全匹配后回滚该 Job。
- 配置 `[emulator]` 时，适配器只启动显式给定的可执行文件，不加入 Job；
  只有"由适配器本次启动"且 PID 与创建时间双重匹配的模拟器进程才会在
  `stop_after_run` 下被关闭，已在运行的模拟器不受影响。

Server 重启时：

- 身份匹配：恢复退出监视；
- 明确终态：释放旧锁；
- 身份冲突、记录缺失或证据不足：标记或保留 `Unknown`，继续持有保护锁。

`Unknown` 需要人工处理，系统不会自动开始新任务。

## 日志

`get_run_log`：

- 只接受 `stdout` 或 `stderr`；
- 路径必须等于该 `run_id` 的记录路径；
- 规范化后的文件仍必须位于运行目录；
- 从尾部最多读取 64 KiB；
- 最多返回 200 行；
- 丢弃被截断且无法完整脱敏的半行；
- 应用 `redaction_patterns` 后才返回模型。

日志脱敏是纵深防御，不应把秘密主动写入 MAA 日志。

## 保留与删除

默认只清理七天前的 `Succeeded`、`Failed`、`Stopped`。清理前完整枚举目录；出现未知文件、子目录、读取错误或不确定状态时不删除。记录文件最后删除，避免产生失联日志。

## 已知限制

- 当前只支持 Windows。
- 真实 `maa-cli` 的命令和输出兼容性必须在每个拟发布版本上人工验收。
- CTRL_BREAK 是否可用取决于 Windows 控制台环境；超时后使用 Named Job 强制回退。
- 本项目不能防御已经拥有同一 Windows 用户权限的恶意软件。
- 本项目不会判断游戏运营规则或账号风险；用户必须自行确认适用性。

## 漏洞报告

请不要在公开 Issue 中提交账号标识、日志、token 或真实配置。先提供最小化、已脱敏的复现步骤，并保留状态目录以便核对原子记录和进程身份。
