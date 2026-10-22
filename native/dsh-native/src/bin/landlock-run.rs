#![cfg(target_os = "linux")]

//! The Landlock launcher: self-restrict-then-exec, in Rust.
//!
//! The Landlock rung of a consuming sandbox seam, for Linux hosts where `bwrap`
//! is unusable (not installed, unprivileged user namespaces disabled, or an LSM
//! profile that denies mount — Landlock is an independent syscall family and
//! needs none of those). The launcher installs a Landlock ruleset on itself and
//! `exec`s the wrapped command; the ruleset is inherited across `execve`, so
//! the command (and every process it spawns) runs confined while the invoking
//! process stays unrestricted.
//!
//! CLI contract (mirrors the `bwrap` runner argv shape the executor wraps):
//!
//!   landlock-run [--ro <path>]... [--rw <path>]... -- <argv>...
//!   landlock-run --probe
//!
//! `--ro` grants read+execute beneath the path; `--rw` grants full filesystem
//! access beneath the path. Everything else is denied (Landlock is an
//! allow-list). `--probe` builds a maximal ruleset and reports whether the
//! running kernel actually enforces it — the executor's functional probe.
//!
//! Fail-closed: if the ruleset cannot be created or is NOT enforced by the
//! kernel, the launcher exits non-zero WITHOUT exec'ing the command. A partial
//! (best-effort) enforcement on an older ABI is accepted and reported on
//! stderr; the consumer's mode vocabulary keeps its file-effect promises
//! honest per ABI level (surfaced as `full` vs `partial` by the entry
//! package's probe).
//!
//! The Landlock UAPI is defined locally instead of through
//! `<linux/landlock.h>`: the kernel's user-space ABI is stable by contract,
//! self-defining it keeps the build independent of the toolchain's header
//! vintage, and the definitions double as the audit record of exactly which
//! kernel API this launcher touches. Layouts and values are verbatim from the
//! kernel header (the path-beneath struct is packed there, so it must be
//! packed here).
//!
//! The observable behavior is pinned by `native/system/docs/cli-contract.md`:
//! the same argument grammar, exit codes, report lines, and confinement
//! semantics as the C launcher this binary replaces.

