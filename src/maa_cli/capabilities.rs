use super::MaaCliCapabilities;

pub fn from_help(run_help: &str, global_help: &str) -> MaaCliCapabilities {
    MaaCliCapabilities {
        batch: has_option(global_help, "--batch"),
        profile: has_option(run_help, "--profile"),
        dry_run: has_option(run_help, "--dry-run"),
        absolute_task_path: false,
        log_file: has_option(global_help, "--log-file"),
    }
}

fn has_option(help: &str, expected: &str) -> bool {
    help.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(character, ',' | ';' | ':' | '[' | ']' | '(' | ')')
        });
        token == expected || token.starts_with(&format!("{expected}="))
    })
}

#[cfg(test)]
mod tests {
    use super::from_help;

    #[test]
    fn similar_option_names_do_not_count_as_capabilities() {
        let capabilities = from_help(
            "run --profiles --not-dry-run",
            "maa --batch-mode --log-files",
        );

        assert!(!capabilities.batch);
        assert!(!capabilities.profile);
        assert!(!capabilities.dry_run);
        assert!(!capabilities.log_file);
    }
}
