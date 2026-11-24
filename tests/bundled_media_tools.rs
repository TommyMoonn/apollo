//! Opt-in runtime check against a staged distribution, without mutating the test runner's PATH.
use std::env;
#[path = "support/release_media_acceptance.rs"]
mod release_media_acceptance;
#[cfg(windows)]
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use apollo::media::tools::{
    MediaToolKind, MediaToolValidationError, MediaToolsError, cached_video_encoder_capabilities,
    discover_base_media_tools,
};

#[test]
#[ignore = "subprocess entry point; requires an explicit discovery mode and isolated environment"]
fn discovery_subprocess() {
    let mode = env::var("APOLLO_DISCOVERY_TEST_MODE")
        .expect("discovery harness requires APOLLO_DISCOVERY_TEST_MODE");
    let directory = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let result = discover_base_media_tools();
    match mode.as_str() {
        "bundle" | "path" => {
            let tools = result.unwrap();
            let expected = if mode == "bundle" {
                directory
            } else {
                directory.join("path tools")
            };
            assert_eq!(tools.ffmpeg, expected.join("ffmpeg.exe"));
            assert_eq!(tools.ffprobe, expected.join("ffprobe.exe"));
            let capabilities = cached_video_encoder_capabilities(&tools.ffmpeg);
            println!("Resolved {}: {capabilities:?}", tools.ffmpeg.display());
            if env::var_os("APOLLO_TEST_REQUIRE_NVENC").is_some() {
                use apollo::media::tools::VideoEncoderCapability;
                assert_eq!(capabilities.h264_nvenc, VideoEncoderCapability::Available);
                assert_eq!(capabilities.h265_nvenc, VideoEncoderCapability::Available);
            }
            if let Ok(acceptance) = env::var("APOLLO_RELEASE_ACCEPTANCE") {
                release_media_acceptance::run(&tools, &capabilities, &acceptance);
            }
        }
        "partial" => assert_eq!(
            result,
            Err(MediaToolsError::PartialBundle {
                directory,
                missing: MediaToolKind::Ffprobe,
            })
        ),
        "absent" => assert_eq!(result, Err(MediaToolsError::MissingBoth)),
        "invalid-ffmpeg" | "invalid-ffprobe" => {
            let expected_tool = if mode == "invalid-ffmpeg" {
                MediaToolKind::Ffmpeg
            } else {
                MediaToolKind::Ffprobe
            };
            let error = result.unwrap_err();
            let MediaToolsError::ValidationFailed { tool, path, reason } = error else {
                panic!("expected validation failure, got {error:?}");
            };
            assert_eq!(tool, expected_tool);
            let name = if tool == MediaToolKind::Ffmpeg {
                "ffmpeg.exe"
            } else {
                "ffprobe.exe"
            };
            assert_eq!(path, directory.join(name));
            let MediaToolValidationError::QueryFailed(diagnostic) = reason else {
                panic!("expected rejected version query, got {reason:?}");
            };
            assert!(!diagnostic.trim().is_empty());
        }
        _ => panic!("unknown subprocess mode: {mode}"),
    }
    println!("PASS discovery: {mode}");
}

#[cfg(windows)]
#[test]
#[ignore = "requires APOLLO_TEST_MEDIA_TOOLS_DIR pointing to the staged Windows pair"]
fn real_pair_uses_application_directory_and_explicit_path_fallback() {
    let source = PathBuf::from(
        env::var_os("APOLLO_TEST_MEDIA_TOOLS_DIR").expect("set staged pair directory"),
    );
    let root = env::temp_dir().join(format!(
        "apollo runtime discovery {}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let decoy = root.join("working directory");
    fs::create_dir(&decoy).unwrap();
    fs::write(decoy.join("ffmpeg.exe"), b"invalid working-directory decoy").unwrap();
    // Each scenario has immutable executable files, avoiding Windows image-file replacement locks.
    for mode in [
        "bundle",
        "invalid-ffmpeg",
        "invalid-ffprobe",
        "partial",
        "absent",
        "path",
    ] {
        let scenario = root.join(mode);
        fs::create_dir(&scenario).unwrap();
        let executable = scenario.join("discovery-test.exe");
        fs::copy(env::current_exe().unwrap(), &executable).unwrap();
        let path_tools = scenario.join("path tools");
        fs::create_dir(&path_tools).unwrap();
        for (name, invalid_mode) in [
            ("ffmpeg.exe", "invalid-ffmpeg"),
            ("ffprobe.exe", "invalid-ffprobe"),
        ] {
            fs::copy(source.join(name), path_tools.join(name)).unwrap();
            if mode == "absent" || mode == "path" || (mode == "partial" && name == "ffprobe.exe") {
                continue;
            }
            let input = if mode == invalid_mode {
                executable.clone()
            } else {
                source.join(name)
            };
            fs::copy(input, scenario.join(name)).unwrap();
        }
        let path = if mode == "bundle" || mode == "absent" {
            ""
        } else {
            path_tools.to_str().unwrap()
        };
        // Redirect to files so waiting for process exit cannot deadlock on full output pipes.
        let stdout = scenario.join("stdout.log");
        let stderr = scenario.join("stderr.log");
        let mut child = Command::new(&executable)
            .args([
                "--ignored",
                "--exact",
                "discovery_subprocess",
                "--nocapture",
            ])
            .env("APOLLO_DISCOVERY_TEST_MODE", mode)
            .env_remove("APOLLO_RELEASE_ACCEPTANCE")
            .env_remove("APOLLO_RELEASE_OUTPUT_DIRECTORY")
            .env("PATH", path)
            .current_dir(&decoy)
            .stdout(Stdio::from(fs::File::create(&stdout).unwrap()))
            .stderr(Stdio::from(fs::File::create(&stderr).unwrap()))
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{mode}: discovery subprocess timed out");
            }
            thread::sleep(Duration::from_millis(10));
        };
        let stdout = fs::read_to_string(stdout).unwrap();
        let stderr = fs::read_to_string(stderr).unwrap();
        assert!(status.success(), "{mode}: {stdout}\n{stderr}");
        assert!(
            stdout.contains(&format!("PASS discovery: {mode}")),
            "discovery scenario did not execute: {stdout}\n{stderr}"
        );
        print!("{stdout}");
    }
}
