# 0.1.0 发布验收清单

## 已自动完成

- [x] `cargo fmt --all -- --check`
- [x] fake CLI crate 格式检查
- [x] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] `cargo test --all-targets --all-features`：78 项通过
- [x] fake MAA 完整 STDIO 流程：
  `check_environment → get_daily_options → create → update → preview → execute → status → log → stop`
- [x] elicitation accept、decline、cancel 和 capability 缺失
- [x] 计划修改、旧哈希、任务文件修改和并发执行拒绝
- [x] PID 创建时间、Named Job、Server 重启恢复和活动锁
- [x] 日志 200 行/64 KiB、脱敏和路径逃逸
- [x] 七天终态保留和未知内容保护
- [x] `cargo build --release --locked`
- [x] release EXE 独立 STDIO 初始化并列出十个 Tools
- [x] 发布候选仅包含适配器、校验和、LICENSE、NOTICE、README 和文档
- [x] 未修改 `.github/workflows`
- [x] 未推送或创建 Release

## 发布前必须人工完成

- [x] 在真实 `maa-cli v0.7.5` / MaaCore `v6.14.2` 上执行不启动游戏的 capability probe 和 dry-run。
- [ ] 用户明确批准后，使用专用测试配置档完成一次真实安全日常。
- [ ] 验证真实环境中的 CTRL_BREAK 优雅停止；失败时确认 Named Job 强制回退。
- [x] Claude Code：加载十个 Tools，完成 fake MAA 全流程和 form elicitation。
- [ ] Codex：已加载十个 Tools 并完成规划与预览；本机 CLI `0.111.0` 未声明 form elicitation 能力，尚未完成执行。
- [ ] 两个客户端分别验证：关闭客户端后任务继续，重启后状态和日志可恢复。
- [ ] 人工审阅 README、NOTICE、商标声明和发布说明。
- [ ] 明确授权后再推送分支、创建 PR/Tag/GitHub Release。

## 发布候选

文件名：

```text
maa-mcp-adapter-v0.1.0-windows-x86_64.zip
```

ZIP 必须包含：

```text
maa-mcp-adapter.exe
maa-mcp-adapter.exe.sha256
LICENSE
NOTICE
README.md
docs/configuration.md
docs/client-setup.md
docs/security-model.md
```

不得包含 MAA、MaaCore、`maa-cli`、模拟器配置、账号数据或测试日志。
