use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let command = env::args().nth(1).unwrap_or_else(|| "help".to_owned());

    let exit_code = match command.as_str() {
        "doctor" => doctor(),
        "help" | "--help" | "-h" => {
            print_help();
            0
        }
        "version" | "--version" | "-V" => {
            println!("gateflow {}", env!("CARGO_PKG_VERSION"));
            0
        }
        other => {
            eprintln!("error: unknown command: {other}");
            eprintln!();
            print_help();
            2
        }
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}

fn print_help() {
    println!("gateflow {}", env!("CARGO_PKG_VERSION"));
    println!("Real per-test Linux network sandboxing for Rust.");
    println!();
    println!("Usage:");
    println!("  gateflow doctor     Check host prerequisites without changing the host");
    println!("  gateflow version    Print the installed version");
    println!("  gateflow help       Show this help");
}

fn doctor() -> i32 {
    println!("gateflow doctor {}", env!("CARGO_PKG_VERSION"));
    println!("Read-only checks for the host running gateflow tests.");
    println!();

    let linux = cfg!(target_os = "linux");
    let procfs =
        Path::new("/proc/self/ns/user").exists() && Path::new("/proc/self/ns/net").exists();
    let mut required_ok = true;

    check("Linux target", linux, "namespace isolation is Linux-only");
    check(
        "namespace handles",
        procfs,
        "/proc/self/ns/user and /proc/self/ns/net",
    );
    required_ok &= linux && procfs;

    if let Some(value) = read_trimmed("/proc/sys/kernel/unprivileged_userns_clone") {
        let enabled = value == "1";
        check(
            "unprivileged user namespaces",
            enabled,
            &format!("kernel.unprivileged_userns_clone={value}"),
        );
        required_ok &= enabled;
    } else {
        check(
            "unprivileged user namespaces",
            true,
            "sysctl is not exposed; verify with a sandbox test",
        );
    }

    if let Some(value) = read_trimmed("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") {
        check(
            "AppArmor userns policy",
            value == "0",
            &format!("kernel.apparmor_restrict_unprivileged_userns={value}"),
        );
    } else {
        check(
            "AppArmor userns policy",
            true,
            "sysctl is not exposed; verify with a sandbox test",
        );
    }

    let cgroup_v2 = Path::new("/sys/fs/cgroup/cgroup.controllers").is_file();
    check(
        "cgroup v2",
        cgroup_v2,
        if cgroup_v2 {
            "available for opt-in resource limits"
        } else {
            "not mounted; cgroup limits will be unavailable"
        },
    );

    let seccomp = read_trimmed("/proc/sys/kernel/seccomp/actions_avail").is_some();
    check(
        "seccomp",
        seccomp,
        if seccomp {
            "kernel reports seccomp actions"
        } else {
            "kernel capability could not be confirmed"
        },
    );

    println!();
    if required_ok {
        println!("status: READY — prerequisites look compatible");
        println!("note: run a real isolated test to verify namespace netlink permissions");
        0
    } else {
        println!("status: BLOCKED — fix the required checks before running gateflow tests");
        1
    }
}

fn check(name: &str, passed: bool, detail: &str) {
    let marker = if passed { "PASS" } else { "WARN" };
    println!("[{marker}] {name}: {detail}");
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}
