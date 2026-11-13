//! The sandbox: a default-deny capability set for one process launch.
//!
//! The capability set is `{ fs_read, fs_write, net, spawn, time_ms,
//! memory_mb, output_bytes }`. Anything not on the allowlist is denied by
//! default, including output past the cap. Enforcement is threefold:
//!
//! - the runtime's file entry points consult the allowlist (canonical paths,
//!   resolved at launch) before opening anything;
//! - Windows enforces memory, process count and kill-on-close with a Job
//!   Object; the POSIX hosts set rlimits for the child;
//! - time and output are measured by the driver that launches the child.
//!
//! A denied capability is `E1900` naming the capability; a breached limit is
//! `E1901` naming the limit. Both OSes or it is not a sandbox.

/// The capability set one launch runs with.
#[derive(Clone, Debug)]
pub struct CapabilitySet {
    /// Canonical paths the program may read.
    pub fs_read: Vec<std::path::PathBuf>,
    /// Canonical paths the program may write.
    pub fs_write: Vec<std::path::PathBuf>,
    pub net: bool,
    pub spawn: bool,
    pub time_ms: u64,
    pub memory_mb: u64,
    pub output_bytes: u64,
}

impl Default for CapabilitySet {
    fn default() -> CapabilitySet {
        CapabilitySet {
            fs_read: Vec::new(),
            fs_write: Vec::new(),
            net: false,
            spawn: false,
            time_ms: 5000,
            memory_mb: 256,
            output_bytes: 1024 * 1024,
        }
    }
}

/// Canonicalise a `--allow-*` root, so the allowlist check is a prefix test
/// on real paths rather than a string test on spellings. The `\\?\` verbatim
/// prefix Rust's canonicalise adds on Windows is stripped, because the
/// runtime's own canonicalisation (`GetFullPathNameA`) does not add it.
pub fn canonical_root(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let text = canonical.to_string_lossy().into_owned();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
    Ok(std::path::PathBuf::from(text))
}

/// Whether `path` is inside one of the allowlisted `roots`.
pub fn allowed(roots: &[std::path::PathBuf], path: &std::path::Path) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    roots.iter().any(|root| canonical.starts_with(root))
}

/// The environment a sandboxed child runs with: the capability set as
/// variables the runtime reads, so there is no second channel.
pub fn sandbox_env(capabilities: &CapabilitySet) -> Vec<(String, String)> {
    let join = |roots: &[std::path::PathBuf]| {
        roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(";")
    };
    vec![
        ("YASHY_SANDBOX".to_string(), "1".to_string()),
        (
            "YASHY_SANDBOX_READ".to_string(),
            join(&capabilities.fs_read),
        ),
        (
            "YASHY_SANDBOX_WRITE".to_string(),
            join(&capabilities.fs_write),
        ),
        (
            "YASHY_SANDBOX_NET".to_string(),
            if capabilities.net { "1" } else { "0" }.to_string(),
        ),
        (
            "YASHY_SANDBOX_SPAWN".to_string(),
            if capabilities.spawn { "1" } else { "0" }.to_string(),
        ),
        (
            "YASHY_SANDBOX_TIME_MS".to_string(),
            capabilities.time_ms.to_string(),
        ),
        (
            "YASHY_SANDBOX_MEMORY_MB".to_string(),
            capabilities.memory_mb.to_string(),
        ),
        (
            "YASHY_SANDBOX_OUTPUT".to_string(),
            capabilities.output_bytes.to_string(),
        ),
    ]
}

