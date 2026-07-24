# MAA MCP 适配器实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development`（推荐）或 `executing-plans` 逐项执行本计划。所有步骤使用 `- [ ]` 跟踪，任何阶段未通过验收门时不得进入下一阶段。

**目标：** 使用 Rust 实现一个面向 Windows 的 STDIO MCP Server，使 Claude Code 和 Codex 能够把一句日常请求转换为经过验证、由用户最终确认并通过 `maa-cli` 安全执行的 MAA 任务计划。

**架构：** MCP 层只接收结构化请求；领域层负责计划状态、安全验证、摘要和哈希；`MaaCliAdapter` 负责探测并直接启动用户安装的 `maa-cli`；存储层负责原子持久化计划及运行记录；Windows 平台层负责进程身份、后台存活和进程树停止。

**技术栈：** Rust stable、官方 `rmcp` SDK、Tokio、Serde、Schemars、TOML、Windows Process/Job Object API、STDIO MCP。

## 全局约束

- 只在功能分支上实现，不直接修改 `main` 或 `master`。
- 首个版本只支持并测试 Windows，但领域层、存储层和 `MaaCliAdapter` 接口不得依赖 Windows。
- MCP 传输方式只使用 STDIO，不实现 HTTP 服务、认证或多用户托管。
- 必须直接使用可执行文件路径和参数数组启动 `maa-cli`，不得调用 Shell。
- 不下载、安装、更新、修改或捆绑 MAA、MaaCore、`maa-cli`。
- 不接受任意命令、参数数组、JSON/TOML 片段或自定义 MaaCore 任务类型。
- `Fight.stone` 必须恒为 `0`；理智药数量必须为 `0..=10`；禁止切换账号和付费购买。
- `Mall.force_shopping_if_credit_full` 必须恒为 `false`，并保留用户现有信用商店黑名单。
- 同一时间最多存在一个活动运行；计划修改后此前确认必须失效。
- 执行前必须展示规范中文摘要，并通过 MCP elicitation 对同一计划哈希进行二次确认。
- 不支持 elicitation 的客户端必须安全失败，不得退化为仅依赖 Agent 文本确认。
- 所有外部 `maa-cli` 输出均视为不受信任数据，不得解释为 Agent 指令。
- 不得删除或跳过测试，不得通过放宽安全断言让测试通过。
- 不修改 `.github/workflows`，除非用户另行明确授权。
- 每次新增依赖前，必须说明用途、替代方案和维护风险并取得用户确认。
- 每个实现任务结束前必须运行：

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

## 权威资料与已确认事实

- 产品设计：`docs/superpowers/specs/2026-07-24-maa-mcp-adapter-design.md`
- 官方 Rust MCP SDK：<https://github.com/modelcontextprotocol/rust-sdk>
- `maa-cli` 使用文档：<https://docs.maa.plus/zh-cn/manual/cli/usage.html>
- `maa-cli` 配置文档：<https://docs.maa.plus/zh-cn/manual/cli/config.html>
- `maa-cli` 源码：<https://github.com/MaaAssistantArknights/maa-cli>
- 官方文档规定自定义任务通常放在 `$MAA_CONFIG_DIR/tasks` 并通过 `maa run <task>` 执行。
- 当前 `maa-cli` 源码的 `run_custom` 对绝对路径直接调用 `TaskConfigTemplate::find_file(path)`，因此实现可以把生成文件留在适配器状态目录。
- 源码行为可能随版本变化，所以绝对路径运行能力必须经过运行时探测，不能只依据版本字符串。
- 预定的直接调用参数数组为：

```text
[
  "--batch",
  "--log-file=<adapter-log-path>",
  "run",
  "<absolute-generated-task-path>",
  "--profile",
  "<profile-name>"
]
```

- 在真正执行前，适配器必须先以 `--dry-run` 对同一绝对任务文件和配置档进行验证。

## 阶段拆分策略

采用纵向可验收切片，而不是“先写完所有模块，最后一次集成”。

1. **纵向切片（采用）：** 每阶段都形成可运行、可测试的能力链，较早暴露 MCP 与 `maa-cli` 的真实兼容问题。
2. **按技术层横向完成：** 先写完领域层、再写存储层和 MCP 层；短期文件整齐，但集成风险集中到后期。
3. **一次性原型后重写：** 最快看到演示，但容易把 Shell 调用、宽松输入和不可恢复状态带入正式实现。

## 计划文件结构

```text
maa-mcp-adapter/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── .gitignore
├── LICENSE
├── NOTICE
├── README.md
├── src/
│   ├── main.rs
│   ├── server.rs
│   ├── config.rs
│   ├── error.rs
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── plan.rs
│   │   ├── task.rs
│   │   └── run.rs
│   ├── planning/
│   │   ├── mod.rs
│   │   ├── policy.rs
│   │   ├── normalize.rs
│   │   ├── summary.rs
│   │   └── hash.rs
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── atomic.rs
│   │   ├── plans.rs
│   │   ├── runs.rs
│   │   └── retention.rs
│   ├── maa_cli/
│   │   ├── mod.rs
│   │   ├── capabilities.rs
│   │   ├── inspect.rs
│   │   ├── task_file.rs
│   │   └── process.rs
│   ├── platform/
│   │   ├── mod.rs
│   │   └── windows.rs
│   └── tools/
│       ├── mod.rs
│       ├── environment.rs
│       ├── plans.rs
│       └── runs.rs
├── tests/
│   ├── support/
│   │   ├── mod.rs
│   │   └── fake_maa.rs
│   ├── environment_tools.rs
│   ├── plan_tools.rs
│   ├── execution_flow.rs
│   ├── recovery.rs
│   └── stdio_protocol.rs
└── docs/
    ├── configuration.md
    ├── client-setup.md
    ├── security-model.md
    └── superpowers/
        ├── specs/
        └── plans/
```

每个源文件只承担一种职责。`tools` 不直接操作文件或进程；它们调用 `planning`、`storage` 和 `maa_cli` 服务。`platform/windows.rs` 是唯一允许直接使用 Windows API 的模块。

