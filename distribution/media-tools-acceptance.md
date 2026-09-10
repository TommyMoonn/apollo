# Pinned media-tool build acceptance

`media-tools.json` is the distribution identity and staging contract. This record
explains the evidence behind its compatibility baseline. It does not certify
installer or portable artifacts, which belong to phases 0.5.0.3 and 0.5.0.4.

## Archive provenance

Selected from the distributor's [7.0.2 release](https://github.com/GyanD/codexffmpeg/releases/tag/7.0.2):
the Windows x64 static essentials ZIP. Its contents identify FFmpeg source commit
`e3a61e9103`, build `7.0.2-essentials_build-www.gyan.dev`, GPL v3, and external
library versions. The archive was downloaded over HTTPS and SHA-256 measured
locally. A second clean-cache staging download produced the same checksum.
This is a measured archive digest, not a claim of a publisher signature.

| File | SHA-256 |
| --- | --- |
| Archive | `d5308d30872b2739cf53169df61faba8639d39a19b20b91e611c177ef676f64c` |
| ffmpeg.exe | `33cf0d2a42486a59f74f3b3741d8ff71bed82169db7125e91804cf264b365a4a` |
| ffprobe.exe | `af3c38d4a25acf3bf0f16c3e36b7f0700bbf8fc1159057186a6f3f1fe7cd1611` |

The archive's `LICENSE` and `README.txt` are preserved under `licenses/ffmpeg/`.
The README retains distributor attribution, source reference, build configuration
and dependency versions. Release-wide notices and source redistribution handling
remain the packaging/release phase's responsibility.

## Measured NVIDIA baseline

Acceptance ran on 2026-09-10 with Windows 11 build 26200, NVIDIA GeForce RTX 3060
Laptop GPU, and Windows driver **566.14**. That is the selected tested baseline;
it is not a measured minimum driver version or a certification of other GPUs.

The pinned archive README reports `ffnvcodec n12.2.72.0-1-g9934f17`.
The [corresponding NVIDIA header](https://github.com/FFmpeg/nv-codec-headers/blob/9934f17/include/ffnvcodec/nvEncodeAPI.h#L112)
defines NVENC API major 12 and minor 2. This ties the API expectation to this
distribution's recorded dependency identity, rather than to FFmpeg's source
version alone.

Both `h264_nvenc` and `hevc_nvenc` successfully encoded a frame using Apollo's
capability-probe arguments, with verbose logging added:

```powershell
# Repeat for h264_nvenc and hevc_nvenc using the staged executable.
& .scratch/staged-media-tools/ffmpeg.exe -hide_banner -loglevel verbose `
  -f lavfi -i color=c=black:s=256x256:r=1 -frames:v 1 -an `
  -c:v h264_nvenc -preset p5 -f null -
```

Each exited 0, reported `Loaded Nvenc version 12.2` and
`Nvenc initialized successfully`, and encoded one frame. Both also completed
a one-second 320x180 to 640x360 Lanczos Upscale at preset p5, VBR, CQ 16.
FFprobe confirmed H.264/HEVC output and 640x360 dimensions. No required-API-newer-
than-driver failure occurred. These results demonstrate that this exact binary
works with the selected baseline.

Apollo's existing real CPU and NVENC Upscale integration tests were then run
with the staged directory prepended to PATH in the test process only:

```powershell
$env:PATH = (Resolve-Path .scratch/staged-media-tools).Path + ';' + $env:PATH
cargo test --test upscale real_ -- --ignored --nocapture
```

Both passed. They exercise Apollo's command builder and probe owner for SDR
H.264, SDR HEVC, and 10-bit PQ/HLG HEVC, including color metadata preservation.
They supplement the direct capability probes; these tests set the request's
capability state to available themselves.

## Media contract checks

The staged pair passed both `-version` checks and the required encoder inventory.
Short synthetic-media checks passed for Audio Mix (volume, amix, alimiter, AAC),
loudness normalization (loudnorm), stream-copy MP4-to-MKV Remux, original/AAC/FLAC
Audio Extract, and PNG/JPEG Frame Capture. Every generated artifact was readable
by the staged FFprobe and exposed the expected codecs. The real Upscale tests
above exercised both CPU encoders and Lanczos scaling.

These are binary and command-level checks. Installed/portable no-PATH acceptance,
unsupported-driver behavior and the complete release gate remain unverified here.

## Staging and update contract

```powershell
pwsh -NoProfile -File scripts/stage-media-tools.ps1 --output .scratch/media-payload
pwsh -NoProfile -File scripts/stage-media-tools.ps1 --offline --output .scratch/media-payload-offline
pwsh -NoProfile -File tests/scripts/run.ps1
```

The default archive cache is `.scratch/media-tools-cache`. Each output must be a
new directory. Existing output is refused; failed staging publishes nothing.
Corrupt cache files fail with a checksum diagnostic before extraction. Supply a
fresh cache directory to redownload, or remove the identified corrupt archive.
Staging never invokes an NVENC encode and does not require an NVIDIA GPU.

Any archive URL, checksum, build identity or source-version change requires renewed
encoder validation, CPU smoke, both NVENC runtime probes, short NVENC Upscale
smoke and license/notice verification before replacing the accepted pin. Do not
infer acceptance from a matching source version or inventory alone.