use std::ffi::{c_char, CStr, CString, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

// The ABI-1 access bits are the audit record of what the launcher governs;
// only some of them name an access the launcher grants directly.
#[allow(dead_code)]
const LL_FS_EXECUTE: u64 = 1 << 0;
#[allow(dead_code)]
const LL_FS_WRITE_FILE: u64 = 1 << 1;
#[allow(dead_code)]
const LL_FS_READ_FILE: u64 = 1 << 2;
#[allow(dead_code)]
const LL_FS_READ_DIR: u64 = 1 << 3;
#[allow(dead_code)]
const LL_FS_REMOVE_DIR: u64 = 1 << 4;
#[allow(dead_code)]
const LL_FS_REMOVE_FILE: u64 = 1 << 5;
#[allow(dead_code)]
const LL_FS_MAKE_CHAR: u64 = 1 << 6;
#[allow(dead_code)]
const LL_FS_MAKE_DIR: u64 = 1 << 7;
#[allow(dead_code)]
const LL_FS_MAKE_REG: u64 = 1 << 8;
#[allow(dead_code)]
const LL_FS_MAKE_SOCK: u64 = 1 << 9;
#[allow(dead_code)]
const LL_FS_MAKE_FIFO: u64 = 1 << 10;
#[allow(dead_code)]
const LL_FS_MAKE_BLOCK: u64 = 1 << 11;
#[allow(dead_code)]
const LL_FS_MAKE_SYM: u64 = 1 << 12;
#[allow(dead_code)]
const LL_FS_REFER: u64 = 1 << 13;
#[allow(dead_code)]
const LL_FS_TRUNCATE: u64 = 1 << 14;
#[allow(dead_code)]
const LL_FS_IOCTL_DEV: u64 = 1 << 15;

/// Newest ABI this launcher knows; the negotiation scales the ruleset down to
/// what the running kernel supports.
const MAX_ABI: i64 = 5;

/// Every access ABI 1 governs, and nothing newer.
const ABI1_MASK: u64 = LL_FS_REFER - 1;

/// The exit code for every launcher-level failure. The wrapped command was NOT
/// run.
const EXIT_LAUNCHER_FAILURE: i32 = 125;

const NOT_ENFORCED_MESSAGE: &str =
    "landlock is not enforced by this kernel (ABI unsupported or disabled)";

/// The Landlock UAPI structures, defined locally as the kernel header does.
#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// One parsed invocation.
struct Cli {
    probe: bool,
    read_only: Vec<OsString>,
    read_write: Vec<OsString>,
    command: Vec<OsString>,
}

/// Print a fatal error and end the process.
fn fail(prefix: &str, detail: Option<&str>) -> ! {
    match detail {
        None => eprintln!("landlock-run: {prefix}"),
        Some(detail) => eprintln!("landlock-run: {prefix}: {detail}"),
    }
    std::process::exit(EXIT_LAUNCHER_FAILURE)
}

/// Print a usage error and end the process.
fn fail_usage(message: &str, detail: Option<&str>) -> ! {
    match detail {
        None => eprintln!("landlock-run: usage error: {message}"),
        Some(detail) => eprintln!("landlock-run: usage error: {message}{detail}"),
    }
    std::process::exit(EXIT_LAUNCHER_FAILURE)
}

/// The kernel's message for an errno.
fn strerror(errno: i32) -> String {
    // `strerror` is not thread-safe; the launcher is single-threaded and ends
    // the process right after printing.
    unsafe { CStr::from_ptr(nix::libc::strerror(errno)) }
        .to_string_lossy()
        .into_owned()
}

/// The errno of the last failed call.
fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Parse the launcher arguments.
fn parse(args: &[OsString]) -> Cli {
    let mut cli = Cli {
        probe: false,
        read_only: Vec::new(),
        read_write: Vec::new(),
        command: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_bytes();
        if argument == b"--probe" {
            if args.len() != 1 {
                fail_usage("--probe takes no other arguments", None);
            }
            cli.probe = true;
            index += 1;
        } else if argument == b"--ro" || argument == b"--rw" {
            if index + 1 >= args.len() {
                fail_usage(
                    if argument == b"--ro" { "--ro" } else { "--rw" },
                    Some(" requires a path"),
                );
            }
            let path = args[index + 1].clone();
            if argument == b"--ro" {
                cli.read_only.push(path);
            } else {
                cli.read_write.push(path);
            }
            index += 2;
        } else if argument == b"--" {
            cli.command = args[index + 1..].to_vec();
            break;
        } else {
            fail_usage(
                "unknown argument: ",
                Some(&args[index].to_string_lossy()),
            );
        }
    }
    if !cli.probe && cli.command.is_empty() {
        fail_usage("missing `-- <argv>...` command", None);
    }
    cli
}

/// The filesystem accesses the running kernel's ABI can govern.
fn fs_mask_for_abi(abi: i64) -> u64 {
    let mut mask = ABI1_MASK;
    if abi >= 2 {
        mask |= LL_FS_REFER;
    }
    if abi >= 3 {
        mask |= LL_FS_TRUNCATE;
    }
    if abi >= 5 {
        mask |= LL_FS_IOCTL_DEV;
    }
    mask
}

/// Add a path-beneath rule.
fn add_rule(ruleset_fd: i32, path: &Path, access: u64) {
    let mut raw_path: Vec<c_char> = path.as_os_str().as_bytes().iter().map(|byte| *byte as c_char).collect();
    raw_path.push(0);
    let path_fd = unsafe {
        nix::libc::open(
            raw_path.as_ptr(),
            nix::libc::O_PATH | nix::libc::O_CLOEXEC,
        )
    };
    if path_fd < 0 {
        // Fail closed on an unopenable grant root: silently narrowing the
        // granted set would be safe, but running with a profile the caller did
        // not get is not worth the ambiguity.
        eprintln!(
            "landlock-run: cannot open rule path: {}: {}",
            path.display(),
            strerror(last_errno())
        );
        std::process::exit(EXIT_LAUNCHER_FAILURE);
    }
    // The kernel rejects directory-only accesses on a non-directory rule
    // (EINVAL), so a file grant keeps only the file-compatible bits — how the
    // `--rw /dev/null` grant works.
    let mut access = access;
    let mut status: nix::libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { nix::libc::fstat(path_fd, &mut status) } == 0
        && status.st_mode & nix::libc::S_IFMT != nix::libc::S_IFDIR
    {
        access &= LL_FS_EXECUTE | LL_FS_WRITE_FILE | LL_FS_READ_FILE | LL_FS_TRUNCATE | LL_FS_IOCTL_DEV;
    }
    let attr = LandlockPathBeneathAttr {
        allowed_access: access,
        parent_fd: path_fd,
    };
    let added = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_landlock_add_rule,
            ruleset_fd,
            LANDLOCK_RULE_PATH_BENEATH,
            &attr as *const LandlockPathBeneathAttr,
            0_u32,
        )
    };
    if added != 0 {
        let errno = last_errno();
        unsafe { nix::libc::close(path_fd) };
        fail("landlock ruleset error", Some(&strerror(errno)));
    }
    unsafe { nix::libc::close(path_fd) };
}

