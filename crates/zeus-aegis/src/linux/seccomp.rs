//! Linux seccomp-bpf sandbox implementation
//!
//! Uses seccomp (Secure Computing Mode) with BPF filters to restrict
//! system calls available to the process.
//!
//! ## Filter Levels
//!
//! - **None**: No filtering
//! - **Basic**: Block dangerous syscalls (ptrace, kexec, etc.)
//! - **Standard**: Allowlist common syscalls for typical operation
//! - **Strict**: Minimal syscalls for network/file operations
//! - **Paranoid**: Only read/write/exit syscalls

use crate::sandbox::SandboxLevel;
use std::collections::HashSet;
use zeus_core::{Error, Result};

// Seccomp constants
const SECCOMP_MODE_FILTER: libc::c_ulong = 2;
const SECCOMP_RET_ALLOW: u32 = 0x7fff0000;
const SECCOMP_RET_KILL_PROCESS: u32 = 0x80000000;
const SECCOMP_RET_ERRNO: u32 = 0x00050000;

// BPF constants
const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

/// BPF instruction
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SockFilterInstruction {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

/// BPF program
#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilterInstruction,
}

/// Seccomp filter builder
pub struct SeccompFilter {
    level: SandboxLevel,
    allowed_syscalls: HashSet<i64>,
    blocked_syscalls: HashSet<i64>,
}

impl SeccompFilter {
    /// Create a new filter for the given sandbox level
    pub fn new(level: SandboxLevel) -> Self {
        let mut filter = Self {
            level,
            allowed_syscalls: HashSet::new(),
            blocked_syscalls: HashSet::new(),
        };

        // Initialize based on level
        filter.init_for_level();
        filter
    }

    /// Initialize syscall sets based on sandbox level
    fn init_for_level(&mut self) {
        match self.level {
            SandboxLevel::None => {
                // No filtering
            }
            SandboxLevel::Basic => {
                // Block dangerous syscalls
                self.block_dangerous_syscalls();
            }
            SandboxLevel::Standard => {
                // Allow common syscalls
                self.allow_standard_syscalls();
            }
            SandboxLevel::Strict => {
                // Minimal syscalls for operation
                self.allow_strict_syscalls();
            }
            SandboxLevel::Paranoid => {
                // Only read/write/exit
                self.allow_paranoid_syscalls();
            }
        }
    }

    /// Block dangerous syscalls (for Basic level)
    fn block_dangerous_syscalls(&mut self) {
        // Dangerous syscalls that should be blocked
        let dangerous = [
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_kexec_load,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_kexec_file_load,
            libc::SYS_reboot,
            libc::SYS_swapon,
            libc::SYS_swapoff,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_acct,
            libc::SYS_settimeofday,
            libc::SYS_clock_settime,
            // SYS_stime removed — deprecated in modern glibc, covered by clock_settime
            libc::SYS_setdomainname,
            libc::SYS_sethostname,
        ];

        for syscall in dangerous {
            self.blocked_syscalls.insert(syscall);
        }
    }

    /// Allow standard syscalls (for Standard level)
    fn allow_standard_syscalls(&mut self) {
        // Common syscalls needed for typical operation
        let standard = [
            // Process
            libc::SYS_exit,
            libc::SYS_exit_group,
            libc::SYS_getpid,
            libc::SYS_gettid,
            libc::SYS_getuid,
            libc::SYS_geteuid,
            libc::SYS_getgid,
            libc::SYS_getegid,
            // Memory
            libc::SYS_brk,
            libc::SYS_mmap,
            libc::SYS_munmap,
            libc::SYS_mprotect,
            libc::SYS_mremap,
            libc::SYS_madvise,
            // File I/O
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_readv,
            libc::SYS_writev,
            libc::SYS_pread64,
            libc::SYS_pwrite64,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_open,
            libc::SYS_openat,
            libc::SYS_close,
            libc::SYS_lseek,
            libc::SYS_fstat,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_stat,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_lstat,
            libc::SYS_newfstatat,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_access,
            libc::SYS_faccessat,
            libc::SYS_fcntl,
            libc::SYS_dup,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_dup2,
            libc::SYS_dup3,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_pipe,
            libc::SYS_pipe2,
            libc::SYS_ftruncate,
            libc::SYS_fsync,
            libc::SYS_fdatasync,
            libc::SYS_flock,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_readlink,
            libc::SYS_readlinkat,
            libc::SYS_getcwd,
            libc::SYS_chdir,
            libc::SYS_fchdir,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_renameat,
            libc::SYS_renameat2,
            libc::SYS_mkdirat,
            libc::SYS_unlinkat,
            libc::SYS_symlinkat,
            libc::SYS_linkat,
            libc::SYS_getdents64,
            // Network
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_sendto,
            libc::SYS_recvfrom,
            libc::SYS_sendmsg,
            libc::SYS_recvmsg,
            libc::SYS_shutdown,
            libc::SYS_getsockname,
            libc::SYS_getpeername,
            libc::SYS_setsockopt,
            libc::SYS_getsockopt,
            // Polling/Select
            libc::SYS_ppoll,
            libc::SYS_pselect6,
            libc::SYS_epoll_create1,
            libc::SYS_epoll_ctl,
            libc::SYS_epoll_pwait,
            // Signals
            libc::SYS_rt_sigaction,
            libc::SYS_rt_sigprocmask,
            libc::SYS_rt_sigreturn,
            libc::SYS_sigaltstack,
            // Time
            // SYS_gettimeofday removed — use clock_gettime instead
            libc::SYS_clock_gettime,
            libc::SYS_clock_getres,
            libc::SYS_nanosleep,
            // Misc
            libc::SYS_getrandom,
            libc::SYS_futex,
            libc::SYS_set_tid_address,
            libc::SYS_set_robust_list,
            libc::SYS_get_robust_list,
            libc::SYS_prctl,
            // SYS_arch_prctl is x86-only, skipped on other archs
            libc::SYS_uname,
            libc::SYS_sysinfo,
            libc::SYS_ioctl,
        ];

        for syscall in standard {
            self.allowed_syscalls.insert(syscall);
        }
    }

