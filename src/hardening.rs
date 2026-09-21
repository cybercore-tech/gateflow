//! Optional resource and syscall hardening for a sandboxed test.
//!
//! These controls are deliberately opt-in. A normal [`crate::Sandbox`] keeps
//! the existing namespace/network behavior; callers that need stronger
//! containment can add cgroup v2 limits and a seccomp-BPF profile explicitly.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nix::libc;
use nix::unistd::Pid;

use crate::Error;

static NEXT_CGROUP_ID: AtomicU64 = AtomicU64::new(0);

/// Resource limits applied through a cgroup v2 sandbox.
///
/// The default cgroup root is `/sys/fs/cgroup`. Rootless callers should use
/// [`CgroupLimits::root`] to point at a cgroup delegated to their user, such
/// as a systemd user-slice subtree. The parent process creates and configures
/// the leaf cgroup; the forked sandbox child is added before it is released
/// to enter its network namespace.
#[derive(Debug, Clone)]
pub struct CgroupLimits {
    root: PathBuf,
    memory_max: Option<u64>,
    pids_max: Option<u64>,
    cpu_max: Option<CpuMax>,
}

impl Default for CgroupLimits {
    fn default() -> Self {
        Self::new()
    }
}

impl CgroupLimits {
    /// Starts an empty cgroup v2 limit set under `/sys/fs/cgroup`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("/sys/fs/cgroup"),
            memory_max: None,
            pids_max: None,
            cpu_max: None,
        }
    }

    /// Uses a caller-provided delegated cgroup v2 directory as the parent.
    #[must_use]
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = root.into();
        self
    }

    /// Limits the sandbox's memory usage in bytes.
    #[must_use]
    pub fn memory_max(mut self, bytes: u64) -> Self {
        self.memory_max = Some(bytes);
        self
    }

    /// Limits the number of processes/threads in the sandbox.
    #[must_use]
    pub fn pids_max(mut self, count: u64) -> Self {
        self.pids_max = Some(count);
        self
    }

    /// Limits CPU time to `quota_us` per `period_us`.
    #[must_use]
    pub fn cpu_max(mut self, quota_us: u64, period_us: u64) -> Self {
        self.cpu_max = Some(CpuMax {
            quota_us,
            period_us,
        });
        self
    }

    pub(crate) fn create(self) -> Result<CgroupGuard, Error> {
        if self.cpu_max.is_some_and(|cpu| cpu.period_us == 0) {
            return Err(Error::Cgroup(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cgroup cpu period must be greater than zero",
            )));
        }

        let controllers = self.root.join("cgroup.controllers");
        if !controllers.is_file() {
            return Err(Error::Cgroup(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not a writable cgroup v2 root", self.root.display()),
            )));
        }

        let id = NEXT_CGROUP_ID.fetch_add(1, Ordering::Relaxed);
        let path = self
            .root
            .join(format!("gateflow-{}-{id}", std::process::id()));
        fs::create_dir(&path).map_err(Error::Cgroup)?;

        let result = (|| {
            if let Some(bytes) = self.memory_max {
                write_value(&path, "memory.max", bytes.to_string())?;
            }
            if let Some(count) = self.pids_max {
                write_value(&path, "pids.max", count.to_string())?;
            }
            if let Some(cpu) = self.cpu_max {
                write_value(
                    &path,
                    "cpu.max",
                    format!("{} {}", cpu.quota_us, cpu.period_us),
                )?;
            }
            Ok::<(), Error>(())
        })();

        if let Err(error) = result {
            let _ = fs::remove_dir(&path);
            return Err(error);
        }

        Ok(CgroupGuard { path })
    }
}

/// CPU quota parameters for cgroup v2's `cpu.max` file.
#[derive(Debug, Clone, Copy)]
struct CpuMax {
    quota_us: u64,
    period_us: u64,
}

fn write_value(path: &Path, file: &str, value: String) -> Result<(), Error> {
    fs::write(path.join(file), value).map_err(Error::Cgroup)
}

/// A configured cgroup v2 leaf owned by one sandbox invocation.
pub(crate) struct CgroupGuard {
    path: PathBuf,
}

impl CgroupGuard {
    pub(crate) fn add_pid(&self, pid: Pid) -> Result<(), Error> {
        write_value(&self.path, "cgroup.procs", pid.as_raw().to_string())
    }
}

impl Drop for CgroupGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

