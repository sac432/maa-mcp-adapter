# MAA MCP Adapter MVP Design

Date: 2026-07-24

## Summary

`maa-mcp-adapter` is an unofficial, independently released MCP server that lets
an AI agent guide a user through planning and safely running common MAA daily
tasks.

The user should be able to begin with a short request such as:

> 帮我启动游戏刷日常。

The agent then asks only for information that is not already explicit or safely
available from the user's existing `maa-cli` configuration. Before any MAA task
starts, the agent presents a complete execution summary and the MCP server asks
for a second, plan-bound confirmation.

The MVP targets Claude Code and Codex over STDIO. Its code remains portable, but
only Windows is a supported and tested platform for the first release.

## Goals

- Convert a short natural-language daily-task request into a validated MAA task
  plan through a guided conversation.
- Reuse the user's existing `maa-cli` installation, profiles, device
  connection, client type, and MaaCore resources.
- Expose a small, structured MCP tool surface instead of arbitrary commands or
  arbitrary MAA task configuration.
- Prevent premium-currency spending, account switching, forced credit-shop
  purchases, and unsupported task types by deterministic server-side policy.
- Require an inspectable plan summary and explicit confirmation before launch.
- Run long MAA tasks in the background, preserve their state across MCP client
  restarts, and support status, log, and stop operations.
- Be suitable for publication in an independent GitHub repository.

## Non-goals

- Bundling, downloading, installing, updating, or modifying MAA, MaaCore, or
  `maa-cli`.
- Controlling the MAA graphical interface.
- Accepting arbitrary shell commands, CLI arguments, JSON, TOML, scripts, or
  custom MaaCore task types from the agent.
- Supporting Roguelike, Copilot, SSS Copilot, Reclamation, Custom, SingleStep,
  VideoRecognition, or other non-daily task families in the MVP.
- Running more than one MAA task at a time.
- Providing a remote HTTP MCP service, multi-user hosting, or authentication.
- Maintaining a second user-preference database that duplicates `maa-cli`
  profiles.
- Claiming affiliation with or endorsement by MAA Team.

## Architecture

```text
Claude Code / Codex
        |
        | STDIO MCP
        v
maa-mcp-adapter executable
        |
        | validated executable + argument array
        v
user-installed maa-cli
        |
        v
MaaCore -> ADB -> emulator -> Arknights
```

The adapter is a standalone Rust executable built with the official `rmcp` SDK.
It locates `maa` on `PATH` by default and supports an explicit executable-path
override. It never launches `maa` through a shell.

The adapter and `maa-cli` remain separate programs. The adapter writes a
validated generated task file to its own state directory and invokes the
documented `maa-cli` command needed to run that file in batch mode.

Exact `maa-cli` invocation syntax is isolated behind a `MaaCliAdapter`
interface. The implementation probes the installed executable's capabilities
and fails closed when the detected command surface does not match a supported
adapter implementation. It does not guess based solely on an unrecognized
version string.

## Component boundaries

### MCP transport

- Runs over STDIO.
- Registers server instructions that require the agent to collect missing
  answers, preview the plan, obtain user agreement, and only then call the
  execution tool.
- Exposes structured input and output schemas for every tool.
- Marks environment, option, preview, status, and log tools as read-only.
- Marks execution and stop tools with accurate MCP risk annotations.

### Environment inspector

- Locates the `maa` executable.
- Reads `maa-cli` version and supported command capabilities.
- Discovers available profiles without modifying them.
- Reports configuration problems in user-facing terms.
- Never reads or returns secrets that are not necessary for task planning.

### Daily-plan service

- Owns the plan state machine and validation policy.
- Merges explicit user choices with safe values obtained from an existing
  `maa-cli` profile.
- Returns missing fields and legal choices in structured form so the agent can
  ask focused questions.
- Produces the canonical Chinese execution summary.
- Computes the canonical plan hash used by confirmation and execution.

### Safety-policy service

- Defines the MVP task allowlist.
- Rejects prohibited task types and parameters before any task file is written.
- Revalidates the complete plan immediately before process creation.
- Contains no model-generated rules and does not rely on prompts for safety.

### Maa CLI adapter