    /// Allow strict syscalls (for Strict level)
    fn allow_strict_syscalls(&mut self) {
        // Minimal syscalls for network/file operations
        let strict = [
            // Essential
            libc::SYS_exit,
            libc::SYS_exit_group,
            // Memory
            libc::SYS_brk,
            libc::SYS_mmap,
            libc::SYS_munmap,
            libc::SYS_mprotect,
            // File I/O
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_readv,
            libc::SYS_writev,
            libc::SYS_openat,
            libc::SYS_close,
            libc::SYS_fstat,
            libc::SYS_newfstatat,
            libc::SYS_fcntl,
            libc::SYS_lseek,
            // Network (limited)
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_sendto,
            libc::SYS_recvfrom,
            libc::SYS_shutdown,
            libc::SYS_getsockopt,
            libc::SYS_setsockopt,
            // Polling
            libc::SYS_epoll_create1,
            libc::SYS_epoll_ctl,
            libc::SYS_epoll_pwait,
            // Signals
            libc::SYS_rt_sigaction,
            libc::SYS_rt_sigprocmask,
            libc::SYS_rt_sigreturn,
            // Time
            libc::SYS_clock_gettime,
            libc::SYS_nanosleep,
            // Misc
            libc::SYS_getrandom,
            libc::SYS_futex,
        ];

        for syscall in strict {
            self.allowed_syscalls.insert(syscall);
        }
    }

    /// Allow paranoid syscalls (for Paranoid level)
    fn allow_paranoid_syscalls(&mut self) {
        // Only the absolute minimum
        let paranoid = [
            libc::SYS_exit,
            libc::SYS_exit_group,
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_close,
            libc::SYS_brk,
            libc::SYS_mmap,
            libc::SYS_munmap,
            libc::SYS_rt_sigreturn,
        ];

        for syscall in paranoid {
            self.allowed_syscalls.insert(syscall);
        }
    }

    /// Add an additional allowed syscall
    pub fn allow_syscall(&mut self, syscall: i64) -> &mut Self {
        self.allowed_syscalls.insert(syscall);
        self
    }

    /// Block a specific syscall
    pub fn block_syscall(&mut self, syscall: i64) -> &mut Self {
        self.blocked_syscalls.insert(syscall);
        self.allowed_syscalls.remove(&syscall);
        self
    }

    /// Build the BPF filter program
    fn build_filter(&self) -> Vec<SockFilterInstruction> {
        let mut filter = Vec::new();

        match self.level {
            SandboxLevel::None => {
                // Allow everything
                filter.push(Self::bpf_ret(SECCOMP_RET_ALLOW));
            }
            SandboxLevel::Basic => {
                // Block specific syscalls, allow rest
                // Load syscall number
                filter.push(Self::bpf_load_syscall_nr());

                // Check each blocked syscall
                for &syscall in &self.blocked_syscalls {
                    let remaining = self.blocked_syscalls.len() - filter.len() + 1;
                    filter.push(Self::bpf_jeq(
                        syscall as u32,
                        0,                     // jt: next instruction (block)
                        (remaining + 1) as u8, // jf: skip to allow
                    ));
                }

                // Block action (kill)
                filter.push(Self::bpf_ret(SECCOMP_RET_KILL_PROCESS));
                // Allow action
                filter.push(Self::bpf_ret(SECCOMP_RET_ALLOW));
            }
            SandboxLevel::Standard | SandboxLevel::Strict | SandboxLevel::Paranoid => {
                // Allowlist mode: only allow specific syscalls
                // Load syscall number
                filter.push(Self::bpf_load_syscall_nr());

                // Build allowlist checks
                let syscalls: Vec<i64> = self.allowed_syscalls.iter().copied().collect();
                let total = syscalls.len();

                for (i, &syscall) in syscalls.iter().enumerate() {
                    let remaining = total - i;
                    filter.push(Self::bpf_jeq(
                        syscall as u32,
                        (remaining) as u8, // jt: jump to allow
                        0,                 // jf: continue checking
                    ));
                }

                // Default: kill (syscall not in allowlist)
                filter.push(Self::bpf_ret(SECCOMP_RET_KILL_PROCESS));
                // Allow action
                filter.push(Self::bpf_ret(SECCOMP_RET_ALLOW));
            }
        }

        filter
    }