/// Built-in seccomp profile that denies namespace changes and selected
/// kernel-control operations after sandbox setup is complete.
///
/// This is a defense-in-depth profile, not a complete syscall allow-list.
/// It preserves ordinary test behavior, including threads, files, sockets,
/// and subprocesses without namespace flags. It blocks `unshare`, `setns`,
/// mount operations, `ptrace`, `bpf`, `reboot`, and namespace-bearing
/// `clone` calls. The filter is irreversible for the child process.
#[derive(Debug, Clone, Copy, Default)]
pub struct SeccompProfile;

impl SeccompProfile {
    /// Returns the built-in deny profile.
    #[must_use]
    pub fn deny_namespace_changes() -> Self {
        Self
    }
}

pub(crate) fn install_seccomp(_profile: SeccompProfile) -> Result<(), Error> {
    let mut filter = vec![stmt(BPF_LD | BPF_W | BPF_ABS, 0)];

    for syscall in [
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_ptrace,
        libc::SYS_bpf,
        libc::SYS_reboot,
    ] {
        filter.push(jump(BPF_JMP | BPF_JEQ | BPF_K, syscall as u32, 0, 1));
        filter.push(stmt(
            BPF_RET | BPF_K,
            SECCOMP_RET_ERRNO | libc::EPERM as u32,
        ));
    }

    // clone3 has a pointer argument, so inspecting its namespace flags would
    // require dereferencing untrusted memory in the filter. Deny it outright;
    // ordinary Rust threads use clone and remain supported below.
    filter.push(jump(
        BPF_JMP | BPF_JEQ | BPF_K,
        libc::SYS_clone3 as u32,
        0,
        1,
    ));
    filter.push(stmt(
        BPF_RET | BPF_K,
        SECCOMP_RET_ERRNO | libc::EPERM as u32,
    ));

    // For clone(), deny only namespace-bearing flags and allow normal thread
    // or subprocess creation. The first argument is at seccomp_data offset 16.
    filter.push(jump(
        BPF_JMP | BPF_JEQ | BPF_K,
        libc::SYS_clone as u32,
        0,
        3,
    ));
    filter.push(stmt(BPF_LD | BPF_W | BPF_ABS, 16));
    filter.push(jump(BPF_JMP | BPF_JSET | BPF_K, CLONE_NEW_MASK, 0, 1));
    filter.push(stmt(
        BPF_RET | BPF_K,
        SECCOMP_RET_ERRNO | libc::EPERM as u32,
    ));
    filter.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));

    let mut program = SockFprog {
        len: filter.len() as u16,
        filter: filter.as_mut_ptr(),
    };

    // SAFETY: these calls only install process-local kernel policy. The BPF
    // program and its backing vector remain alive for the duration of the
    // syscall, and no raw pointer escapes this function.
    let no_new_privs = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if no_new_privs != 0 {
        return Err(Error::Seccomp(io::Error::last_os_error()));
    }

    // SAFETY: `program` points to a valid classic-BPF instruction array for
    // the duration of this syscall; the kernel copies it before returning.
    let result = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            0,
            &mut program as *mut SockFprog,
        )
    };
    if result != 0 {
        return Err(Error::Seccomp(io::Error::last_os_error()));
    }

    Ok(())
}

#[repr(C)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *mut SockFilter,
}

const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_JSET: u16 = 0x40;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const CLONE_NEW_MASK: u32 = 0x7e02_0000;

fn stmt(code: u16, k: u32) -> SockFilter {
    SockFilter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

fn jump(code: u16, k: u32, jt: u8, jf: u8) -> SockFilter {
    SockFilter { code, jt, jf, k }
}

#[cfg(test)]
mod tests {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::{ForkResult, fork};

    use super::*;

    #[test]
    fn rejects_zero_cpu_period_before_touching_the_cgroup_root() {
        let result = CgroupLimits::new().cpu_max(1, 0).create();
        assert!(
            matches!(result, Err(Error::Cgroup(source)) if source.kind() == io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn seccomp_profile_blocks_namespace_changes_in_child() {
        // Keep the irreversible filter in a forked child so this test process
        // and the cargo test harness remain unaffected.
        let status = match unsafe { fork() }.expect("fork for seccomp test") {
            ForkResult::Child => {
                if install_seccomp(SeccompProfile::deny_namespace_changes()).is_err() {
                    std::process::exit(2);
                }

                let result = nix::sched::unshare(nix::sched::CloneFlags::CLONE_NEWUTS);
                std::process::exit(if result == Err(nix::Error::EPERM) {
                    0
                } else {
                    1
                });
            }
            ForkResult::Parent { child } => waitpid(child, None).expect("wait for seccomp test"),
        };

        assert!(matches!(status, WaitStatus::Exited(_, 0)));
    }
}