## 建议依赖及实施前确认门

本计划不会直接添加依赖。阶段 0 实施前，必须逐项向用户解释并取得确认。

| 依赖 | 用途 | 不采用时的代价 |
|---|---|---|
| `rmcp` | 官方 MCP Server、Tool Schema、STDIO、elicitation | 需要自行实现协议，风险不可接受 |
| `tokio` | 异步 STDIO、子进程等待和超时 | 需要自行实现异步运行时 |
| `serde`、`serde_json`、`toml` | 计划、记录及 MAA 任务文件序列化 | 手写解析容易产生安全和兼容错误 |
| `schemars` | MCP Tool 输入输出 JSON Schema | 需要手写且维护重复 Schema |
| `thiserror` | 稳定错误类型 | 可以手写，但会产生大量样板代码 |
| `uuid` | 不可预测的 `plan_id` 与 `run_id` | 自制 ID 容易冲突 |
| `sha2`、`hex` | 规范计划 SHA-256 哈希 | 不能使用不稳定的默认哈希器 |
| `directories` | Windows 应用数据目录 | 需要维护平台目录规则 |
| `windows-sys` | PID 创建时间、Named Job Object、进程终止 | 无法可靠验证和停止 Windows 进程树 |
| `tempfile`（仅开发） | 原子写入、恢复和路径测试 | 测试清理更复杂 |

批准后锁定准确版本并提交 `Cargo.lock`。不得使用 Git 分支依赖；正式依赖必须来自 crates.io 的稳定版本。

---

## 阶段 0：工程基线与最小 STDIO Server

### 阶段目标

建立可重复构建的 Rust 工程、统一错误出口和最小 STDIO MCP 握手，使后续阶段始终在可格式化、可 lint、可测试的基线上开发。

### 要完成的任务

- [x] **任务 0.1：确认环境和依赖**

  运行 `rustc --version`、`cargo --version`，记录 Windows 架构；向用户提交上一节依赖说明并取得明确同意。

  验收要求：使用受支持的 Rust stable；未经同意时停止，不创建 `Cargo.toml`。

- [x] **任务 0.2：创建工程骨架**

  创建：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`.gitignore`、`src/main.rs`、`src/server.rs`、`src/error.rs`。

  `main.rs` 只负责装配和启动：

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    maa_mcp_adapter::server::serve_stdio().await
}
```

  若使用单 package 布局，则同时创建 `src/lib.rs` 导出模块，集成测试只通过公共接口访问内部服务。

- [x] **任务 0.3：实现最小 MCP 初始化**

  `server::serve_stdio()` 使用 `tokio::io::{stdin, stdout}` 和 `rmcp::ServiceExt` 启动 Server。此时可以没有业务 Tool，但必须提供固定服务名、版本、中文服务器说明和 tools capability。

- [x] **任务 0.4：添加协议冒烟测试**

  `tests/stdio_protocol.rs` 启动构建后的二进制，发送 `initialize`、`notifications/initialized` 和 `tools/list`，验证返回合法 JSON-RPC，且 stdout 不混入日志文本。

- [x] **任务 0.5：提交阶段成果**

```text
git add Cargo.toml Cargo.lock rust-toolchain.toml .gitignore src tests
git commit -m "chore: bootstrap Rust MCP server"
```

### 阶段验收标准

- [x] `cargo build --locked` 成功。
- [x] `cargo fmt --check` 成功。
- [x] `cargo clippy --all-targets --all-features -- -D warnings` 成功。
- [x] `cargo test --all-targets` 成功，协议冒烟测试至少覆盖初始化和 `tools/list`。
- [x] 运行服务器时 stdout 只有 MCP 帧，诊断信息只写 stderr。
- [x] Git 分支不是 `main` 或 `master`，工作区只包含预期文件。

### 阶段审查记录

- 实现提交：`cdec74c chore: bootstrap Rust MCP server`
- TDD RED：测试因缺少 `initialize` 响应而失败；首次 GREEN 又正确发现服务名称误报为 `rmcp`。
- TDD GREEN：修正服务身份后协议测试通过。
- 审查修正：为子进程协议测试增加 5 秒超时，并补充版本和中文 instructions 断言。
- 最终结论：无 Critical 或 Important 遗留问题，可以进入阶段 1。

---

## 阶段 1：`maa-cli` 兼容性与环境检查

### 阶段目标

在实现计划业务之前证明适配器能够安全定位并探测真实 `maa-cli`，且能够以绝对路径和参数数组验证生成任务文件。

### 核心接口

创建 `src/maa_cli/mod.rs`，固定接口名称：

```rust
pub struct MaaCliAdapter {
    executable: std::path::PathBuf,
}

pub struct MaaEnvironment {
    pub executable: std::path::PathBuf,
    pub cli_version: String,
    pub core_version: Option<String>,
    pub config_dir: std::path::PathBuf,
    pub profiles: Vec<String>,
    pub capabilities: MaaCliCapabilities,
}

