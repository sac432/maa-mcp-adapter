# License review for macOS support

The repository declares Apache-2.0 in both LICENSE and Cargo.toml. NOTICE retains
Copyright 2026 sac432 and the independent, unofficial, no-endorsement statement.
No additional contribution agreement or CONTRIBUTING instructions were present
in the checked source tree. This change does not include MAA, MaaCore, maa-cli,
resources, game assets, app bundles, local emulator configuration or credentials.

The added macOS-only dependency, libc 0.2.189, is MIT OR Apache-2.0. Existing
direct dependencies use Apache-2.0, MIT, or a choice of the two. A review of all
135 packages returned by `cargo metadata` for the lockfile found MIT/Apache
choices, Unicode-3.0, the LLVM exception as an optional license alternative,
and Unlicense OR MIT. The dependency offering LGPL also offers MIT OR
Apache-2.0; no LGPL alternative needs to be selected. No conflicting mandatory
copyleft license was identified. Unicode data notices and applicable dependency
copyright/license notices must still be preserved when distributing binaries.
The code-only PR does not relicense or redistribute external MAA software.