/// Windows: create the job object with the limits. The caller assigns the
/// child process to it after spawn via [`assign_child`].
#[cfg(target_os = "windows")]
pub fn windows_job(capabilities: &CapabilitySet) -> std::io::Result<usize> {
    #[repr(C)]
    struct JobObjectLimits {
        per_process_memory: usize,
        job_memory: usize,
        cpu_rate: u32,
        affinity: usize,
        flags: u32,
        min_rate: u32,
        active_processes: u32,
    }
    unsafe extern "system" {
        fn CreateJobObjectW(
            security: *const std::ffi::c_void,
            name: *const u16,
        ) -> *mut std::ffi::c_void;
        fn SetInformationJobObject(
            job: *mut std::ffi::c_void,
            info_type: u32,
            info: *const std::ffi::c_void,
            length: u32,
        ) -> i32;
    }
    const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x8;
    const JOB_OBJECT_LIMIT_JOB_MEMORY: u32 = 0x100;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let limits = JobObjectLimits {
            per_process_memory: 0,
            job_memory: capabilities.memory_mb as usize * 1024 * 1024,
            cpu_rate: 0,
            affinity: 0,
            flags: JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_JOB_MEMORY
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            min_rate: 0,
            active_processes: 64,
        };
        let ok = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &limits as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JobObjectLimits>() as u32,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job as usize)
    }
}

#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawHandle;

/// Windows: assign a spawned child to the job `windows_job` created.
#[cfg(target_os = "windows")]
pub fn assign_child(job: usize, child: &std::process::Child) -> std::io::Result<()> {
    unsafe extern "system" {
        fn AssignProcessToJobObject(
            job: *mut std::ffi::c_void,
            process: *mut std::ffi::c_void,
        ) -> i32;
    }
    let process = child.as_raw_handle() as *mut std::ffi::c_void;
    let ok = unsafe { AssignProcessToJobObject(job as *mut _, process) };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Apply to a not-yet-spawned command (Linux rlimits; on Windows use
/// [`windows_job`] + [`assign_child`] around the spawn).
pub fn apply_limits(
    command: &mut std::process::Command,
    capabilities: &CapabilitySet,
) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let _ = command;
        let _ = capabilities;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        linux_apply(command, capabilities)
    }
}

/// POSIX (Linux and Darwin): rlimits in the child before exec.
#[cfg(not(target_os = "windows"))]
pub fn linux_apply(
    command: &mut std::process::Command,
    capabilities: &CapabilitySet,
) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    #[repr(C)]
    struct Rlimit {
        cur: u64,
        max: u64,
    }
    unsafe extern "C" {
        fn setrlimit(resource: i32, new_limit: *const Rlimit) -> i32;
    }
    // The `RLIMIT_*` values are ABI, not portable: Darwin inserts `RLIMIT_AS`
    // at 5 and puts `RLIMIT_NPROC` at 7, where Linux puts them at 9 and 6.
    // Handing Darwin Linux's 9 names a resource outside its enum, `setrlimit`
    // answers `EINVAL`, the pre-exec hook fails and *every* sandboxed program
    // refuses to start. `RLIMIT_CPU` happens to be 0 on both.
    const RLIMIT_CPU: i32 = 0;
    #[cfg(target_os = "macos")]
    const RLIMIT_AS: i32 = 5;
    #[cfg(not(target_os = "macos"))]
    const RLIMIT_AS: i32 = 9;
    #[cfg(target_os = "macos")]
    const RLIMIT_NPROC: i32 = 7;
    #[cfg(not(target_os = "macos"))]
    const RLIMIT_NPROC: i32 = 6;
    let memory = capabilities.memory_mb * 1024 * 1024;
    let cpu = capabilities.time_ms / 1000 + 1;
    unsafe {
        command.pre_exec(move || {
            let apply = |resource: i32, limit: u64| -> std::io::Result<()> {
                let rlimit = Rlimit {
                    cur: limit,
                    max: limit,
                };
                if setrlimit(resource, &rlimit) != 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            };
            // The address-space limit is best effort. Darwin accepts
            // `RLIMIT_AS` but does not enforce it the way Linux does, and a
            // limit the platform ignores must not refuse to start the
            // program. The bound that holds on every OS is the runtime's
            // live-allocation cap (`YASHY_SANDBOX_MEMORY_MB`), which is what
            // `alloc_bytes` past the cap trips.
            let _ = apply(RLIMIT_AS, memory);
            apply(RLIMIT_CPU, cpu)?;
            apply(RLIMIT_NPROC, 64)?;
            Ok(())
        });
    }
    Ok(())
}
