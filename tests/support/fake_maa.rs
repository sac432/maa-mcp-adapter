use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct FakeMaa {
    executable: PathBuf,
    args_log: PathBuf,
}

impl FakeMaa {
    pub fn build(root: &Path) -> Self {
        let fixture_manifest =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/fake_maa_fixture/Cargo.toml");
        let target_directory = root.join("fixture-target");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--offline", "--manifest-path"])
            .arg(&fixture_manifest)
            .env("CARGO_TARGET_DIR", &target_directory)
            .status()
            .expect("start Cargo for fake maa fixture");
        assert!(status.success(), "compile fake maa fixture");

        let compiled = target_directory.join("debug/fake-maa-fixture.exe");
        let executable = root.join("玛恩纳 maa & safe.exe");
        fs::copy(compiled, &executable).expect("copy fake maa executable");
        let args_log = executable.with_extension("args");

        Self {
            executable,
            args_log,
        }
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    #[allow(dead_code)]
    pub fn set_config_directory(&self, directory: &Path) {
        fs::write(
            self.executable.with_extension("config-dir"),
            directory.as_os_str().to_string_lossy().as_bytes(),
        )
        .expect("configure fake maa config directory");
    }

    pub fn set_mode(&self, mode: &str) {
        fs::write(self.executable.with_extension("mode"), mode).expect("configure fake maa mode");
    }

    #[allow(dead_code)]
    pub fn recorded_args(&self) -> Vec<String> {
        self.recorded_invocations()
            .into_iter()
            .last()
            .unwrap_or_default()
    }

    pub fn recorded_invocations(&self) -> Vec<Vec<String>> {
        let contents = fs::read_to_string(&self.args_log).expect("read fake maa argument log");
        contents
            .split('\u{1e}')
            .filter(|record| !record.is_empty())
            .map(|record| record.split('\u{1f}').map(ToOwned::to_owned).collect())
            .collect()
    }
}
