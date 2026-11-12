#![cfg(target_os = "macos")]

use maa_mcp_adapter::{
    domain::{ProcessStartIdentity, RunId},
    platform::*,
};
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};

fn sleeper() -> std::process::Child {
    Command::new("/bin/sleep")
        .arg("60")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn stop_rejects_wrong_identity_and_leaves_other_groups_alive() {
    let mut owned = sleeper();
    let mut unrelated = sleeper();
    let identity = process_identity_for_child(&owned).unwrap();
    let wrong = ProcessStartIdentity {
        creation_time_100ns: identity.creation_time_100ns + 1,
    };
    assert!(matches!(
        terminate_job(&RunId::new(), 1, owned.id(), wrong),
        Err(PlatformError::IdentityMismatch)
    ));
    assert!(owned.try_wait().unwrap().is_none());
    assign_new_job(&RunId::new(), &owned).unwrap();
    terminate_job(&RunId::new(), 1, owned.id(), identity).unwrap();
    owned.wait().unwrap();
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
}

#[test]
fn group_termination_reaches_children_without_signalling_the_test_runner() {
    let temp = tempfile::tempdir().unwrap();
    let child_pid = temp.path().join("child-pid");
    let mut child = Command::new("/bin/sh")
        .args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "test"])
        .arg(&child_pid)
        .process_group(0)
        .spawn()
        .unwrap();
    let identity = process_identity_for_child(&child).unwrap();
    for _ in 0..100 {
        if child_pid.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let descendant: u32 = std::fs::read_to_string(child_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let descendant_identity = process_identity_for_pid(descendant).unwrap();
    terminate_job(&RunId::new(), 1, child.id(), identity).unwrap();
    child.wait().unwrap();
    assert!(
        wait_for_exit(descendant, descendant_identity, Duration::from_secs(5))
            .unwrap()
            .is_some()
    );
}