    // BPF instruction builders
    fn bpf_load_syscall_nr() -> SockFilterInstruction {
        // Load the syscall number from seccomp_data.nr
        SockFilterInstruction {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k: 0, // offsetof(struct seccomp_data, nr)
        }
    }

    fn bpf_jeq(value: u32, jt: u8, jf: u8) -> SockFilterInstruction {
        SockFilterInstruction {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt,
            jf,
            k: value,
        }
    }

    fn bpf_ret(value: u32) -> SockFilterInstruction {
        SockFilterInstruction {
            code: BPF_RET | BPF_K,
            jt: 0,
            jf: 0,
            k: value,
        }
    }
}

/// Seccomp sandbox enforcer
pub struct SeccompSandbox {
    filter: SeccompFilter,
    applied: bool,
}

impl SeccompSandbox {
    /// Create a new sandbox with the given level
    pub fn new(level: SandboxLevel) -> Self {
        Self {
            filter: SeccompFilter::new(level),
            applied: false,
        }
    }

    /// Allow an additional syscall
    pub fn allow_syscall(&mut self, syscall: i64) -> &mut Self {
        self.filter.allow_syscall(syscall);
        self
    }

    /// Block a specific syscall
    pub fn block_syscall(&mut self, syscall: i64) -> &mut Self {
        self.filter.block_syscall(syscall);
        self
    }

    /// Check if sandbox is applied
    pub fn is_applied(&self) -> bool {
        self.applied
    }

    /// Apply the seccomp filter
    ///
    /// This is a one-way operation - once applied, restrictions cannot be removed.
    pub fn apply(&mut self) -> Result<()> {
        if self.applied {
            return Err(Error::Security("Sandbox already applied".into()));
        }

        if self.filter.level == SandboxLevel::None {
            tracing::debug!("Seccomp sandbox level is None, skipping");
            self.applied = true;
            return Ok(());
        }

        // First, set no_new_privs to ensure seccomp works
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(Error::Security(format!(
                    "Failed to set no_new_privs: {}",
                    std::io::Error::last_os_error()
                )));
            }
        }

        // Build and apply the filter
        let filter = self.filter.build_filter();
        tracing::debug!(instructions = filter.len(), "Built seccomp BPF filter");

        let prog = SockFprog {
            len: filter.len() as u16,
            filter: filter.as_ptr(),
        };

        unsafe {
            if libc::prctl(
                libc::PR_SET_SECCOMP,
                SECCOMP_MODE_FILTER as libc::c_ulong,
                &prog as *const SockFprog,
                0,
                0,
            ) != 0
            {
                return Err(Error::Security(format!(
                    "Failed to apply seccomp filter: {}",
                    std::io::Error::last_os_error()
                )));
            }
        }

        self.applied = true;
        tracing::info!(level = %self.filter.level, "Seccomp sandbox applied");

        Ok(())
    }
}

/// Check if seccomp is available
pub fn is_available() -> bool {
    // Check kernel support for seccomp
    unsafe {
        let result = libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0);
        // Returns 0 if seccomp is disabled, 2 if in filter mode, -1 on error
        result >= 0
    }
}

/// Get current seccomp status
pub fn status() -> SeccompStatus {
    let mode = unsafe { libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) };

    SeccompStatus {
        available: mode >= 0,
        mode: match mode {
            0 => SeccompMode::Disabled,
            2 => SeccompMode::Filter,
            _ => SeccompMode::Unknown,
        },
    }
}

/// Seccomp mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompMode {
    /// Seccomp is disabled
    Disabled,
    /// Seccomp is in filter mode
    Filter,
    /// Unknown mode
    Unknown,
}

/// Seccomp status information
#[derive(Debug, Clone)]
pub struct SeccompStatus {
    /// Whether seccomp is available
    pub available: bool,
    /// Current seccomp mode
    pub mode: SeccompMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_creation() {
        let filter = SeccompFilter::new(SandboxLevel::Basic);
        assert!(!filter.blocked_syscalls.is_empty());
    }

    #[test]
    fn test_filter_standard() {
        let filter = SeccompFilter::new(SandboxLevel::Standard);
        assert!(filter.allowed_syscalls.contains(&libc::SYS_read));
        assert!(filter.allowed_syscalls.contains(&libc::SYS_write));
    }

    #[test]
    fn test_filter_paranoid() {
        let filter = SeccompFilter::new(SandboxLevel::Paranoid);
        assert!(filter.allowed_syscalls.len() < 20);
        assert!(filter.allowed_syscalls.contains(&libc::SYS_exit));
    }

    #[test]
    fn test_is_available() {
        // Just check it doesn't panic
        let _ = is_available();
    }

    #[test]
    fn test_status() {
        let status = status();
        // On most systems, seccomp should be available
        assert!(status.available || status.mode == SeccompMode::Unknown);
    }
}
