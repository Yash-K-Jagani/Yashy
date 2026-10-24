mod common;

use common::yashy;
use std::process::Command;

#[test]
fn doctor_reports_the_toolchain_and_stdlib() {
    let output = Command::new(yashy())
        .arg("doctor")
        .output()
        .expect("the yashy CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "doctor must succeed on a working development machine:\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("yashy"), "stdout:\n{stdout}");
    assert!(stdout.contains("clang"), "stdout:\n{stdout}");
    assert!(stdout.contains("stdlib"), "stdout:\n{stdout}");
}

#[test]
fn doctor_rejects_a_clang_that_cannot_run() {
    let dir = std::env::temp_dir().join(format!("yashy-doctor-bad-clang-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the fake toolchain directory must be creatable");

    let fake = dir.join(if cfg!(windows) { "clang.exe" } else { "clang" });
    std::fs::write(&fake, "this is not a runnable compiler\n")
        .expect("the fake compiler must be writable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake)
            .expect("the fake compiler must exist")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake, permissions).expect("the fake compiler must be executable");
    }

    let output = Command::new(yashy())
        .arg("doctor")
        .env("PATH", &dir)
        .env_remove("YASHY_LLC")
        .output()
        .expect("the yashy CLI must be runnable");

    assert!(
        !output.status.success(),
        "doctor must not pass a clang it cannot run:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("clang"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