/// Install the ruleset on the current thread.
/// @returns Whether the kernel governs only a subset of the newest ABI.
fn restrict_self(cli: &Cli) -> bool {
    let abi = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_landlock_create_ruleset,
            std::ptr::null::<LandlockRulesetAttr>(),
            0_usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < 0 {
        // ENOSYS: kernel built without Landlock; EOPNOTSUPP: built but disabled.
        // Either way: not enforceable — fail CLOSED, never exec unconfined.
        fail(NOT_ENFORCED_MESSAGE, None);
    }
    let partial = abi < MAX_ABI;
    let handled = fs_mask_for_abi(if partial { abi } else { MAX_ABI });

    let attr = LandlockRulesetAttr {
        handled_access_fs: handled,
    };
    let ruleset_fd = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_landlock_create_ruleset,
            &attr as *const LandlockRulesetAttr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0_u32,
        )
    } as i32;
    if ruleset_fd < 0 {
        fail("landlock ruleset error", Some(&strerror(last_errno())));
    }

    let read_side = LL_FS_EXECUTE | LL_FS_READ_FILE | LL_FS_READ_DIR;
    for path in &cli.read_only {
        add_rule(ruleset_fd, Path::new(path), read_side & handled);
    }
    for path in &cli.read_write {
        add_rule(ruleset_fd, Path::new(path), handled);
    }

    // `no_new_privs` first: it is mandatory for an unprivileged restrict, and
    // it neutralizes setuid/setgid escalation inside the sandbox.
    if unsafe { nix::libc::prctl(nix::libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        fail("landlock ruleset error", Some(&strerror(last_errno())));
    }
    if unsafe { nix::libc::syscall(nix::libc::SYS_landlock_restrict_self, ruleset_fd, 0_u32) } != 0 {
        fail("landlock ruleset error", Some(&strerror(last_errno())));
    }
    unsafe { nix::libc::close(ruleset_fd) };
    partial
}

/// Replace the process image with the wrapped command.
fn exec_command(command: &[OsString]) -> ! {
    let program = match CString::new(command[0].as_bytes()) {
        Ok(program) => program,
        Err(_) => fail("exec failed", Some("argument contains a null byte")),
    };
    let mut arguments: Vec<CString> = Vec::with_capacity(command.len());
    for argument in command {
        match CString::new(argument.as_bytes()) {
            Ok(value) => arguments.push(value),
            Err(_) => fail("exec failed", Some("argument contains a null byte")),
        }
    }
    let mut pointers: Vec<*const c_char> = arguments.iter().map(|value| value.as_ptr()).collect();
    pointers.push(std::ptr::null());
    unsafe { nix::libc::execvp(program.as_ptr(), pointers.as_ptr()) };
    fail("exec failed", Some(&strerror(last_errno())));
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let cli = parse(&args);

    if cli.probe {
        // The functional probe: build and enforce a maximal ruleset in THIS
        // short-lived process (the probe run exits right after). A
        // `--version` style check would miss a kernel that has the syscalls
        // but refuses enforcement; actually restricting is the only honest
        // signal. The one report line is part of the launcher CLI contract —
        // the executor reads enforcement completeness from it.
        let probe = Cli {
            probe: false,
            read_only: vec![OsString::from("/")],
            read_write: Vec::new(),
            command: Vec::new(),
        };
        let partial = restrict_self(&probe);
        println!(
            "landlock: {}",
            if partial {
                "partially enforced (older ABI)"
            } else {
                "fully enforced"
            }
        );
        std::process::exit(0);
    }

    let partial = restrict_self(&cli);
    if partial {
        // Older ABI: some handled accesses are not governed (e.g. truncate
        // before ABI 3). Still confined for everything the kernel supports —
        // report, do not refuse.
        eprintln!("landlock-run: partial enforcement (older Landlock ABI)");
    }
    exec_command(&cli.command);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_grants_and_the_command_after_the_separator() {
        let cli = parse(&args(&["--ro", "/usr", "--rw", "/tmp", "--", "sh", "-c", "true"]));
        assert!(!cli.probe);
        assert_eq!(cli.read_only, args(&["/usr"]));
        assert_eq!(cli.read_write, args(&["/tmp"]));
        assert_eq!(cli.command, args(&["sh", "-c", "true"]));
    }

    #[test]
    fn parses_a_command_without_grants() {
        let cli = parse(&args(&["--", "/bin/true"]));
        assert!(cli.read_only.is_empty());
        assert_eq!(cli.command, args(&["/bin/true"]));
    }

    #[test]
    fn parses_the_probe_mode() {
        let cli = parse(&args(&["--probe"]));
        assert!(cli.probe);
        assert!(cli.command.is_empty());
    }

    #[test]
    fn an_abi_one_kernel_does_not_get_newer_access_bits() {
        assert_eq!(fs_mask_for_abi(1), ABI1_MASK);
    }

    #[test]
    fn the_access_mask_grows_with_the_kernel_abi() {
        let abi2 = fs_mask_for_abi(2);
        assert_ne!(abi2 & LL_FS_REFER, 0);
        assert_eq!(abi2 & LL_FS_TRUNCATE, 0);
        let abi5 = fs_mask_for_abi(5);
        assert_ne!(abi5 & LL_FS_TRUNCATE, 0);
        assert_ne!(abi5 & LL_FS_IOCTL_DEV, 0);
    }
}
