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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("clang"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("next:"),
        "the failure must end with a next step:\n{stderr}"
    );
}

#[test]
fn doctor_json_reports_the_same_facts() {
    let output = Command::new(yashy())
        .arg("doctor")
        .arg("--json")
        .output()
        .expect("the yashy CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "doctor --json must succeed on a working development machine:\n{}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.starts_with('{'), "stdout:\n{stdout}");
    assert!(stdout.trim_end().ends_with('}'), "stdout:\n{stdout}");
    assert!(
        stdout.contains("\"schema_version\": 1"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("\"tool\": \"doctor\""), "stdout:\n{stdout}");
    assert!(stdout.contains("\"ok\": true"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"yashy_version\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"target\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"clang\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"stdlib\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"path\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"next\": null"), "stdout:\n{stdout}");
}

#[test]
fn doctor_json_reports_failures_without_human_output() {
    let dir = std::env::temp_dir().join(format!("yashy-doctor-json-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the fake toolchain directory must be creatable");
    let empty_stdlib = dir.join("stdlib");
    std::fs::create_dir_all(&empty_stdlib).expect("the empty stdlib directory must be creatable");

    let output = Command::new(yashy())
        .arg("doctor")
        .arg("--json")
        .env("PATH", &dir)
        .env("YASHY_STDLIB", &empty_stdlib)
        .env_remove("YASHY_LLC")
        .output()
        .expect("the yashy CLI must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "doctor must fail:\n{stdout}\n{stderr}"
    );
    assert!(
        stderr.is_empty(),
        "JSON mode must not print human errors:\n{stderr}"
    );
    assert!(stdout.contains("\"ok\": false"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"clang\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"stdlib\":"), "stdout:\n{stdout}");
    assert!(stdout.contains("\"next\":"), "stdout:\n{stdout}");
    assert!(!stdout.contains("\"next\": null"), "stdout:\n{stdout}");
}

#[test]
fn doctor_hint_matches_the_platform() {
    let dir = std::env::temp_dir().join(format!("yashy-doctor-hint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the empty PATH directory must be creatable");

    let output = Command::new(yashy())
        .arg("doctor")
        .env("PATH", &dir)
        .env_remove("YASHY_LLC")
        .output()
        .expect("the yashy CLI must be runnable");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "doctor must fail without clang:\n{stderr}"
    );
    assert!(stderr.contains("next:"), "stderr:\n{stderr}");
    if cfg!(windows) {
        assert!(stderr.contains("winget"), "stderr:\n{stderr}");
    } else if cfg!(target_os = "linux") {
        let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        if os_release.contains("ubuntu") || os_release.contains("debian") {
            assert!(stderr.contains("apt"), "stderr:\n{stderr}");
        } else if os_release.contains("fedora") || os_release.contains("rhel") {
            assert!(stderr.contains("dnf"), "stderr:\n{stderr}");
        } else if os_release.contains("arch") {
            assert!(stderr.contains("pacman"), "stderr:\n{stderr}");
        }
    }
}