- Converts a validated domain plan into the minimal Maa task configuration.
- Starts `maa` with a direct executable path and an argument array.
- Captures stdout and stderr to a log file.
- Translates exit codes and known failures into stable adapter errors.
- Provides graceful and forced stop operations through a platform abstraction.

### Run registry

- Enforces the global single-active-run lock.
- Persists plan summary, plan hash, PID, process start identity, generated
  configuration path, log path, timestamps, and terminal status.
- Recovers status after the MCP process restarts.
- Atomically updates state so a partial write cannot become a valid run record.

## Supported daily-task scope

The MVP may compose the following MAA task families:

- `StartUp`: connect to the configured device and optionally start the game.
- `Fight`: main story, resource stages, current activities, and annihilation.
- `Recruit`: public recruitment with all consumable choices explicitly planned.
- `Infrast`: infrastructure shift operations using the existing profile.
- `Mall`: friend visits and credit-store operations under the restrictions
  below.
- `Award`: daily/weekly, mail, and supported event reward collection.
- `CloseDown`: optional game shutdown when explicitly included in the confirmed
  plan.

The adapter also supports environment inspection, plan cancellation, current
run status, bounded log reads, and stopping the current run.

## Safety policy

The following rules are mandatory and cannot be weakened by the agent:

- `Fight.stone` is always `0`.
- Account switching is unavailable.
- Premium-currency or paid purchasing is unavailable.
- Arbitrary Maa task types and arbitrary configuration fragments are
  unavailable.
- `Mall.force_shopping_if_credit_full` is always `false`.
- Credit-store shopping must preserve the user's configured blacklist.
- A task plan must contain an explicit stage or a validated supported semantic
  selection when a fight is requested.
- Fight count and medicine usage must be explicit before a plan becomes ready.
- Medicine is an integer in the range `0..=10` for the MVP.
- Recruit settings that consume expedited plans or confirm recruitments must be
  explicit and included in the summary.
- A new run cannot start while another active run exists.
- A modified plan invalidates every previous confirmation.
- Process termination targets only a process whose PID and start identity match
  the active run record.

Existing `maa-cli` settings may supply:

- Profile name.
- Device connection and emulator address.
- Client type.
- Resource location and platform resource selection.
- Existing infrastructure plan and ordinary task defaults that do not consume
  limited resources or weaken an MVP safety rule.

The agent must ask about:

- Daily-task categories when the user's intent is ambiguous.
- Fight category, stage, and count.
- Medicine usage and exact maximum amount.
- Recruitment actions that consume permits or expedited plans.
- Credit-store shopping choices when the existing profile does not resolve
  them.
- Optional mail/event reward collection and optional game shutdown when they
  are not explicit.

## Plan state machine

```text
Draft -> Ready -> Confirmed -> Running -> Succeeded
                                      -> Failed
                                      -> Stopped

Draft/Ready -> Cancelled
Confirmed   -> Ready, when any field changes
```

- `Draft`: one or more required fields are missing or invalid.
- `Ready`: the plan is complete, valid, and has a canonical preview.
- `Confirmed`: the user accepted the exact canonical preview through MCP
  elicitation.
- `Running`: a matching `maa-cli` process was created and recorded.
- Terminal states record the final result and release the active-run lock.

Confirmation is bound to the canonical plan hash. Execution performs a final
validation and recomputes the hash. A mismatch returns the plan to `Ready`
without starting a process.

## MCP tool surface

### `check_environment`

Read-only. Returns:

- Whether `maa` was found and how.
- Version and capability probe results.
- Available profile names.
- Whether the adapter can safely create a plan and run a task.
- Actionable diagnostics without secret values.

### `get_daily_options`

Read-only. Accepts an optional profile and partial intent. Returns:

- Daily-task categories.
- Supported fight categories.
- Available choices that can be determined from local installed resources.
- Fields the agent will need to collect.
- Fixed safety restrictions.

The MVP does not fetch untrusted web content to populate choices. If installed
resources cannot enumerate a current event stage, the agent asks the user for a
stage identifier and the adapter validates its syntax and lets `maa-cli`
perform the authoritative runtime check.

### `create_daily_plan`

Creates a local draft from structured daily-task choices. It never runs MAA.
Returns a `plan_id`, the normalized plan, missing fields, validation errors, and
the next recommended question.

### `update_daily_plan`

Applies structured answers to an existing draft. It rejects unknown fields and
prohibited parameters. Any change invalidates an existing confirmation.