pub struct MaaCliCapabilities {
    pub batch: bool,
    pub profile: bool,
    pub dry_run: bool,
    pub absolute_task_path: bool,
    pub log_file: bool,
}
```

### 要完成的任务

- [x] **任务 1.1：实现可执行文件定位**

  `src/config.rs` 定义适配器配置；解析优先级为 `config.toml` 中的显式绝对路径，其次是 Windows `PATH` 中的 `maa.exe`，最后是 `maa-cli.exe`。拒绝目录、相对覆盖路径和不存在的文件。

- [x] **任务 1.2：实现无 Shell 命令执行器**

  `src/maa_cli/process.rs` 只能通过 `tokio::process::Command::new(&executable).args(&args)` 调用程序。接口不接受拼接完成的命令字符串。

- [x] **任务 1.3：实现能力探测**

  依次探测：

```text
maa version
maa dir config
maa run --help
maa --help
```

  只解析稳定字段；无法确认 `--batch`、`--profile`、`--dry-run`、`--log-file` 或绝对任务路径能力时，将 `can_execute` 设为 `false`。

- [x] **任务 1.4：实现配置档发现**

  从 `maa dir config` 返回的规范化目录读取 `profiles` 子目录，只返回合法文件名对应的配置档名称，不读取或返回配置内容中的账号标识和其他秘密。

- [x] **任务 1.5：验证绝对任务路径**

  在适配器状态目录写入最小禁用任务文件，通过以下参数执行安全 dry-run：

```text
maa --batch run <absolute-path> --dry-run --profile <profile>
```

  探测成功后删除探测文件；失败时保留经过脱敏的诊断，不猜测兼容性。

- [x] **任务 1.6：创建 fake maa 测试夹具**

  `tests/support/fake_maa.rs` 模拟版本、帮助、配置目录、成功 dry-run、缺失能力、非零退出和含中文/空格路径。测试必须检查收到的是参数数组，而不是 Shell 字符串。

- [x] **任务 1.7：提交阶段成果**

```text
git add src/config.rs src/error.rs src/maa_cli tests/support tests/environment_tools.rs
git commit -m "feat: inspect maa-cli environment safely"
```

### 阶段验收标准

- [x] 显式路径、`maa.exe` 和 `maa-cli.exe` 三种定位路径均有测试。
- [x] 路径含空格、中文和 Shell 元字符时仍作为单个参数传递。
- [x] 能力缺失、版本命令失败或配置目录无效时安全失败。
- [x] 绝对任务文件只写入适配器状态目录，不写入用户 `$MAA_CONFIG_DIR/tasks`。
- [x] 真实 `maa-cli` 的 `--dry-run` 兼容测试通过；若本机未配置 MaaCore，记录为人工环境阻塞，不伪造成功。
- [x] 三条必需 Cargo 检查全部通过。

### 阶段审查记录

- 实现提交：`042f8d5 feat: inspect maa-cli environment safely`
- TDD RED：依次确认了配置模块缺失、PATH 安全失败类型缺失、TOML 解析缺失、无 Shell 进程层缺失、环境检查接口缺失、stderr 泄露、目录双向重叠和非法配置档名称等失败。
- TDD GREEN：完成可执行文件定位、参数数组进程执行、稳定字段解析、合法配置档枚举、能力关闭策略和状态目录绝对路径 dry-run。
- 审查修正：非零退出不再向调用方展示 stderr；状态目录与 MAA 配置目录任一方向重叠都会拒绝；过滤空白、`.`、`..` 和控制字符配置档名称；固定 fake 夹具锁文件，避免测试污染工作树。
- 自动验证：`cargo fmt --check`、`cargo build --locked`、严格 clippy 和 `cargo test --locked --all-targets` 全部通过；测试计数为 1 个单元测试、16 个环境测试和 1 个 stdio 协议测试。
- 人工环境阻塞：PATH、MAA 源码目录及当前工作区均未发现真实 `maa.exe` 或 `maa-cli.exe`，因此未声称真实 MaaCore dry-run 成功。发布前须在已安装并配置 MaaCore/profile 的 Windows 环境补做该项。
- 最终结论：阶段 1 的代码与自动验收无 Critical 或 Important 遗留问题；真实 `maa-cli` 兼容性保留为明确的发布前人工验收项，可以进入阶段 2。

---

## 阶段 2：领域模型、计划状态机和确定性安全策略

### 阶段目标

在不接触 MCP、文件系统或进程的纯 Rust 代码中，完整表达允许的日常任务、缺失字段、状态转换和不可绕过的安全规则。

### 核心接口

```rust
pub enum PlanState {
    Draft,
    Ready,
    Confirmed,
    Running,
    Succeeded,
    Failed,
    Stopped,
    Cancelled,
}

pub enum DailyTask {
    StartUp(StartUpPlan),
    Fight(FightPlan),
    Recruit(RecruitPlan),
    Infrast(InfrastPlan),
    Mall(MallPlan),
    Award(AwardPlan),
    CloseDown(CloseDownPlan),
}

