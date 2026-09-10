//! Opt-in runtime check against a staged distribution, without mutating the test runner's PATH.
use std::env;
#[path = "support/release_media_acceptance.rs"]
mod release_media_acceptance;
#[cfg(windows)]
use std::{fs, path::PathBuf, process::Command};

use apollo::media::tools::{
    MediaToolKind, MediaToolsError, cached_video_encoder_capabilities, discover_base_media_tools,
};

#[test]
fn discovery_subprocess() {
    let Ok(mode) = env::var("APOLLO_DISCOVERY_TEST_MODE") else {
        return;
    };
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
            assert!(
                reason.to_string().contains("Unrecognized option"),
                "{reason}"
            );
        }
        _ => panic!("unknown subprocess mode: {mode}"),
    }
}

#[cfg(windows)]
#[test]
#[ignore = "requires APOLLO_TEST_MEDIA_TOOLS_DIR pointing to the staged Windows pair"]
fn real_pair_uses_application_directory_and_explicit_path_fallback() {
    let source = PathBuf::from(
        env::var_os("APOLLO_TEST_MEDIA_TOOLS_DIR").expect("set staged pair directory"),
    );
    let root = env::temp_dir().join(format!("apollo runtime discovery {}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
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
        let output = Command::new(&executable)
            .args(["--exact", "discovery_subprocess", "--nocapture"])
            .env("APOLLO_DISCOVERY_TEST_MODE", mode)
            .env("PATH", path)
            .current_dir(&decoy)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
}