### `preview_daily_plan`

Read-only with respect to external systems. Validates the plan and returns:

- Canonical Chinese summary.
- Per-task operations in execution order.
- Limited-resource usage.
- Fixed safety values such as `stone = 0`.
- Profile and device target.
- Canonical plan hash.

It only transitions a complete `Draft` to `Ready`.

### `execute_daily_plan`

Side-effecting. Requirements:

- Plan is `Ready`.
- No active run exists.
- Environment and `maa-cli` capabilities still pass.
- MCP elicitation presents the canonical summary and receives explicit user
  acceptance.
- Plan hash remains unchanged after elicitation.

On success it starts the background process and returns `run_id`, initial
status, PID, summary, and log location.

### `get_run_status`

Read-only. Reconciles the persisted record with the process identity and
returns `Running`, `Succeeded`, `Failed`, `Stopped`, or `Unknown`, with
timestamps and a concise result.

### `get_run_log`

Read-only. Returns a bounded tail of the selected run log. It enforces a maximum
byte/line count, redacts configured sensitive patterns, and never loads an
unbounded log into model context.

### `stop_run`

Protective side effect. It verifies process identity, requests graceful
termination, waits for a bounded interval, and then terminates the recorded
process tree if needed. It records whether the stop was graceful or forced.

### `cancel_daily_plan`

Cancels an unstarted plan. It cannot affect a running task.

## Example interaction

```text
User:
帮我启动游戏刷日常。

Agent:
今天要刷哪一类？
1. 主线
2. 当前活动
3. 剿灭
4. 资源关

User:
活动。

Agent:
刷哪个活动关卡、几次？是否使用理智药？

User:
XX-8，刷 5 次，不吃药。基建、公招、信用和奖励也做。

Agent:
将使用现有 default profile：
- 启动已配置客户端
- 活动 XX-8，5 次
- 不使用理智药，不使用源石
- 执行公招（按已确认的招募参数）
- 执行基建换班
- 访问好友并按现有黑名单购买信用商店物品
- 领取日常/周常奖励
- 不切换账号，不关闭游戏

是否同意执行以上计划？

User:
同意。

Agent:
[calls execute_daily_plan; MCP server presents the same summary in elicitation]
```

## Background execution and recovery

The server starts a detached `maa-cli` process with stdout and stderr redirected
to files owned by the adapter. The task continues if Claude Code, Codex, or the
STDIO MCP server exits.

On restart, the adapter:

1. Loads the active-run record.
2. Checks that PID and process start identity still match.
3. Reconciles exit/log evidence with the stored status.
4. Keeps the single-run lock only while the matching process is active.
5. Marks ambiguous state as `Unknown` and requires manual inspection instead of
   guessing.

On Windows, process creation, identity checks, and process-tree termination are
implemented behind a Windows-specific module. Portable interfaces allow later
Linux and macOS implementations without changing MCP tools or plan semantics.

## Local storage

The adapter uses the platform application-data directory, with a configurable
override for tests and advanced users. It stores:

```text
config.toml
plans/<plan_id>.json
runs/<run_id>/record.json
runs/<run_id>/task.toml
runs/<run_id>/stdout.log
runs/<run_id>/stderr.log
locks/active-run.lock
```

`config.toml` contains only adapter settings such as an explicit `maa`
executable path, state-directory override, retention duration, and redaction
patterns. It does not duplicate `maa-cli` profiles.

Plans and run records use atomic replacement. Completed run data is retained
for seven days by default and then removed during safe startup housekeeping.
Active and ambiguous records are never removed automatically.

## Error handling

The adapter fails closed when:

- `maa` is missing or its executable path is invalid.
- Capability probing fails or returns an unsupported command surface.
- The selected profile is unavailable.
- Required plan fields are missing.
- A plan includes an unknown or prohibited value.
- The plan hash changes after preview or confirmation.
- Another run is active or its state is ambiguous.
- The generated task configuration cannot be written atomically.
- Process identity cannot be verified for status or stop operations.

Errors are stable structured values with:

- Machine-readable code.
- Short user-facing Chinese message.
- Relevant safe details.
- Recommended recovery action.
- A bounded, redacted log tail when applicable.