pub struct DailyPlan {
    pub plan_id: PlanId,
    pub profile: String,
    pub tasks: Vec<DailyTask>,
    pub state: PlanState,
    pub revision: u64,
    pub confirmed_hash: Option<PlanHash>,
}
```

`FightPlan` 不提供可写 `stone` 字段；转换到 MAA 配置时固定写入 `stone = 0`。`MallPlan` 不提供可写 `force_shopping_if_credit_full` 字段。

### 要完成的任务

- [x] **任务 2.1：先写状态机失败测试**

  覆盖 `Draft -> Ready -> Confirmed -> Running`、终止状态、取消、任意修改使 `Confirmed -> Ready`，以及非法跳转拒绝。

- [x] **任务 2.2：实现任务白名单类型**

  使用封闭 enum 表示七类任务。反序列化必须拒绝未知 variant 和未知字段，禁止 `flatten` 任意参数映射。

- [x] **任务 2.3：实现必填字段发现**

  返回稳定的 `MissingField` 枚举及合法选项。战斗至少要求类别/关卡、次数、理智药最大量；有限资源相关公招参数必须明确。

- [x] **任务 2.4：实现安全策略**

  `planning/policy.rs` 提供：

```rust
pub fn validate_plan(plan: &DailyPlan) -> ValidationReport;
pub fn validate_transition(from: PlanState, to: PlanState) -> Result<(), DomainError>;
```

  覆盖次数边界、理智药 `0..=10`、重复任务规则、禁止账号切换、禁止强制信用购物和所有非白名单任务。

- [x] **任务 2.5：实现规范化、中文摘要和哈希**

  规范化固定任务顺序、字段顺序和默认值；摘要明确列出有限资源与固定安全值；哈希对规范 JSON 的 UTF-8 字节计算 SHA-256。

- [x] **任务 2.6：验证哈希不变量**

  相同语义的输入顺序差异必须得到相同哈希；任何会改变执行行为的字段必须改变哈希；仅时间戳和日志路径不得进入哈希。

- [x] **任务 2.7：提交阶段成果**

```text
git add src/domain src/planning tests/plan_tools.rs
git commit -m "feat: add safe daily plan domain"
```

### 阶段验收标准

- [x] 七类允许任务均有合法与非法测试。
- [x] 所有禁止任务类型和未知字段都在领域边界被拒绝。
- [x] 无法通过反序列化构造 `stone > 0` 或强制信用购物。
- [x] 摘要始终包含关卡、次数、理智药、源石为零、公招消耗、信用购物、账号及关闭游戏决定。
- [x] 修改执行相关字段后确认哈希必然失效。
- [x] 所有状态转换测试和三条必需 Cargo 检查通过。

### 阶段审查记录

- 实现提交：`0064cff feat: add safe daily plan domain`
- TDD RED：依次确认状态机模块、七类任务边界、计划/缺失字段、规范化/摘要/哈希/修订接口缺失，并通过增强测试发现同关卡 Fight 判重不足、战斗类别未中文化、profile/hash 输入验证不足。
- TDD GREEN：完成封闭任务枚举、严格 serde、稳定缺失字段选项、确定性安全策略、规范任务顺序、中文汇总、SHA-256 确认哈希和原子修订。
- 审查修正：同类别+关卡的 Fight 即使次数不同也拒绝重复；摘要将四类战斗映射为中文；状态目录等外部能力未进入纯领域层；profile 和确认哈希增加严格语法验证；逐字段验证所有执行变化都会改变哈希。
- 安全常量：领域输入不存在 `stone`、`force_shopping_if_credit_full`、账号切换或任意任务映射；规范计划固定 `stone = 0`、强制信用购物为 `false`、保留黑名单为 `true`、账号切换和付费购买为 `false`。
- 自动验证：根 crate 与 fake 夹具格式检查、`cargo build --locked`、严格 clippy 和 `cargo test --locked --all-targets` 全部通过；测试计数为 1 个单元测试、16 个环境测试、17 个计划测试和 1 个 stdio 协议测试，共 35 项。
- 最终结论：无 Critical 或 Important 遗留问题，可以进入阶段 3。

---

## 阶段 3：计划持久化和规划服务

### 阶段目标

提供可在进程重启后恢复的本地计划仓库，以及创建、更新、预览、取消四个不启动 MAA 的应用服务。

### 核心接口

```rust
pub trait PlanRepository {
    fn create(&self, plan: &DailyPlan) -> Result<(), StorageError>;
    fn load(&self, id: &PlanId) -> Result<DailyPlan, StorageError>;
    fn replace(&self, expected_revision: u64, plan: &DailyPlan)
        -> Result<(), StorageError>;
}

pub struct PlanService<R: PlanRepository> {
    repository: R,
}
```

### 要完成的任务

- [x] **任务 3.1：实现状态目录解析**

  默认使用 Windows 应用数据目录，测试通过显式覆盖目录。拒绝状态目录逃逸、文件路径组件中的 `..` 和非法 ID。

- [x] **任务 3.2：实现原子替换**

  同目录临时文件写入、flush、关闭后再 rename；写入中断时旧记录仍可读取，临时文件不能被识别为计划。

- [x] **任务 3.3：实现带 revision 的计划仓库**

  `plans/<plan_id>.json` 保存完整状态。更新必须携带期望 revision，防止两个 Agent 调用相互覆盖。

- [x] **任务 3.4：实现创建和更新服务**

  创建返回规范计划、缺失字段和下一问题；更新只接受结构化 patch enum，拒绝未知字段，并在任何变化后清除 `confirmed_hash`。

- [x] **任务 3.5：实现预览和取消服务**

  完整有效的 `Draft` 经预览转为 `Ready`；不完整计划返回缺失字段；只有未启动计划可以取消。

- [x] **任务 3.6：实现计划清理**

  只清理超过七天且处于 `Cancelled` 的计划；`Ready`、`Confirmed` 和状态不明确记录不自动删除。

- [x] **任务 3.7：提交阶段成果**

```text
git add src/storage src/planning tests/plan_tools.rs
git commit -m "feat: persist and update daily plans"
```

### 阶段验收标准

- [x] 模拟写入中断后，旧计划仍完整可读。
- [x] revision 冲突返回稳定错误，不发生静默覆盖。
- [x] 重启 `PlanService` 后可继续更新已有草稿。
- [x] 预览只在计划完整时进入 `Ready`。
- [x] 已取消、运行中和终止计划的非法修改均被拒绝。
- [x] 三条必需 Cargo 检查全部通过。

### 阶段审查记录

- 实现提交：`40ea6e6 feat: persist and update daily plans`
- TDD RED：依次确认 storage 模块、规划服务和固定清理 API 缺失；测试先覆盖重启恢复、临时文件隔离、revision 冲突、严格 patch、preview/cancel 和清理失败关闭。
- TDD GREEN：实现 `plans/<UUID>.json` 仓库、同目录临时文件写入/flush/sync/关闭后替换、Windows `ReplaceFileW` 写穿替换、revision CAS、结构化 patch 服务和七天 Cancelled 清理。
- 审查修正：普通 Draft 更新不再提前转 Ready；只有 preview 可提升完整 Draft；清理 API 内部固定七天，调用方不能传任意 cutoff；仓库锁改为进程级全局锁，覆盖同一服务器内多个仓库/Agent 实例。
- 安全边界：状态目录必须是无 `.`/`..` 的绝对路径；计划文件名只由强类型 UUID 生成；损坏、未知、临时、Ready、Confirmed 和状态不明确记录均不自动删除。
- 自动验证：根 crate 与 fake 夹具格式检查、`cargo build --locked`、严格 clippy 和 `cargo test --locked --all-targets` 全部通过；最后一次测试计数为 1 个单元测试、16 个环境测试、24 个计划/存储/服务测试和 1 个 stdio 协议测试，共 42 项。
- 最终结论：无 Critical 或 Important 遗留问题，可以进入阶段 4。

---

## 阶段 4：受控 MAA 任务文件生成与 dry-run

### 阶段目标

把规范计划映射为最小 TOML 自定义任务文件，确保每个输出字段来自封闭领域类型或服务器固定值，并在执行前由真实 `maa-cli` dry-run 验证。

### 要完成的任务

- [x] **任务 4.1：定义内部任务文件 DTO**

  `src/maa_cli/task_file.rs` 定义仅用于序列化的 `GeneratedTaskFile`、`GeneratedTask` 和各任务参数结构。不得使用 `serde_json::Value` 或任意 map 承载 Agent 输入。

- [x] **任务 4.2：逐类实现映射**

  按固定顺序输出 `StartUp`、`Fight`、`Recruit`、`Infrast`、`Mall`、`Award`、`CloseDown`。未选择的任务不输出；每种任务建立独立 golden fixture。

- [x] **任务 4.3：强制写入安全字段**

```toml
[[tasks]]
type = "Fight"
[tasks.params]
stone = 0

