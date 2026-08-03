//! Smoke tests for pikr.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pikr")
}

#[test]
fn prints_version() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("pikr "), "got: {stdout}");
}

#[test]
fn prints_help() {
    let out = Command::new(bin()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("--show"));
    assert!(stdout.contains("--dmenu"));
}

/// Without `WAYLAND_DISPLAY`, pikr must exit non-zero and print the
/// "WAYLAND_DISPLAY is not set" guard message. The full live-render path
/// is exercised by the e2e harness (`tests/e2e/`), which runs pikr inside
/// a `sway --headless` fixture so dev machines don't see a stray window
/// pop on every `cargo test` invocation.
#[test]
fn missing_wayland_display_exits_with_guard() {
    let out = Command::new(bin())
        .args(["--show", "drun"])
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("spawn pikr");

    assert!(
        !out.status.success(),
        "pikr without WAYLAND_DISPLAY must exit non-zero; got {}",
        out.status
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("WAYLAND_DISPLAY"),
        "expected guard message, got stderr: {err}"
    );
}

#[cfg(target_os = "linux")]
mod startup_readiness_script {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    enum WtypeBehavior {
        Succeed,
        Fail,
        Hang,
    }

    struct ProbeOutput {
        process: Output,
        wtype_args: String,
    }

    fn script() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("scripts/test-startup-readiness.sh")
    }

    fn executable(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn bounded_output(mut command: Command) -> Output {
        let mut child = command
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if started.elapsed() > Duration::from_secs(6) {
                let group = format!("-{}", child.id());
                Command::new("kill")
                    .args(["-KILL", "--", &group])
                    .status()
                    .unwrap();
                let output = child.wait_with_output().unwrap();
                panic!(
                    "startup probe exceeded test timeout; stdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn run_probe(
        focus_us: Option<u64>,
        result: &str,
        status: i32,
        wtype_behavior: WtypeBehavior,
    ) -> ProbeOutput {
        let dir = tempfile::tempdir().unwrap();
        let trigger = dir.path().join("trigger");
        let args_log = dir.path().join("wtype-args");
        let pid_log = dir.path().join("pikr-pid");
        let pikr = dir.path().join("pikr");
        let wtype = dir.path().join("wtype");
        executable(
            &pikr,
            &format!(
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$$\" >\"$PIKR_TEST_PID\"\ncat >/dev/null\nprintf '2026-08-04T00:00:00Z DEBUG pikr::app: config loaded cfg=Config {{ font: \"startup first focus received elapsed_us=1\" }}\\n' >&2\nif [[ \"${{NO_COLOR:-}}\" == 1 && -n \"${{PIKR_TEST_FOCUS_US:-}}\" ]]; then printf '2026-08-04T00:00:00Z DEBUG pikr::ui::view: startup first focus received elapsed_us=%s\\n' \"$PIKR_TEST_FOCUS_US\" >&2; fi\nprintf 'banana\\n' >&2\nwhile [[ ! -e \"$PIKR_TEST_TRIGGER\" ]]; do sleep 0.01; done\nprintf '%s\\n' '{result}'\nexit {status}\n"
            ),
        );
        let wtype_body = match wtype_behavior {
            WtypeBehavior::Succeed => {
                "printf '%s\\n' \"$*\" >>\"$PIKR_TEST_WTYPE_ARGS\"\ntouch \"$PIKR_TEST_TRIGGER\"\n"
            }
            WtypeBehavior::Fail => "printf '%s\\n' \"$*\" >>\"$PIKR_TEST_WTYPE_ARGS\"\nexit 7\n",
            WtypeBehavior::Hang => {
                "printf '%s\\n' \"$*\" >>\"$PIKR_TEST_WTYPE_ARGS\"\nexec sleep 9999\n"
            }
        };
        executable(&wtype, &format!("#!/usr/bin/env bash\n{wtype_body}"));

        let mut command = Command::new("bash");
        command
            .arg(script())
            .args(["--delay", "0.500"])
            .env("WAYLAND_DISPLAY", "test")
            .env("PIKR_BIN", pikr)
            .env("WTYPE_BIN", wtype)
            .env("PIKR_FOCUS_ATTEMPTS", "5")
            .env("PIKR_TEST_TRIGGER", trigger)
            .env("PIKR_TEST_PID", &pid_log)
            .env("PIKR_TEST_WTYPE_ARGS", &args_log);
        if let Some(focus_us) = focus_us {
            command.env("PIKR_TEST_FOCUS_US", focus_us.to_string());
        }
        let process = bounded_output(command);
        let wtype_args = fs::read_to_string(args_log).unwrap_or_default();
        let pikr_pid = fs::read_to_string(pid_log).unwrap();
        let pikr_alive = Command::new("kill")
            .args(["-0", pikr_pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!pikr_alive, "probe left fake Pikr process {pikr_pid} alive");
        ProbeOutput {
            process,
            wtype_args,
        }
    }

    #[test]
    fn accepts_matching_candidate_after_focus_within_deadline() {
        let out = run_probe(Some(1_000), "banana", 0, WtypeBehavior::Succeed);
        assert!(
            out.process.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.process.stdout),
            String::from_utf8_lossy(&out.process.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.process.stdout);
        assert!(stdout.contains("PASS"));
        assert!(stdout.contains("first focus at 1000 us"));
        assert_eq!(out.wtype_args, "-k F12 -k Return\n");
    }

    #[test]
    fn rejects_wrong_result() {
        let out = run_probe(Some(1_000), "apple", 0, WtypeBehavior::Succeed);
        assert!(!out.process.status.success());
        assert!(String::from_utf8_lossy(&out.process.stderr).contains("FAIL"));
        assert_eq!(out.wtype_args, "-k F12 -k Return\n");
    }

    #[test]
    fn rejects_focus_after_deadline_without_injecting_keys() {
        let out = run_probe(Some(500_001), "banana", 0, WtypeBehavior::Succeed);
        assert!(!out.process.status.success());
        assert!(
            String::from_utf8_lossy(&out.process.stderr)
                .contains("exceeding the 500000 us deadline")
        );
        assert!(out.wtype_args.is_empty());
    }

    #[test]
    fn terminates_when_focus_marker_never_arrives() {
        let out = run_probe(None, "banana", 0, WtypeBehavior::Succeed);
        assert!(!out.process.status.success());
        assert!(
            String::from_utf8_lossy(&out.process.stderr)
                .contains("did not report an authentic first-focus event")
        );
        assert!(out.wtype_args.is_empty());
    }

    #[test]
    fn rejects_failed_wtype() {
        let out = run_probe(Some(1_000), "banana", 0, WtypeBehavior::Fail);
        assert!(!out.process.status.success());
        assert!(String::from_utf8_lossy(&out.process.stderr).contains("wtype could not send"));
        assert_eq!(out.wtype_args, "-k F12 -k Return\n");
    }

    #[test]
    fn times_out_hanging_wtype() {
        let out = run_probe(Some(1_000), "banana", 0, WtypeBehavior::Hang);
        assert!(!out.process.status.success());
        assert!(String::from_utf8_lossy(&out.process.stderr).contains("wtype could not send"));
        assert_eq!(out.wtype_args, "-k F12 -k Return\n");
    }
}
