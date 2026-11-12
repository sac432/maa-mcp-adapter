# macOS (experimental)

Build with Rust 1.97.1 or later:

```sh
cargo build --release --locked
```

Register `target/release/maa-mcp-adapter` as a **stdio** MCP server. A client
must support MCP form elicitation to execute with the default confirmation mode.
The adapter does not provide an HTTP/SSE endpoint.

The native configuration is `~/Library/Application Support/maa-mcp-adapter/config.toml`.
`LOCALAPPDATA`, when explicitly set, overrides the application-data root (also
useful for isolated tests). An explicit `state_directory` relocates run/plan
storage, but not the configuration file itself.

Install **maa-cli** separately following its [official documentation](https://github.com/MaaAssistantArknights/maa-cli/blob/main/crates/maa-cli/docs/zh-CN/install.md).
The adapter searches PATH for `maa` then `maa-cli`. GUI hosts may have a different
PATH from Terminal; set `maa_executable` to the absolute CLI executable in that case.
An `.app` directory or the MAA GUI executable is not a substitute for maa-cli.
MaaCore, resources and a compatible CLI connection profile must be installed and
configured separately. This adapter does not bundle them.

```toml
maa_executable = '/opt/homebrew/bin/maa'
confirmation = 'elicitation'
```

MAA Mac GUI stores preferences differently from Windows `gui.json` and
`gui.new.json`. Automatic setup and derivation of saved default tasks currently
support the Windows JSON format only. Do not point `maa_gui_config` at the Mac
app, its sandbox container, or a plist; do not manufacture a saved task queue
from Mac GUI defaults. On macOS configure the CLI profile explicitly and provide
explicit tasks when creating a plan. Missing saved tasks never authorize a
fallback daily run. No access-control or app-signature changes are required.

For an already running emulator, omit `[emulator]`. The CLI profile supplies its
ADB connection. For optional adapter-managed startup, `emulator.path` must name
a real executable, not an `.app` directory or `/usr/bin/open`: launcher exit does
not establish ownership of the launched application. Configure an emulator only
when its executable/lifecycle is understood. Do not use a global shutdown command
for shared instances. An already reachable emulator is never marked as started
by the adapter.

Each CLI run starts in a fresh process group. PID, microsecond-resolution kernel
start time and group-leader identity are checked before group signals. Stop uses
SIGTERM then SIGKILL; it never enumerates processes by name. Recovered processes
must still match the recorded identity. If the leader has disappeared, control
fails closed instead of signalling a possibly reused group. Children which
explicitly leave the group cannot be controlled; macOS process groups are not
Windows Job objects. The kernel does not expose a pidfd-style atomic identity
check-and-signal API here. A recovered non-child exit has no wait status and is
reported as -1, rather than claiming a successful exit.

Tests use fake CLI binaries and isolated child processes, including rejection of
wrong identities and termination of descendants without affecting unrelated
groups. Windows Job handling remains enabled and checked separately. Real game
daily execution, Mac GUI preference import and Intel Mac runtime behavior have
not been verified by these tests.