[[tasks]]
type = "Mall"
[tasks.params]
force_shopping_if_credit_full = false
```

  实际输出同时包含计划中已验证的其他字段；上面两个值不能从请求参数覆盖。

- [x] **任务 4.4：生成到运行专属目录**

  写入 `runs/<run_id>/task.toml`，先完成原子写入，再将规范绝对路径交给 `maa-cli`。验证路径位于状态目录内。

- [x] **任务 4.5：实现执行前 dry-run**

  对准确的生成文件、配置档和地址覆盖执行 `maa --batch run <absolute-path> --dry-run ...`。dry-run 后再次读取文件并核对 SHA-256，防止检查与执行之间文件被替换。

- [x] **任务 4.6：提交阶段成果**

```text
git add src/maa_cli/task_file.rs tests
git commit -m "feat: generate validated maa task files"
```

### 阶段验收标准

- [x] 每类任务的 golden 文件与 MAA 参数文档一致。
- [x] 生成器 API 不存在任意 JSON、TOML、任务名或参数注入口。
- [x] 所有 Fight golden 文件都包含 `stone = 0`。
- [x] 所有 Mall golden 文件都包含 `force_shopping_if_credit_full = false`。
- [x] 中文、空格和长路径可通过 fake maa 与真实 `maa-cli --dry-run`。
- [x] dry-run 后文件改变时执行被拒绝。
- [x] 三条必需 Cargo 检查全部通过。

### 阶段审查记录

- 实现提交：`07fae4c feat: generate validated maa task files`
- TDD RED：先确认 task_file 模块、RunId/store/artifact、dry-run 与完整性错误缺失；golden 测试还发现 toml 1.1 的值表达式解析 API 不适合完整文档，并修正为文档解析。
- TDD GREEN：完成七类仅序列化 DTO、规范顺序映射、运行专属绝对路径、同目录原子写入、profile 绑定、参数数组 dry-run 和执行后 SHA-256 复核。
- 安全字段：Fight 永远写 `stone = 0`；Mall 永远写 `force_shopping_if_credit_full = false`；配置黑名单通过专用 `TaskFilePolicy` 保留，并且即使配置遗漏也强制补入“加急许可”。
- 审查修正：重复 RunId 拒绝且不覆盖既有 task；artifact 与生成时 profile 绑定；任务文件校验方法保留为 crate 内接口，供阶段 5 在真正执行前再次核对。
- 自动验证：根 crate 与 fake 夹具格式检查、`cargo build --locked`、严格 clippy 和 `cargo test --locked --all-targets` 全部通过；测试计数为 1 个单元测试、16 个环境测试、24 个计划/存储/服务测试、1 个 stdio 协议测试和 7 个任务文件测试，共 49 项。
- 人工环境阻塞：fake maa 已覆盖中文、空格、长路径、地址元字符和 dry-run 篡改；本机仍未发现真实 `maa.exe`/`maa-cli.exe`，因此未声称真实 MaaCore dry-run 成功，发布前必须补做。
- 最终结论：自动化范围内无 Critical 或 Important 遗留问题；真实 `maa-cli` 验证保留为明确人工验收项，可以进入阶段 5。

---

## 阶段 5：MCP 环境与计划 Tools

### 阶段目标

通过 STDIO 暴露六个不启动 MAA 的 MCP Tools，并验证 Tool Schema、风险标注和结构化错误。

### 要完成的任务

- [x] **任务 5.1：实现 `check_environment`**

  调用阶段 1 的检查服务；返回可执行文件发现方式、版本、能力、配置档名称、`can_plan`、`can_execute` 和安全诊断。不得返回配置内容或完整环境变量。

- [x] **任务 5.2：实现 `get_daily_options`**

  返回七类白名单任务、四类战斗语义选择、需要收集的字段以及固定安全限制。不得访问网络获取活动内容。

- [x] **任务 5.3：实现计划 Tools**

  注册：

```text
create_daily_plan
update_daily_plan
preview_daily_plan
cancel_daily_plan
```

  输入结构通过 Schemars 生成，所有 struct 使用 `deny_unknown_fields`。

- [x] **任务 5.4：配置 Tool annotations**

  `check_environment`、`get_daily_options` 标记为只读。`preview_daily_plan` 不产生外部 MAA 副作用，但会把完整计划从 `Draft` 持久化为 `Ready`，因此设置 `readOnlyHint = false`、`destructiveHint = false`、`idempotentHint = true`。创建、更新和取消同样准确标记为本地状态修改，不得标记为外部 MAA 执行。

- [x] **任务 5.5：实现稳定错误映射**

  每个错误包含机器码、中文消息、安全详情和恢复建议。内部路径只在确实需要用户修复配置时返回。

- [x] **任务 5.6：编写 MCP 协议测试**

  通过真实 STDIO 测试 `tools/list` 中恰好出现六个阶段内 Tool、Schema 拒绝未知字段、完整创建—更新—预览—取消流程。

- [x] **任务 5.7：提交阶段成果**

```text
git add src/server.rs src/tools tests/stdio_protocol.rs tests/environment_tools.rs tests/plan_tools.rs
git commit -m "feat: expose daily planning MCP tools"
```

### 阶段验收标准

- [x] `tools/list` 中名称、说明、输入 Schema 和 annotations 与设计一致。
- [x] 一句话意图可以创建 `Draft`，返回缺失字段和一个明确的下一问题。
- [x] 未知字段、禁止字段和非法 ID 都返回稳定结构化错误。
- [x] 除 `check_environment` 的有界只读能力探测外，其余阶段 5 Tool 不创建 `maa-cli` 进程；所有阶段 5 Tool 都不会启动日常执行或游戏。
- [x] stdout 仍只包含 MCP 协议数据。
- [x] 三条必需 Cargo 检查全部通过。

### 阶段审查记录

- 实现提交：`1fa903b feat: expose daily planning MCP tools`
- TDD RED：先通过真实 STDIO 将 `tools/list` 从空列表固定为恰好六个 Tool；随后覆盖一句话创建 Draft、严格未知字段、禁止 `stone`、完整创建—更新—预览—取消流程、非法/不存在 ID、revision 冲突和损坏配置。
- TDD GREEN：`check_environment` 复用阶段 1 探测并只返回发现来源、版本、能力、配置档和安全诊断；`get_daily_options` 静态返回七类任务、四类战斗语义与固定安全限制，不访问网络；四个计划 Tool 仅调用既有计划和存储服务。
- Schema 与错误：所有输入及嵌套任务/patch 使用 Schemars 和 `deny_unknown_fields`；Tool 错误统一包含机器码、中文消息、安全详情和恢复建议，只有损坏配置需要用户修复时才返回 `config.toml` 路径。
- annotations：环境和选项 Tool 只读；创建为非破坏、非幂等本地写入；预览为非破坏、幂等本地写入；更新和取消为破坏性、幂等本地状态修改；全部 `openWorldHint = false`，均不表示外部 MAA 执行。
- 审查修正：把启动配置读取从 `tools` 移到独立 `runtime` 应用层，确保 Tool 层不直接读文件或创建进程；并修正文档中“环境检查需要探测进程”与“所有 Tool 不创建进程”的矛盾，冻结为“仅环境检查可做有界只读探测，任何 Tool 都不得启动日常执行或游戏”。
- 自动验证：根 crate 与 fake 夹具格式检查、`cargo build`、严格 clippy 和全量 `cargo test` 全部通过；测试计数为 1 个单元测试、16 个环境测试、24 个计划/存储/服务测试、4 个 stdio 协议测试和 7 个任务文件测试，共 52 项。
- 人工环境阻塞：本机仍未发现真实 `maa.exe`/`maa-cli.exe`，因此 `check_environment` 的真实 MAA 输出兼容性与真实 MaaCore dry-run 仍是发布前人工验收项。
- 最终结论：自动化范围内无 Critical 或 Important 遗留问题；阶段 5 可以进入阶段 6。

---

## 阶段 6：计划绑定确认、单任务锁和后台执行

### 阶段目标

实现唯一能启动 MAA 的 `execute_daily_plan`，确保二次确认与准确计划哈希绑定，并把运行状态持久化到可恢复注册表。

### 核心接口

```rust
pub struct RunRecord {
    pub run_id: RunId,
    pub plan_id: PlanId,
    pub plan_hash: PlanHash,
    pub pid: u32,
    pub process_start_identity: ProcessStartIdentity,
    pub task_path: std::path::PathBuf,
    pub stdout_path: std::path::PathBuf,
    pub stderr_path: std::path::PathBuf,
    pub status: RunStatus,
}
```

### 要完成的任务

- [x] **任务 6.1：实现全局活动任务锁**

  通过 `locks/active-run.lock` 的原子 `create_new` 获取锁。锁内记录 `run_id`；锁有效性的重启判断由任务 7.6 结合运行记录和进程身份完成。

- [x] **任务 6.2：实现 MCP elicitation**

  `execute_daily_plan` 只接受 `plan_id` 和 `expected_plan_hash`。服务器重新加载计划、验证状态并展示规范摘要，请求单一布尔确认。

- [x] **任务 6.3：处理 elicitation 三种结果**

  `accept` 继续；`decline` 返回未执行结果；`cancel` 返回取消结果。客户端未声明 elicitation capability 时返回 `CLIENT_ELICITATION_UNSUPPORTED`。

- [x] **任务 6.4：执行最终一致性检查**

  elicitation 返回后重新加载计划、检查 revision、重新验证安全策略、重新生成摘要及哈希。任一值变化则返回 `PLAN_CHANGED`，不创建进程。

- [x] **任务 6.5：创建后台进程**

  打开 stdout/stderr 文件后，使用直接路径和参数数组创建 `maa-cli`。Windows 上设置独立进程组和分离标志，不继承 MCP 的 STDIO。

- [x] **任务 6.6：持久化运行记录**

  记录 PID、进程创建时间、任务文件哈希、路径和时间戳后才返回 `run_id`。记录失败时必须终止刚创建且身份匹配的进程。

- [x] **任务 6.7：覆盖竞态与失败路径**

  测试两个并发执行请求、确认期间计划改变、dry-run 后文件改变、进程创建失败和记录写入失败。

- [x] **任务 6.8：提交阶段成果**

```text
git add src/domain/run.rs src/storage/runs.rs src/maa_cli/process.rs src/tools/runs.rs tests/execution_flow.rs
git commit -m "feat: confirm and launch daily plans"
```

### 阶段验收标准

- [x] 未预览、哈希错误、缺少 elicitation 或用户拒绝时不启动进程。
- [x] 相同计划的两个并发执行请求最多一个成功。
- [x] Agent 无法在确认后追加、删除或修改任务。
- [x] MCP Server 退出后 fake maa 子进程继续运行。
- [x] 返回结果包含 `run_id`、PID、初始状态、摘要和日志位置。
- [x] 所有错误路径都不静默重试副作用。
- [x] 三条必需 Cargo 检查全部通过。

### 阶段审查记录

- 实现提交：`92d1fa4 feat: confirm and launch daily plans`
- TDD RED：先固定 Ready 状态和准确哈希要求，再覆盖原子活动锁、两个并发启动、确认期间计划变化、dry-run 后任务文件变化、进程创建失败、运行记录写入失败、缺失 elicitation 能力以及 accept/decline/cancel 三种协议结果。
- TDD GREEN：`execute_daily_plan` 只接受 `plan_id` 和 `expected_plan_hash`；以规范中文摘要和哈希请求单一布尔确认；接受后重新加载并逐项核对 state、revision、策略、摘要与哈希，随后才探测环境、生成任务并创建后台进程。
- 进程与持久化：Windows 后台进程使用参数数组、独立进程组和分离标志；MCP STDIO 句柄禁止继承；运行记录包含 PID、创建时间、任务 SHA-256、绝对路径和启动时间，记录失败时只终止身份完全匹配的进程。
- 审查修正：为 `rmcp` 启用既有 `schemars` 功能以生成 elicitation 表单；补充 Windows `SYNCHRONIZE` 访问以等待精确终止；补充最终摘要重生成比较，防止确认快照与实际执行摘要脱节。
- 自动验证：根 crate 与 fake 夹具格式检查、`cargo build --all-targets`、严格 clippy 和全量 `cargo test --all-targets --all-features` 全部通过；测试计数为 2 个单元测试、16 个环境测试、24 个计划/存储/服务测试、12 个执行流测试、4 个 stdio 协议测试和 7 个任务文件测试，共 65 项。
- 人工环境阻塞：本机仍未发现真实 `maa.exe`/`maa-cli.exe`；未启动真实游戏或账号日常，真实 MAA dry-run 和执行仍保留为阶段 8 的人工验收。
- 最终结论：自动化范围内无 Critical 或 Important 遗留问题；阶段 6 可以进入阶段 7，活动锁的重启有效性判断按任务 7.6 完成。

---

## 阶段 7：状态、受限日志、停止和重启恢复

### 阶段目标

实现剩余三个运行 Tools，并确保只查询或停止记录中身份完全匹配的进程。

### 要完成的任务

- [x] **任务 7.1：实现 Windows 进程身份**

  `platform/windows.rs` 使用 PID 加进程创建时间作为 `ProcessStartIdentity`。PID 存在但创建时间不同必须视为 PID 已复用，返回 `Unknown`。

- [x] **任务 7.2：实现 Named Job Object**

  每个运行创建可重开的命名 Job Object，并将 `maa-cli` 分配进去；不使用“句柄关闭即杀死”标志，以保证 MCP Server 退出后任务继续。停止时重新打开同名 Job 并终止其进程树。

- [x] **任务 7.3：实现 `get_run_status`**

  核对运行记录、PID 创建时间、退出码和日志证据，返回 `Running`、`Succeeded`、`Failed`、`Stopped` 或 `Unknown`。

- [x] **任务 7.4：实现 `get_run_log`**

  只允许读取已记录的 stdout/stderr 文件；每次最多返回 200 行且不超过 64 KiB；按配置模式遮盖地址、账号标识和令牌格式。

- [x] **任务 7.5：实现 `stop_run`**

  先发送优雅终止并等待 10 秒，再对身份匹配的 Named Job 强制终止。记录 `graceful` 或 `forced`，释放活动任务锁。

- [x] **任务 7.6：实现启动恢复**

  Server 启动时扫描唯一活动记录。身份匹配则保留锁；进程已退出则核对终止状态；证据冲突则标记 `Unknown` 并保留保护锁，等待人工处理。

- [x] **任务 7.7：实现七天保留策略**

  只自动清理超过七天的明确终止运行。`Running` 和 `Unknown` 永不自动删除。

- [x] **任务 7.8：协议与恢复测试**

  覆盖正常完成、非零退出、优雅停止、强制停止、PID 复用、Server 重启、日志超限、路径逃逸和脱敏。

- [x] **任务 7.9：提交阶段成果**

```text
git add src/platform src/storage/runs.rs src/storage/retention.rs src/tools/runs.rs tests/recovery.rs tests/execution_flow.rs
git commit -m "feat: recover monitor and stop maa runs"
```

### 阶段验收标准

- [x] `tools/list` 中最终出现全部十个设计 Tool。
- [x] 状态查询不会把 PID 相同但创建时间不同的进程判为当前运行。
- [x] `stop_run` 不能停止未记录进程或身份不匹配进程。
- [x] 日志读取严格遵守 200 行和 64 KiB 双重限制。
- [x] 重启后能恢复运行中任务，并在结束后释放全局锁。
- [x] 状态不明确时安全保留锁，不自动开始新任务。
- [x] 三条必需 Cargo 检查全部通过。

### 阶段审查记录

- 实现提交：`a4183e5 feat: recover monitor and stop maa runs`；审查修正：`ff6a554 fix: honor graceful stop timeout`
- TDD RED：先固定运行记录 revision/CAS 与终态保留，再覆盖 Named Job 重开终止、成功/非零退出、强制停止、PID 创建时间冲突、重启后重新监视、日志 200 行/64 KiB 双限制、路径逃逸、配置脱敏、未知目录内容保留及三个 Tool 的真实 STDIO 协议。
- TDD GREEN：运行监视持有真实进程句柄并记录退出码；Server 重启后仅对 PID 和创建时间匹配的 Running 记录恢复监视；无可靠证据时写 `Unknown` 并保留活动锁，不猜测成功或失败。
- Job 与停止：每个 `run_id` 创建可按名称重开的 Windows Job，并把 Job 句柄复制进被启动进程以跨 Server 生命周期保持对象；`stop_run` 先尝试 CTRL_BREAK，固定等待 10 秒，未退出才终止完整 Job，所有操作前必须通过进程身份校验。
- 日志与保留：日志只允许记录内的规范 stdout/stderr，按文件尾读取并限制 64 KiB 和 200 行，再应用配置字面模式脱敏；终态默认保留七天，目录含未知内容、读取失败、Running 或 Unknown 时不做自动删除。
- 审查修正：解决 Job 最后句柄关闭后名称消失、进程退出与 PID 复用语义混淆、停止与监视双写 revision 竞态、记录失败只终止主进程、以及清理先删记录导致残余数据失联等问题。
- 自动验证：根 crate 与 fake 夹具格式检查、`cargo build --all-targets`、严格 clippy 和全量 `cargo test --all-targets --all-features` 全部通过；测试计数为 2 个单元测试、16 个环境测试、24 个计划/存储/服务测试、12 个执行流测试、10 个恢复/状态/日志/停止测试、5 个 stdio 协议测试和 7 个任务文件测试，共 76 项。
- 人工验收边界：自动测试稳定覆盖 CTRL_BREAK 失败后的 Named Job 强制回退；由于 fake CLI 运行在 `DETACHED_PROCESS` 环境中，优雅 CTRL_BREAK 成功分支保留为阶段 8 的真实 Windows/MAA 人工验收，不把未验证分支宣称为已自动通过。
- 最终结论：自动化范围内无 Critical 或 Important 遗留问题；阶段 7 可以进入阶段 8。

---

## 阶段 8：双客户端验收、文档和公开发布准备

### 阶段目标

证明同一 Windows 二进制能在 Claude Code 和 Codex 中完成完整流程，并补齐独立公开项目所需文档与许可证信息。

### 要完成的任务

- [ ] **任务 8.1：完成项目身份文件**

  创建 `LICENSE`（Apache-2.0）、`NOTICE` 和 `README.md`。README 副标题使用 “Unofficial MCP adapter for MAA.”，明确声明项目与 MAA Team 无关联、未获其认可。

- [ ] **任务 8.2：编写配置文档**

  `docs/configuration.md` 说明状态目录、显式 `maa` 路径、保留期和脱敏规则，不复制 `maa-cli` 配置档。

- [ ] **任务 8.3：编写客户端配置文档**

  `docs/client-setup.md` 分别提供 Claude Code 与 Codex 的 STDIO 配置示例、Windows 路径转义、环境检查和卸载步骤。

- [ ] **任务 8.4：编写安全模型**

  `docs/security-model.md` 说明信任边界、白名单、确认哈希、单任务锁、日志限制、禁止操作及威胁模型。

- [ ] **任务 8.5：执行 fake maa 端到端测试**

  从 STDIO 初始化开始，完成：

```text
check_environment
→ get_daily_options
→ create_daily_plan
→ update_daily_plan
→ preview_daily_plan
→ execute_daily_plan
→ get_run_status
→ get_run_log
→ stop_run
```

- [ ] **任务 8.6：执行真实 MAA 人工验收**

  使用专门的测试配置档，先完成不启动游戏的 dry-run，再由用户明确确认是否执行真实游戏日常。不得在自动测试中操作真实账号。

- [ ] **任务 8.7：分别验收 Claude Code 和 Codex**

  两个客户端都必须验证：一句话请求、逐项追问、完整摘要、MCP 二次确认、状态查询、日志读取、停止和重启恢复。

- [ ] **任务 8.8：构建发布候选**

  使用 `cargo build --release --locked`，生成 Windows 二进制和 SHA-256。发布包只包含适配器二进制、校验和、许可证、NOTICE 和文档，不包含 MAA/`maa-cli`。

- [ ] **任务 8.9：提交发布准备**

```text
git add README.md LICENSE NOTICE docs src tests Cargo.toml Cargo.lock
git commit -m "docs: prepare maa-mcp-adapter MVP release"
```

### 阶段验收标准

- [ ] Claude Code 和 Codex 均能列出全部十个 Tool。
- [ ] 两个客户端均完成一次 fake maa 全流程。
- [ ] 用户明确批准后，真实 `maa-cli` 完成一次安全日常流程。
- [ ] 不完整计划、修改后旧确认、碎石、自定义任务及并发执行请求均被拒绝。
- [ ] 客户端关闭后任务继续，重启后可查询状态和日志。
- [ ] README、配置、安全、客户端安装及许可证文档齐全。
- [ ] 发布包中不存在 MAA、MaaCore、`maa-cli` 或其资源。
- [ ] `cargo fmt --check`、严格 clippy、全部测试及 release build 均成功。
- [ ] 不修改 `.github/workflows`，发布操作和推送必须另获用户明确授权。

---

## 最终需求覆盖表

| 设计要求 | 实现阶段 |
|---|---|
| Windows STDIO MCP Server | 0、5、8 |
| 探测并复用用户 `maa-cli` | 1 |
| 七类日常任务白名单 | 2、4 |
| 不碎石、有限理智药、禁止账号切换 | 2、4、6 |
| 引导式创建、更新和预览计划 | 3、5 |
| 规范中文摘要和计划哈希 | 2、6 |
| MCP elicitation 二次确认 | 6 |
| 后台执行和单活动任务锁 | 6 |
| 状态、日志和停止 | 7 |
| MCP/客户端重启恢复 | 7、8 |
| Claude Code 与 Codex 验收 | 8 |
| 独立公开发布及许可声明 | 8 |

## 全项目完成定义

只有同时满足以下条件才能声明 MVP 完成：

- [ ] 阶段 0 至阶段 8 的所有任务和验收项均已勾选。
- [ ] 不存在未完成占位符、被忽略测试或临时安全例外。
- [ ] 全部 Tool Schema、错误码和状态值已冻结并记录。
- [ ] 所有依赖均经过用户批准，`Cargo.lock` 已提交。
- [ ] 所有自动测试、严格 lint、release build 和 Windows 人工验收通过。
- [ ] 设计文档、实现计划和实际行为一致。
- [ ] 发布包内容及第三方许可证声明经过人工复核。
