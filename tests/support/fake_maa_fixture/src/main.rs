use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

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
        args if args.len() >= 6
            && args[0] == "--batch"
            && args[1] == "run"
            && args[3] == "--dry-run"
            && args[4] == "--profile" =>
        {
            if mode == "dry-run-fail" {
                eprintln!("simulated dry-run failure");
                std::process::exit(23);
            }
            if mode == "tamper-task" {
                fs::write(args[2], "tampered = true").expect("tamper checked task file");
            }
            if mode == "block-record"
                && Path::new(&args[2])
                    .components()
                    .any(|part| part.as_os_str() == "runs")
            {
                let record_path = Path::new(&args[2])
                    .parent()
                    .expect("task run directory")
                    .join("record.json");
                fs::create_dir(&record_path).expect("block run record destination");
            }
            println!("dry-run ok");
        }
        ["--batch", "run", _, "--profile", _, "-v"] if mode == "block-record" => {
            fs::write(
                executable.with_extension("pid"),
                std::process::id().to_string(),
            )
            .expect("record fake pid");
            println!("blocking run record");
            std::io::stdout().flush().expect("flush startup output");
            std::thread::sleep(Duration::from_secs(60));
        }
        ["--batch", "run", _, "--profile", _, "-v"] if mode == "background" => {
            println!("background run started");
            eprintln!("[fake] INFO connected and running daily tasks");
            std::io::stdout().flush().expect("flush startup output");
            std::thread::sleep(Duration::from_secs(60));
        }
        ["--batch", "run", _, "--profile", _, "-v"] if mode == "actual-fail" => {
            eprintln!("simulated actual execution failure");
            std::process::exit(29);
        }
        _ => println!("fake maa"),
    }
}