No error path silently retries a side-effecting operation. Retrying requires an
explicit new tool call, and execution remains protected by the existing
confirmed plan hash.

## Security and privacy

- Never invoke a shell.
- Treat all model-supplied fields as untrusted input.
- Canonicalize and validate executable, profile, state, and generated-file
  paths.
- Keep generated files inside the adapter state directory.
- Do not log environment variables, authorization data, account identifiers,
  or full command environments.
- Bound every log response returned through MCP.
- Mark external MAA/`maa-cli` output as untrusted data, not agent instructions.
- Keep final safety checks in deterministic Rust code.

## Testing and verification

### Unit tests

- Plan-state transitions.
- Required-field discovery.
- Allowlist and prohibited-field enforcement.
- Medicine and count bounds.
- Summary canonicalization.
- Plan hashing and confirmation invalidation.
- Path and redaction behavior.
- Run-lock transitions.

### Integration tests

A fake `maa` executable exercises:

- Capability probing.
- Successful process launch and completion.
- Non-zero exit and stderr capture.
- Long-running background status.
- Graceful stop and forced process-tree stop.
- Server restart and run recovery.
- Executable and state paths containing spaces and Chinese characters.
- Concurrent execution rejection.

### MCP protocol tests

- STDIO initialization and capability negotiation.
- Tool schemas and risk annotations.
- Read-only versus side-effecting tool behavior.
- Elicitation acceptance, rejection, and cancellation.
- Structured error results and bounded log output.

### Required local checks

Every implementation change must pass:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Tests must not be deleted or skipped to obtain a passing result.

### Manual acceptance tests

On Windows:

1. Configure the built executable in Claude Code over STDIO.
2. Complete one full “帮我启动游戏刷日常” planning and confirmation flow.
3. Repeat the same flow in Codex.
4. Confirm an incomplete plan cannot execute.
5. Confirm a modified plan requires fresh confirmation.
6. Confirm a prohibited stone or custom-task request is rejected.
7. Confirm the task survives MCP/client shutdown.
8. Confirm a restarted client can query status and logs.
9. Confirm stop targets only the recorded process tree.

## Distribution and project identity

- Independent public GitHub repository.
- Working repository name: `maa-mcp-adapter`.
- Own project icon; no MAA Logo.
- README subtitle: “Unofficial MCP adapter for MAA.”
- README states that the project is not affiliated with or endorsed by MAA
  Team.
- README links to the MAA repository, `maa-cli`, their licenses, and the MAA
  user agreement.
- The adapter source is licensed under Apache-2.0.
- MAA and `maa-cli` remain separately installed AGPL-3.0 programs.
- MVP releases contain only adapter binaries, checksums, license, notices, and
  documentation.

This separation is an engineering and distribution decision, not legal advice.
Bundling or linking MAA/`maa-cli` code in the future requires a new licensing
review.

## Alternatives considered

### Direct MaaCore integration

Rejected for the MVP because it couples the adapter to MaaCore loading,
callbacks, resources, and platform-specific library distribution. It also
increases licensing and packaging complexity.

### TypeScript MCP server

Viable and supported by a Tier 1 official SDK, but it would require Node.js or
an additional bundling strategy on end-user Windows machines.

### Python FastMCP server

Fast to prototype and supported by a Tier 1 official SDK, but it would require
Python/uv or a separate Windows executable-packaging pipeline.

### Rust MCP server

Selected because it produces a standalone executable, matches the implementation
language of `maa-cli`, and provides suitable process, locking, and persistence
control. The MVP needs only the Rust SDK's stable STDIO, tool, schema, and
elicitation capabilities; it does not depend on experimental MCP task support.

## MVP acceptance criteria

The MVP is complete when:

- A Windows user with a configured `maa-cli` can register one executable in
  Claude Code and Codex.
- The agent can turn a one-sentence daily request into a complete plan by asking
  only unresolved questions.
- The server produces a deterministic Chinese summary and requires plan-bound
  confirmation before launch.
- Every prohibited operation is rejected by server-side validation.
- A supported daily plan launches through `maa-cli`, returns a `run_id`, and can
  be queried, logged, and stopped.
- A running task survives MCP client shutdown and is reconciled after restart.
- The global single-run lock prevents emulator-control conflicts.
- All required formatting, linting, unit, integration, MCP, and Windows
  acceptance checks pass.
