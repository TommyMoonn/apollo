# v0.5.0 local candidate acceptance

## Scope and identity

Recorded on **2026-09-10** for phase 0.5.0.4. This is local candidate evidence,
not approval of a published v0.5.0 release. Source baseline:
`2b4e7554b29b534c3f97311e0d0c920578637dc0`, plus the phase's documentation and
acceptance-test changes. Production Rust code and the media manifest were
unchanged. At the time of this run, Cargo and the tested installer identified
Apollo as **0.4.0**. This historical record does not certify artifacts rebuilt
after the version was set to **0.5.0**; those need fresh artifact acceptance.

Environment: Windows build **26200.9168** (25H2), x64 MSVC target, NSIS **3.11**,
**NVIDIA GeForce RTX 3060 Laptop GPU**, Windows driver **566.14**. GPU and driver
were read using `nvidia-smi`, not inferred from the manifest.

The exact distribution remains FFmpeg **7.0.2**, Gyan
`7.0.2-essentials_build-www.gyan.dev`. Its NVENC API **12.2** expectation is tied
to the recorded header build and earlier real encode evidence in
[`media-tools-acceptance.md`](../distribution/media-tools-acceptance.md).
This run does not establish a minimum driver or certify other GPU models.

| Input or artifact | SHA-256 |
| --- | --- |
| Pinned archive | `d5308d30872b2739cf53169df61faba8639d39a19b20b91e611c177ef676f64c` |
| Repository/staged media-tools.json | `52ae1d06e3affa3d501f2b59642476d68c9aa898b61868350fb4821df0300cce` |
| Apollo.exe | `f964b8da951918c90154965ad29dcfe905bc58930da35a103fd272fa6f0c648b` |
| ffmpeg.exe | `33cf0d2a42486a59f74f3b3741d8ff71bed82169db7125e91804cf264b365a4a` |
| ffprobe.exe | `af3c38d4a25acf3bf0f16c3e36b7f0700bbf8fc1159057186a6f3f1fe7cd1611` |
| Apollo-Setup-x64.exe | `4f1f30567b07d812d9f95863c6bf1eebb070f244fdd872cc7b940964645deff1` |
| Apollo-windows-x64.zip | `5d4799bc6bb748011ec6f4cb7e0bb67414304d014558f51bdba84d613f2b366a` |

The installer and portable archive were staged offline into
`.scratch/phase-0.5.0.4-release/`. Each has been checked against `SHA256SUMS.txt`.
The acceptance script independently stages the verified cached archive, checks
all eight payload files byte-for-byte in both distribution forms, and checks
the installed manifest against the repository manifest. Thus the operation
results below concern the pinned binaries from the actual payloads, not PATH
substitutes. Scratch artifacts/logs are local evidence, not clean-clone inputs.

## Commands and results

Release build and staging passed:

```powershell
cargo build --locked --release --target x86_64-pc-windows-msvc
pwsh -NoProfile -File scripts/stage-windows-release.ps1 --offline `
    --makensis '<local NSIS 3.11 makensis.exe>' --output .scratch/phase-0.5.0.4-release
```

The harness was built using the Cargo JSON-artifact selection command in
[RELEASES.md](RELEASES.md#self-contained-media-acceptance). The following passed:

```powershell
pwsh -NoProfile -File tests/scripts/WindowsRelease.Smoke.ps1 `
    -ReleaseDirectory .scratch/phase-0.5.0.4-release `
    -DiscoveryHarness '<compiled bundled_media_tools test executable>' -RequireNvenc
```

The default command was also run without `-RequireNvenc` in a separate shell
with `CUDA_VISIBLE_DEVICES=-1`, so all its child processes had no visible CUDA
devices. That run passed too. Neither run changed the user's PATH or driver.

| Check | Portable | Installed |
| --- | --- | --- |
| Empty-PATH production discovery and probe | Pass | Pass |
| Audio Mix, gain and normalized variants | Pass | Pass |
| CPU H.264 and H.265 Upscale | Pass | Pass |
| Stream-copy MKV Remux | Pass | Pass |
| Original/AAC/FLAC Audio Extract | Pass | Pass |
| PNG/JPEG Frame Capture at 500 ms | Pass | Pass |
| Both real NVENC capability probes on baseline | Available | Available |
| Explicit H.264 and H.265 NVENC Upscale, CQ 16 | Pass | Pass |
| Auto selects NVENC after successful probes | Pass | Pass |
| Real probe failure with CUDA devices hidden | RuntimeUnavailable | RuntimeUnavailable |
| Auto resolves to CPU and completes after failure | Pass | Pass |
| Explicit NVENC refuses with the real retained diagnostic | Pass | Pass |
| Payload licenses, notices and manifest hashes | Pass | Pass |

Each operation used its existing production request validation, command builder,
executor and safe output publication. Generated outputs were checked by the
payload's FFprobe for the contract in RELEASES.md. Upscale used a one-second
320x180 source and produced 640x360 8-bit H.264/HEVC with both AAC tracks.

Successful NVIDIA runs returned both capabilities as `Available` and completed
both CQ 16 encodes. No required-API-newer-than-driver failure occurred. The
deliberate failure run returned `CUDA_ERROR_NO_DEVICE` from both real probes;
Auto completed with CPU CRF 18/20, while explicit selection retained that error.
This is a genuine initialization-failure fixture with unmodified binaries,
**not** an incompatible-driver/API-version test.

Silent installation, installation-path registration, uninstall, registration
and shortcut removal, and preservation of test-owned unrelated files passed.
The smoke removed its isolated installation and scratch outputs after each run.

## Repository validation

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed without warnings.
- `cargo test --all-targets`: **630 passed**, 0 failed, 3 opt-in tests ignored.
- `pwsh -NoProfile -File tests/scripts/run.ps1`: **130 passed**, all 6 suites passed.
  Three existing import/restore backup-advisory warnings were emitted by the
  project-tooling fixtures; the PowerShell log is not warning-free.
- The ignored bundled-resolution integration test was run separately with the
  staged pair: **1 passed**, including invalid/partial bundles and explicit PATH.
- The ignored real CPU and NVIDIA Upscale regressions were run separately with
  the pinned pair on process-local PATH: **2 passed**, including 10-bit PQ/HLG
  H.265 format/color checks.

Local logs use `.scratch/phase-0.5.0.4-` prefixes: `package.log`,
`native-smoke.log`, `cpu-only-smoke.log`, `rust.log`, `powershell.log`,
`bundle-regression.log` and `upscale-regression.log`.

## Limits and publication gates

- No obsolete-driver machine was used; its distinct API-mismatch behavior is
  not certified. No minimum-driver claim is made.
- No rendered native-window walkthrough, installer upgrade-over-existing-install
  check or perceptual media-quality comparison was performed in this phase.
  Operation-level and isolated-install evidence must not be presented as those checks.
- No GitHub-hosted run, tag, release upload or public publication was performed.
  GPU-free behavior was exercised locally with devices hidden, not on a hosted runner.
- License and notice byte retention passed. Complete corresponding-source and
  build-material delivery for all static dependencies has **not** been verified.
  The release operator must complete and document that review before public
  distribution; the FFmpeg source-commit link alone does not close it.
- A version bump, changed notice bytes, rebuilt executable, repackaged installer
  or ZIP, or changed pin requires new artifact identity and acceptance evidence.
  This record must not be reused as approval of later tagged assets.

The [publication checklist](RELEASES.md#publish) remains authoritative.
