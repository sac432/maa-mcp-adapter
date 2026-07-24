use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;

fn main() {
    let executable = env::current_exe().expect("locate fake maa executable");
    let args = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut args_log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(executable.with_extension("args"))
        .expect("open fake maa argument log");
    write!(args_log, "{}\u{1e}", args.join("\u{1f}")).expect("record fake maa arguments");
    let mode = fs::read_to_string(executable.with_extension("mode"))
        .unwrap_or_else(|_| "success".to_owned());

    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["version"] if mode == "version-fail" => {
            eprintln!("secret_account_token=must-not-leak");
            std::process::exit(17);
        }
        ["version"] => {
            println!("maa-cli v2.4.0");
            println!("MaaCore v5.20.0");
        }
        ["dir", "config"] if mode == "relative-config" => {
            println!("relative-config");
        }
        ["dir", "config"] => {
            let directory = fs::read_to_string(executable.with_extension("config-dir"))
                .expect("fake maa config directory is configured");
            println!("{directory}");
        }
        ["run", "--help"] if mode == "missing-dry-run" => {
            println!("Usage: maa run <TASK> --profile <PROFILE>");
        }
        ["run", "--help"] => {
            println!("Usage: maa run <TASK> --profile <PROFILE> --dry-run");
        }
        ["--help"] => {
            println!("Usage: maa --batch --log-file <FILE> <COMMAND>");
        }
        args if args.len() == 6
            && args[0] == "--batch"
            && args[1] == "run"
            && args[3] == "--dry-run"
            && args[4] == "--profile" =>
        {
            if mode == "dry-run-fail" {
                eprintln!("simulated dry-run failure");
                std::process::exit(23);
            }
            println!("dry-run ok");
        }
        _ => println!("fake maa"),
    }
}
