# Third-Party Notices

Apollo redistributes third-party software and font material. This file preserves the notices and license terms required for the materials included in the Windows x64 application distribution.

For Rust packages that offer multiple alternative licenses, Apollo uses the Apache License 2.0 option when that option is present. Packages whose metadata requires additional or different terms are covered separately below. Build-only and proc-macro crates are not part of this binary-distribution inventory.

## Bundled FFmpeg and FFprobe

The Windows installer and portable archive include the Gyan Doshi Windows x64
static essentials distribution `7.0.2-essentials_build-www.gyan.dev` (FFmpeg
source version 7.0.2, source commit `e3a61e9103`).

- Distributor: https://www.gyan.dev/ffmpeg/builds/
- Exact archive: https://github.com/GyanD/codexffmpeg/releases/download/7.0.2/ffmpeg-7.0.2-essentials_build.zip
- Archive SHA-256: `d5308d30872b2739cf53169df61faba8639d39a19b20b91e611c177ef676f64c`
- Distribution license: GNU General Public License version 3, retained in `licenses/ffmpeg/LICENSE`.
- Distributor attribution, source reference, configuration and library versions: `licenses/ffmpeg/README.txt`.
- Exact pinned distribution metadata: `licenses/ffmpeg/media-tools.json`.
- FFmpeg source reference: https://github.com/FFmpeg/FFmpeg/commit/e3a61e9103

FFmpeg and FFprobe are separate executables. Their distribution includes external
libraries such as x264 and x265; the distributor's README identifies those builds.
The repository-owned `distribution/media-tools.json` controls binary selection.
The license and distributor README are included unchanged in both release formats.

Bundled executables are not covered by Apollo's Rust dependency inventory below.
The selected build includes GPL components such as x264 and x265. Keep its license,
README and pinned metadata together when redistributing either package. The
FFmpeg project documents its [licensing considerations](https://ffmpeg.org/legal.html).

The source commit link above identifies FFmpeg itself, not the complete
corresponding source of every statically included library. Before public binary
distribution, the release operator must verify the applicable corresponding-source
delivery and build-material obligations for this exact build and its dependencies,
and record the source locations in the release notes. Shipping this notice and a
license file alone is not evidence that this review has been completed.

## Repository-embedded fonts

### IBM Plex Sans Regular

- File: `assets/fonts/IBMPlexSans-Regular.ttf`
- Embedded font version: `3.005`
- Upstream project: IBM Plex
- Source: https://github.com/IBM/plex
- License: `OFL-1.1`
- Copyright notice: `Copyright 2018 IBM Corp. All rights reserved.`
- Trademark notice: `IBM Plex® is a trademark of IBM Corp, registered in many jurisdictions worldwide.`
- SHA-256: `975dcda37d80f038dcd143c22e33ca2d97a0cc5a929aace1c749153b0fe1afa5`
- License text: see [SIL Open Font License 1.1](#sil-open-font-license-11).

### Fira Code Regular

- File: `assets/fonts/FiraCode-Regular.ttf`
- Embedded font version: `6.002`
- Upstream project: Fira Code
- Source: https://github.com/tonsky/FiraCode
- License: `OFL-1.1`
- Copyright notice: `Copyright 2014-2021 The Fira Code Project Authors (https://github.com/tonsky/FiraCode)`
- Trademark notice: `Fira Mono is a trademark of The Mozilla Corporation.`
- SHA-256: `5992ab9640e2df491b2f609467b1de60e8bc39b2c28db184342a0592d98f6117`
- License text: see [SIL Open Font License 1.1](#sil-open-font-license-11).

## Rust dependency inventory

The following packages are reachable as non-proc-macro runtime dependencies for the locked Windows x64 Apollo application graph. License expressions are reproduced from package metadata.

| Package | Version | License expression | Upstream source |
| --- | --- | --- | --- |
| `accesskit` | `0.24.1` | `MIT OR Apache-2.0` | https://github.com/AccessKit/accesskit |
| `accesskit_consumer` | `0.35.0` | `MIT OR Apache-2.0` | https://github.com/AccessKit/accesskit |
| `accesskit_windows` | `0.32.1` | `MIT OR Apache-2.0` | https://github.com/AccessKit/accesskit |
| `accesskit_winit` | `0.32.2` | `Apache-2.0` | https://github.com/AccessKit/accesskit |
| `adler2` | `2.0.1` | `0BSD OR MIT OR Apache-2.0` | https://github.com/oyvindln/adler2 |
| `ahash` | `0.8.12` | `MIT OR Apache-2.0` | https://github.com/tkaitchuck/ahash |
| `allocator-api2` | `0.2.21` | `MIT OR Apache-2.0` | https://github.com/zakarumych/allocator-api2 |
| `arboard` | `3.6.1` | `MIT OR Apache-2.0` | https://github.com/1Password/arboard |
| `arrayvec` | `0.7.8` | `MIT OR Apache-2.0` | https://github.com/bluss/arrayvec |
| `ash` | `0.38.0+1.3.281` | `MIT OR Apache-2.0` | https://github.com/ash-rs/ash |
| `bit-set` | `0.10.0` | `Apache-2.0 OR MIT` | https://github.com/contain-rs/bit-set |
| `bit-vec` | `0.9.1` | `Apache-2.0 OR MIT` | https://github.com/contain-rs/bit-vec |
| `bitflags` | `2.13.1` | `MIT OR Apache-2.0` | https://github.com/bitflags/bitflags |
| `bytemuck` | `1.25.2` | `Zlib OR Apache-2.0 OR MIT` | https://github.com/Lokathor/bytemuck |
| `byteorder-lite` | `0.1.0` | `Unlicense OR MIT` | https://github.com/image-rs/byteorder-lite |
| `cfg-if` | `1.0.4` | `MIT OR Apache-2.0` | https://github.com/rust-lang/cfg-if |
| `clipboard-win` | `5.4.1` | `BSL-1.0` | https://github.com/DoumanAsh/clipboard-win |
| `codespan-reporting` | `0.13.1` | `Apache-2.0` | https://github.com/brendanzab/codespan |
| `color` | `0.3.3` | `Apache-2.0 OR MIT` | https://github.com/linebender/color |
| `crc32fast` | `1.5.0` | `MIT OR Apache-2.0` | https://github.com/srijs/rust-crc32fast |
| `cursor-icon` | `1.2.0` | `MIT OR Apache-2.0 OR Zlib` | https://github.com/rust-windowing/cursor-icon |
| `dpi` | `0.1.2` | `Apache-2.0 AND MIT` | https://github.com/rust-windowing/winit |
| `ecolor` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui |
| `eframe` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui/tree/main/crates/eframe |
| `egui` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui |
| `egui-wgpu` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui/tree/main/crates/egui-wgpu |
| `egui-winit` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui/tree/main/crates/egui-winit |
| `egui_glow` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui/tree/main/crates/egui_glow |
| `either` | `1.17.0` | `MIT OR Apache-2.0` | https://github.com/rayon-rs/either |
| `emath` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui/tree/main/crates/emath |
| `epaint` | `0.36.1` | `MIT OR Apache-2.0` | https://github.com/emilk/egui/tree/main/crates/epaint |
| `epaint_default_fonts` | `0.36.1` | `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0` | https://github.com/emilk/egui/tree/main/crates/epaint_default_fonts |
| `equivalent` | `1.0.2` | `Apache-2.0 OR MIT` | https://github.com/indexmap-rs/equivalent |
| `error-code` | `3.3.2` | `BSL-1.0` | https://github.com/DoumanAsh/error-code |
| `euclid` | `0.22.14` | `MIT OR Apache-2.0` | https://github.com/servo/euclid |
| `fax` | `0.2.7` | `MIT` | https://github.com/pdf-rs/fax |
| `fdeflate` | `0.3.7` | `MIT OR Apache-2.0` | https://github.com/image-rs/fdeflate |
| `fearless_simd` | `0.4.1` | `Apache-2.0 OR MIT` | https://github.com/linebender/fearless_simd |
| `flate2` | `1.1.9` | `MIT OR Apache-2.0` | https://github.com/rust-lang/flate2-rs |
| `foldhash` | `0.2.0` | `Zlib` | https://github.com/orlp/foldhash |
| `font-types` | `0.12.3` | `MIT OR Apache-2.0` | https://github.com/googlefonts/fontations |
| `form_urlencoded` | `1.2.2` | `MIT OR Apache-2.0` | https://github.com/servo/rust-url |
| `getrandom` | `0.3.4` | `MIT OR Apache-2.0` | https://github.com/rust-random/getrandom |
| `glifo` | `0.2.0` | `Apache-2.0 OR MIT` | https://github.com/linebender/vello |
| `glow` | `0.17.0` | `MIT OR Apache-2.0 OR Zlib` | https://github.com/grovesNL/glow |
| `glutin` | `0.32.3` | `Apache-2.0` | https://github.com/rust-windowing/glutin |
| `glutin-winit` | `0.5.0` | `MIT` | https://github.com/rust-windowing/glutin |
| `glutin_egl_sys` | `0.7.1` | `Apache-2.0` | https://github.com/rust-windowing/glutin |
| `glutin_wgl_sys` | `0.6.1` | `Apache-2.0` | https://github.com/rust-windowing/glutin |
| `gpu-allocator` | `0.28.0` | `MIT OR Apache-2.0` | https://github.com/Traverse-Research/gpu-allocator |
| `guillotiere` | `0.7.0` | `MIT/Apache-2.0` | https://github.com/nical/guillotiere |
| `half` | `2.7.1` | `MIT OR Apache-2.0` | https://github.com/VoidStarKat/half-rs |
| `harfrust` | `0.12.0` | `MIT` | https://github.com/harfbuzz/harfrust |
| `hashbrown` | `0.16.1` | `MIT OR Apache-2.0` | https://github.com/rust-lang/hashbrown |
| `hashbrown` | `0.17.1` | `MIT OR Apache-2.0` | https://github.com/rust-lang/hashbrown |
| `icu_collections` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `icu_locale_core` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `icu_normalizer` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `icu_normalizer_data` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `icu_properties` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `icu_properties_data` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `icu_provider` | `2.2.0` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `idna` | `1.1.0` | `MIT OR Apache-2.0` | https://github.com/servo/rust-url/ |
| `idna_adapter` | `1.2.2` | `Apache-2.0 OR MIT` | https://github.com/hsivonen/idna_adapter |
| `image` | `0.25.10` | `MIT OR Apache-2.0` | https://github.com/image-rs/image |
| `indexmap` | `2.14.0` | `Apache-2.0 OR MIT` | https://github.com/indexmap-rs/indexmap |
| `itertools` | `0.15.0` | `MIT OR Apache-2.0` | https://github.com/rust-itertools/itertools |
| `itoa` | `1.0.18` | `MIT OR Apache-2.0` | https://crates.io/crates/itoa/1.0.18 |
| `khronos-egl` | `6.0.0` | `MIT/Apache-2.0` | https://github.com/timothee-haudebourg/khronos-egl |
| `kurbo` | `0.13.1` | `Apache-2.0 OR MIT` | https://github.com/linebender/kurbo |
| `libc` | `0.2.189` | `MIT OR Apache-2.0` | https://github.com/rust-lang/libc |
| `libloading` | `0.8.9` | `ISC` | https://github.com/nagisa/rust_libloading/ |
| `libm` | `0.2.16` | `MIT` | https://github.com/rust-lang/compiler-builtins |
| `linebender_resource_handle` | `0.1.1` | `Apache-2.0 OR MIT` | https://github.com/linebender/raw_resource_handle |
| `litemap` | `0.8.2` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `lock_api` | `0.4.14` | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| `log` | `0.4.33` | `MIT OR Apache-2.0` | https://github.com/rust-lang/log |
| `memchr` | `2.8.3` | `Unlicense OR MIT` | https://github.com/BurntSushi/memchr |
| `miniz_oxide` | `0.8.9` | `MIT OR Zlib OR Apache-2.0` | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| `moxcms` | `0.8.1` | `BSD-3-Clause OR Apache-2.0` | https://github.com/awxkee/moxcms.git |
| `naga` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `naga-types` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `nohash-hasher` | `0.2.0` | `Apache-2.0 OR MIT` | https://github.com/paritytech/nohash-hasher |
| `num-traits` | `0.2.19` | `MIT OR Apache-2.0` | https://github.com/rust-num/num-traits |
| `once_cell` | `1.21.4` | `MIT OR Apache-2.0` | https://github.com/matklad/once_cell |
| `ordered-float` | `5.3.0` | `MIT` | https://github.com/reem/rust-ordered-float |
| `parking_lot` | `0.12.5` | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| `parking_lot_core` | `0.9.12` | `MIT OR Apache-2.0` | https://github.com/Amanieu/parking_lot |
| `peniko` | `0.6.1` | `Apache-2.0 OR MIT` | https://github.com/linebender/peniko |
| `percent-encoding` | `2.3.2` | `MIT OR Apache-2.0` | https://github.com/servo/rust-url/ |
| `pin-project-lite` | `0.2.17` | `Apache-2.0 OR MIT` | https://github.com/taiki-e/pin-project-lite |
| `png` | `0.18.1` | `MIT OR Apache-2.0` | https://github.com/image-rs/image-png |
| `pollster` | `1.0.1` | `Apache-2.0/MIT` | https://github.com/zesterer/pollster |
| `polycool` | `0.4.0` | `MIT OR Apache-2.0` | https://github.com/linebender/kurbo |
| `potential_utf` | `0.1.5` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `presser` | `0.3.1` | `MIT OR Apache-2.0` | https://github.com/EmbarkStudios/presser |
| `profiling` | `1.0.18` | `MIT OR Apache-2.0` | https://github.com/aclysma/profiling |
| `pxfm` | `0.1.30` | `BSD-3-Clause OR Apache-2.0` | https://github.com/awxkee/pxfm |
| `quick-error` | `2.0.1` | `MIT/Apache-2.0` | http://github.com/tailhook/quick-error |
| `range-alloc` | `0.1.5` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/range-alloc |
| `raw-window-handle` | `0.6.2` | `MIT OR Apache-2.0 OR Zlib` | https://github.com/rust-windowing/raw-window-handle |
| `read-fonts` | `0.41.0` | `MIT OR Apache-2.0` | https://github.com/googlefonts/fontations |
| `renderdoc-sys` | `1.1.0` | `MIT OR Apache-2.0` | https://github.com/ebkalderon/renderdoc-rs |
| `rustc-hash` | `1.1.0` | `Apache-2.0/MIT` | https://github.com/rust-lang-nursery/rustc-hash |
| `rustc-hash` | `2.1.3` | `Apache-2.0 OR MIT` | https://github.com/rust-lang/rustc-hash |
| `scopeguard` | `1.2.0` | `MIT OR Apache-2.0` | https://github.com/bluss/scopeguard |
| `self_cell` | `1.3.0` | `Apache-2.0 OR GPL-2.0-only` | https://github.com/Voultapher/self_cell |
| `serde` | `1.0.229` | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| `serde_core` | `1.0.229` | `MIT OR Apache-2.0` | https://github.com/serde-rs/serde |
| `serde_json` | `1.0.151` | `MIT OR Apache-2.0` | https://crates.io/crates/serde_json/1.0.151 |
| `simd-adler32` | `0.3.10` | `MIT` | https://github.com/mcountryman/simd-adler32 |
| `skrifa` | `0.44.0` | `MIT OR Apache-2.0` | https://github.com/googlefonts/fontations |
| `smallvec` | `1.15.2` | `MIT OR Apache-2.0` | https://github.com/servo/rust-smallvec |
| `smol_str` | `0.2.2` | `MIT OR Apache-2.0` | https://github.com/rust-analyzer/smol_str |
| `spirv` | `0.4.0+sdk-1.4.341.0` | `Apache-2.0` | https://github.com/gfx-rs/rspirv |
| `stable_deref_trait` | `1.2.1` | `MIT OR Apache-2.0` | https://github.com/storyyeller/stable_deref_trait |
| `static_assertions` | `1.1.0` | `MIT OR Apache-2.0` | https://github.com/nvzqz/static-assertions-rs |
| `termcolor` | `1.4.1` | `Unlicense OR MIT` | https://github.com/BurntSushi/termcolor |
| `thiserror` | `2.0.20` | `MIT OR Apache-2.0` | https://github.com/dtolnay/thiserror |
| `tiff` | `0.11.3` | `MIT` | https://github.com/image-rs/image-tiff |
| `tinystr` | `0.8.3` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `tracing` | `0.1.44` | `MIT` | https://github.com/tokio-rs/tracing |
| `tracing-core` | `0.1.36` | `MIT` | https://github.com/tokio-rs/tracing |
| `type-map` | `0.5.1` | `MIT/Apache-2.0` | https://github.com/kardeiz/type-map |
| `unicode-general-category` | `1.1.0` | `Apache-2.0` | https://github.com/yeslogic/unicode-general-category |
| `unicode-ident` | `1.0.24` | `(MIT OR Apache-2.0) AND Unicode-3.0` | https://github.com/dtolnay/unicode-ident |
| `unicode-segmentation` | `1.13.3` | `MIT OR Apache-2.0` | https://github.com/unicode-rs/unicode-segmentation |
| `unicode-width` | `0.2.2` | `MIT OR Apache-2.0` | https://github.com/unicode-rs/unicode-width |
| `url` | `2.5.8` | `MIT OR Apache-2.0` | https://github.com/servo/rust-url |
| `utf8_iter` | `1.0.4` | `Apache-2.0 OR MIT` | https://github.com/hsivonen/utf8_iter |
| `uuid` | `1.24.0` | `Apache-2.0 OR MIT` | https://github.com/uuid-rs/uuid |
| `vello_common` | `0.1.0` | `Apache-2.0 OR MIT` | https://github.com/linebender/vello |
| `vello_cpu` | `0.1.0` | `Apache-2.0 OR MIT` | https://github.com/linebender/vello |
| `web-time` | `1.1.0` | `MIT OR Apache-2.0` | https://github.com/daxpedda/web-time |
| `webbrowser` | `1.2.4` | `MIT OR Apache-2.0` | https://github.com/amodm/webbrowser-rs |
| `weezl` | `0.1.12` | `MIT OR Apache-2.0` | https://github.com/image-rs/weezl |
| `wgpu` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `wgpu-core` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `wgpu-core-deps-windows-linux-android` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `wgpu-hal` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `wgpu-naga-bridge` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `wgpu-types` | `30.0.0` | `MIT OR Apache-2.0` | https://github.com/gfx-rs/wgpu |
| `winapi-util` | `0.1.11` | `Unlicense OR MIT` | https://github.com/BurntSushi/winapi-util |
| `windows` | `0.62.2` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-collections` | `0.3.2` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-core` | `0.62.2` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-future` | `0.3.2` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-link` | `0.2.1` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-numerics` | `0.3.1` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-result` | `0.4.1` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-strings` | `0.5.1` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-sys` | `0.52.0` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-sys` | `0.60.2` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-sys` | `0.61.2` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-targets` | `0.52.6` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-targets` | `0.53.5` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows-threading` | `0.2.1` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows_x86_64_msvc` | `0.52.6` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `windows_x86_64_msvc` | `0.53.1` | `MIT OR Apache-2.0` | https://github.com/microsoft/windows-rs |
| `winit` | `0.30.13` | `Apache-2.0` | https://github.com/rust-windowing/winit |
| `writeable` | `0.6.3` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `yoke` | `0.8.3` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `zerocopy` | `0.8.56` | `BSD-2-Clause OR Apache-2.0 OR MIT` | https://github.com/google/zerocopy |
| `zerofrom` | `0.1.8` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `zerotrie` | `0.2.4` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `zerovec` | `0.11.6` | `Unicode-3.0` | https://github.com/unicode-org/icu4x |
| `zmij` | `1.0.21` | `MIT OR Apache-2.0` | https://crates.io/crates/zmij/1.0.21 |
| `zune-core` | `0.5.3` | `MIT OR Apache-2.0 OR Zlib` | https://github.com/etemesi254/zune-image |
| `zune-jpeg` | `0.5.15` | `MIT OR Apache-2.0 OR Zlib` | https://github.com/etemesi254/zune-image/tree/dev/crates/zune-jpeg |

## License and attribution texts

No runtime package in this inventory contains a top-level `NOTICE` file. Package-level `COPYRIGHT` files and bundled font notices that carry additional attribution are reproduced below.

### Apache License 2.0

Applies under the Apache-2.0 option to: `accesskit 0.24.1`, `accesskit_consumer 0.35.0`, `accesskit_windows 0.32.1`, `accesskit_winit 0.32.2`, `adler2 2.0.1`, `ahash 0.8.12`, `allocator-api2 0.2.21`, `arboard 3.6.1`, `arrayvec 0.7.8`, `ash 0.38.0+1.3.281`, `bit-set 0.10.0`, `bit-vec 0.9.1`, `bitflags 2.13.1`, `bytemuck 1.25.2`, `cfg-if 1.0.4`, `codespan-reporting 0.13.1`, `color 0.3.3`, `crc32fast 1.5.0`, `cursor-icon 1.2.0`, `dpi 0.1.2`, `ecolor 0.36.1`, `eframe 0.36.1`, `egui 0.36.1`, `egui-wgpu 0.36.1`, `egui-winit 0.36.1`, `egui_glow 0.36.1`, `either 1.17.0`, `emath 0.36.1`, `epaint 0.36.1`, `epaint_default_fonts 0.36.1`, `equivalent 1.0.2`, `euclid 0.22.14`, `fdeflate 0.3.7`, `fearless_simd 0.4.1`, `flate2 1.1.9`, `font-types 0.12.3`, `form_urlencoded 1.2.2`, `getrandom 0.3.4`, `glifo 0.2.0`, `glow 0.17.0`, `glutin 0.32.3`, `glutin_egl_sys 0.7.1`, `glutin_wgl_sys 0.6.1`, `gpu-allocator 0.28.0`, `guillotiere 0.7.0`, `half 2.7.1`, `hashbrown 0.16.1`, `hashbrown 0.17.1`, `idna 1.1.0`, `idna_adapter 1.2.2`, `image 0.25.10`, `indexmap 2.14.0`, `itertools 0.15.0`, `itoa 1.0.18`, `khronos-egl 6.0.0`, `kurbo 0.13.1`, `libc 0.2.189`, `linebender_resource_handle 0.1.1`, `lock_api 0.4.14`, `log 0.4.33`, `miniz_oxide 0.8.9`, `moxcms 0.8.1`, `naga 30.0.0`, `naga-types 30.0.0`, `nohash-hasher 0.2.0`, `num-traits 0.2.19`, `once_cell 1.21.4`, `parking_lot 0.12.5`, `parking_lot_core 0.9.12`, `peniko 0.6.1`, `percent-encoding 2.3.2`, `pin-project-lite 0.2.17`, `png 0.18.1`, `pollster 1.0.1`, `polycool 0.4.0`, `presser 0.3.1`, `profiling 1.0.18`, `pxfm 0.1.30`, `quick-error 2.0.1`, `range-alloc 0.1.5`, `raw-window-handle 0.6.2`, `read-fonts 0.41.0`, `renderdoc-sys 1.1.0`, `rustc-hash 1.1.0`, `rustc-hash 2.1.3`, `scopeguard 1.2.0`, `self_cell 1.3.0`, `serde 1.0.229`, `serde_core 1.0.229`, `serde_json 1.0.151`, `skrifa 0.44.0`, `smallvec 1.15.2`, `smol_str 0.2.2`, `spirv 0.4.0+sdk-1.4.341.0`, `stable_deref_trait 1.2.1`, `static_assertions 1.1.0`, `thiserror 2.0.20`, `type-map 0.5.1`, `unicode-general-category 1.1.0`, `unicode-ident 1.0.24`, `unicode-segmentation 1.13.3`, `unicode-width 0.2.2`, `url 2.5.8`, `utf8_iter 1.0.4`, `uuid 1.24.0`, `vello_common 0.1.0`, `vello_cpu 0.1.0`, `web-time 1.1.0`, `webbrowser 1.2.4`, `weezl 0.1.12`, `wgpu 30.0.0`, `wgpu-core 30.0.0`, `wgpu-core-deps-windows-linux-android 30.0.0`, `wgpu-hal 30.0.0`, `wgpu-naga-bridge 30.0.0`, `wgpu-types 30.0.0`, `windows 0.62.2`, `windows-collections 0.3.2`, `windows-core 0.62.2`, `windows-future 0.3.2`, `windows-link 0.2.1`, `windows-numerics 0.3.1`, `windows-result 0.4.1`, `windows-strings 0.5.1`, `windows-sys 0.52.0`, `windows-sys 0.60.2`, `windows-sys 0.61.2`, `windows-targets 0.52.6`, `windows-targets 0.53.5`, `windows-threading 0.2.1`, `windows_x86_64_msvc 0.52.6`, `windows_x86_64_msvc 0.53.1`, `winit 0.30.13`, `zerocopy 0.8.56`, `zmij 1.0.21`, `zune-core 0.5.3`, `zune-jpeg 0.5.15`.

```text
                              Apache License
                        Version 2.0, January 2004
                     http://www.apache.org/licenses/

TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

1. Definitions.

   "License" shall mean the terms and conditions for use, reproduction,
   and distribution as defined by Sections 1 through 9 of this document.

   "Licensor" shall mean the copyright owner or entity authorized by
   the copyright owner that is granting the License.

   "Legal Entity" shall mean the union of the acting entity and all
   other entities that control, are controlled by, or are under common
   control with that entity. For the purposes of this definition,
   "control" means (i) the power, direct or indirect, to cause the
   direction or management of such entity, whether by contract or
   otherwise, or (ii) ownership of fifty percent (50%) or more of the
   outstanding shares, or (iii) beneficial ownership of such entity.

   "You" (or "Your") shall mean an individual or Legal Entity
   exercising permissions granted by this License.

   "Source" form shall mean the preferred form for making modifications,
   including but not limited to software source code, documentation
   source, and configuration files.

   "Object" form shall mean any form resulting from mechanical
   transformation or translation of a Source form, including but
   not limited to compiled object code, generated documentation,
   and conversions to other media types.

   "Work" shall mean the work of authorship, whether in Source or
   Object form, made available under the License, as indicated by a
   copyright notice that is included in or attached to the work
   (an example is provided in the Appendix below).

   "Derivative Works" shall mean any work, whether in Source or Object
   form, that is based on (or derived from) the Work and for which the
   editorial revisions, annotations, elaborations, or other modifications
   represent, as a whole, an original work of authorship. For the purposes
   of this License, Derivative Works shall not include works that remain
   separable from, or merely link (or bind by name) to the interfaces of,
   the Work and Derivative Works thereof.

   "Contribution" shall mean any work of authorship, including
   the original version of the Work and any modifications or additions
   to that Work or Derivative Works thereof, that is intentionally
   submitted to Licensor for inclusion in the Work by the copyright owner
   or by an individual or Legal Entity authorized to submit on behalf of
   the copyright owner. For the purposes of this definition, "submitted"
   means any form of electronic, verbal, or written communication sent
   to the Licensor or its representatives, including but not limited to
   communication on electronic mailing lists, source code control systems,
   and issue tracking systems that are managed by, or on behalf of, the
   Licensor for the purpose of discussing and improving the Work, but
   excluding communication that is conspicuously marked or otherwise
   designated in writing by the copyright owner as "Not a Contribution."

   "Contributor" shall mean Licensor and any individual or Legal Entity
   on behalf of whom a Contribution has been received by Licensor and
   subsequently incorporated within the Work.

2. Grant of Copyright License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   copyright license to reproduce, prepare Derivative Works of,
   publicly display, publicly perform, sublicense, and distribute the
   Work and such Derivative Works in Source or Object form.

3. Grant of Patent License. Subject to the terms and conditions of
   this License, each Contributor hereby grants to You a perpetual,
   worldwide, non-exclusive, no-charge, royalty-free, irrevocable
   (except as stated in this section) patent license to make, have made,
   use, offer to sell, sell, import, and otherwise transfer the Work,
   where such license applies only to those patent claims licensable
   by such Contributor that are necessarily infringed by their
   Contribution(s) alone or by combination of their Contribution(s)
   with the Work to which such Contribution(s) was submitted. If You
   institute patent litigation against any entity (including a
   cross-claim or counterclaim in a lawsuit) alleging that the Work
   or a Contribution incorporated within the Work constitutes direct
   or contributory patent infringement, then any patent licenses
   granted to You under this License for that Work shall terminate
   as of the date such litigation is filed.

4. Redistribution. You may reproduce and distribute copies of the
   Work or Derivative Works thereof in any medium, with or without
   modifications, and in Source or Object form, provided that You
   meet the following conditions:

   (a) You must give any other recipients of the Work or
       Derivative Works a copy of this License; and

   (b) You must cause any modified files to carry prominent notices
       stating that You changed the files; and

   (c) You must retain, in the Source form of any Derivative Works
       that You distribute, all copyright, patent, trademark, and
       attribution notices from the Source form of the Work,
       excluding those notices that do not pertain to any part of
       the Derivative Works; and

   (d) If the Work includes a "NOTICE" text file as part of its
       distribution, then any Derivative Works that You distribute must
       include a readable copy of the attribution notices contained
       within such NOTICE file, excluding those notices that do not
       pertain to any part of the Derivative Works, in at least one
       of the following places: within a NOTICE text file distributed
       as part of the Derivative Works; within the Source form or
       documentation, if provided along with the Derivative Works; or,
       within a display generated by the Derivative Works, if and
       wherever such third-party notices normally appear. The contents
       of the NOTICE file are for informational purposes only and
       do not modify the License. You may add Your own attribution
       notices within Derivative Works that You distribute, alongside
       or as an addendum to the NOTICE text from the Work, provided
       that such additional attribution notices cannot be construed
       as modifying the License.

   You may add Your own copyright statement to Your modifications and
   may provide additional or different license terms and conditions
   for use, reproduction, or distribution of Your modifications, or
   for any such Derivative Works as a whole, provided Your use,
   reproduction, and distribution of the Work otherwise complies with
   the conditions stated in this License.

5. Submission of Contributions. Unless You explicitly state otherwise,
   any Contribution intentionally submitted for inclusion in the Work
   by You to the Licensor shall be under the terms and conditions of
   this License, without any additional terms or conditions.
   Notwithstanding the above, nothing herein shall supersede or modify
   the terms of any separate license agreement you may have executed
   with Licensor regarding such Contributions.

6. Trademarks. This License does not grant permission to use the trade
   names, trademarks, service marks, or product names of the Licensor,
   except as required for reasonable and customary use in describing the
   origin of the Work and reproducing the content of the NOTICE file.

7. Disclaimer of Warranty. Unless required by applicable law or
   agreed to in writing, Licensor provides the Work (and each
   Contributor provides its Contributions) on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
   implied, including, without limitation, any warranties or conditions
   of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
   PARTICULAR PURPOSE. You are solely responsible for determining the
   appropriateness of using or redistributing the Work and assume any
   risks associated with Your exercise of permissions under this License.

8. Limitation of Liability. In no event and under no legal theory,
   whether in tort (including negligence), contract, or otherwise,
   unless required by applicable law (such as deliberate and grossly
   negligent acts) or agreed to in writing, shall any Contributor be
   liable to You for damages, including any direct, indirect, special,
   incidental, or consequential damages of any character arising as a
   result of this License or out of the use or inability to use the
   Work (including but not limited to damages for loss of goodwill,
   work stoppage, computer failure or malfunction, or any and all
   other commercial damages or losses), even if such Contributor
   has been advised of the possibility of such damages.

9. Accepting Warranty or Additional Liability. While redistributing
   the Work or Derivative Works thereof, You may choose to offer,
   and charge a fee for, acceptance of support, warranty, indemnity,
   or other liability obligations and/or rights consistent with this
   License. However, in accepting such obligations, You may act only
   on Your own behalf and on Your sole responsibility, not on behalf
   of any other Contributor, and only if You agree to indemnify,
   defend, and hold each Contributor harmless for any liability
   incurred by, or claims asserted against, such Contributor by reason
   of your accepting any such warranty or additional liability.

END OF TERMS AND CONDITIONS

APPENDIX: How to apply the Apache License to your work.

   To apply the Apache License to your work, attach the following
   boilerplate notice, with the fields enclosed by brackets "[]"
   replaced with your own identifying information. (Don't include
   the brackets!)  The text should be enclosed in the appropriate
   comment syntax for the file format. We also recommend that a
   file or class name and description of purpose be included on the
   same "printed page" as the copyright notice for easier
   identification within third-party archives.

Copyright [yyyy] [name of copyright owner]

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

	http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

### Unicode License v3

Applies to: `icu_collections 2.2.0`, `icu_locale_core 2.2.0`, `icu_normalizer 2.2.0`, `icu_normalizer_data 2.2.0`, `icu_properties 2.2.0`, `icu_properties_data 2.2.0`, `icu_provider 2.2.0`, `litemap 0.8.2`, `potential_utf 0.1.5`, `tinystr 0.8.3`, `writeable 0.6.3`, `yoke 0.8.3`, `zerofrom 0.1.8`, `zerotrie 0.2.4`, `zerovec 0.11.6`.

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2020-2024 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.

SPDX-License-Identifier: Unicode-3.0

—

Portions of ICU4X may have been adapted from ICU4C and/or ICU4J.
ICU 1.8.1 to ICU 57.1 © 1995-2016 International Business Machines Corporation and others.
```

### Unicode License v3 for unicode-ident

The `unicode-ident` package requires Unicode-3.0 in addition to its Apache-2.0 option.

```text
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 1991-2023 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.
```

### Boost Software License 1.0

Applies to: `clipboard-win 5.4.1`, `error-code 3.3.2`.

```text
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization
obtaining a copy of the software and accompanying documentation covered by
this license (the "Software") to use, reproduce, display, distribute,
execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to
do so, all subject to the following:

The copyright notices in the Software and this entire statement, including
the above license grant, this restriction and the following disclaimer,
must be included in all copies of the Software, in whole or in part, and
all derivative works of the Software, unless such copies or derivative
works are solely in the form of machine-executable object code generated by
a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT
SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE
FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### The Unlicense

Applies under the Unlicense option to: `byteorder-lite 0.1.0`, `memchr 2.8.3`, `termcolor 1.4.1`, `winapi-util 0.1.11`.

```text
This is free and unencumbered software released into the public domain.

Anyone is free to copy, modify, publish, use, compile, sell, or
distribute this software, either in source code form or as a compiled
binary, for any purpose, commercial or non-commercial, and by any
means.

In jurisdictions that recognize copyright laws, the author or authors
of this software dedicate any and all copyright interest in the
software to the public domain. We make this dedication for the benefit
of the public at large and to the detriment of our heirs and
successors. We intend this dedication to be an overt act of
relinquishment in perpetuity of all present and future rights to this
software under copyright law.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR
OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE,
ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.

For more information, please refer to <http://unlicense.org/>
```

### zlib License

Applies to: `foldhash 0.2.0`.

```text
Copyright (c) 2024 Orson Peters

This software is provided 'as-is', without any express or implied warranty. In
no event will the authors be held liable for any damages arising from the use of
this software.

Permission is granted to anyone to use this software for any purpose, including
commercial applications, and to alter it and redistribute it freely, subject to
the following restrictions:

1. The origin of this software must not be misrepresented; you must not claim
    that you wrote the original software. If you use this software in a product,
    an acknowledgment in the product documentation would be appreciated but is
    not required.

2. Altered source versions must be plainly marked as such, and must not be
    misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```

### ISC License

Applies to: `libloading 0.8.9`.

```text
Copyright © 2015, Simonas Kazlauskas

Permission to use, copy, modify, and/or distribute this software for any purpose with or without
fee is hereby granted, provided that the above copyright notice and this permission notice appear
in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
```

### MIT-licensed package notices

The following package-specific MIT texts preserve their upstream copyright notices. `dpi 0.1.2` requires its MIT material in addition to Apache-2.0.

#### `dpi 0.1.2`

Exact package source: `dpi 0.1.2/LICENSE-LIBM-MIT`, which the package README identifies as the MIT license for its feature-gated `src/libm.rs`. The text below is reproduced verbatim from that package file.

```text
rust-lang/libm as a whole is available for use under the MIT license:

------------------------------------------------------------------------------
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
------------------------------------------------------------------------------

This Rust library contains the following copyrights:

    Copyright (c) 2018 Jorge Aparicio

Portions of this software are derived from third-party works licensed under
terms compatible with the above MIT license:

* musl libc https://www.musl-libc.org/. This library contains the following
  copyright:

      Copyright © 2005-2020 Rich Felker, et al.

* The CORE-MATH project https://core-math.gitlabpages.inria.fr/. CORE-MATH
  routines are available under the MIT license on a per-file basis.

The musl libc COPYRIGHT file also includes the following notice relevant to
math portions of the library:

------------------------------------------------------------------------------
Much of the math library code (src/math/* and src/complex/*) is
Copyright © 1993,2004 Sun Microsystems or
Copyright © 2003-2011 David Schultz or
Copyright © 2003-2009 Steven G. Kargl or
Copyright © 2003-2009 Bruce D. Evans or
Copyright © 2008 Stephen L. Moshier or
Copyright © 2017-2018 Arm Limited
and labelled as such in comments in the individual source files. All
have been licensed under extremely permissive terms.
------------------------------------------------------------------------------
```

#### `fax 0.2.7`

```text
Copyright © 2021 The pdf-rs contributers.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the “Software”), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

#### `glutin-winit 0.5.0`

```text
Copyright © 2022 Kirill Chibisov

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the “Software”), to deal
in the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to do
so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED “AS IS”, WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

#### `harfrust 0.12.0`

```text
The MIT License (MIT)

Copyright (c) HarfBuzz developers
Copyright (c) 2020 Yevhenii Reizner

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

#### `libm 0.2.16`

```text
rust-lang/libm as a whole is available for use under the MIT license:

------------------------------------------------------------------------------
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
------------------------------------------------------------------------------

As a contributor, you agree that your code can be used under either the MIT
license or the Apache-2.0 license:

------------------------------------------------------------------------------
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
------------------------------------------------------------------------------

This Rust library contains the following copyrights:

    Copyright (c) 2018 Jorge Aparicio

Portions of this software are derived from third-party works licensed under
terms compatible with the above MIT license:

* musl libc https://www.musl-libc.org/. This library contains the following
  copyright:

      Copyright © 2005-2020 Rich Felker, et al.

* The CORE-MATH project https://core-math.gitlabpages.inria.fr/. CORE-MATH
  routines are available under the MIT license on a per-file basis.

The musl libc COPYRIGHT file also includes the following notice relevant to
math portions of the library:

------------------------------------------------------------------------------
Much of the math library code (src/math/* and src/complex/*) is
Copyright © 1993,2004 Sun Microsystems or
Copyright © 2003-2011 David Schultz or
Copyright © 2003-2009 Steven G. Kargl or
Copyright © 2003-2009 Bruce D. Evans or
Copyright © 2008 Stephen L. Moshier or
Copyright © 2017-2018 Arm Limited
and labelled as such in comments in the individual source files. All
have been licensed under extremely permissive terms.
------------------------------------------------------------------------------

Copyright notices are retained in src/* files where relevant.
```

#### `ordered-float 5.3.0`

```text
Copyright (c) 2015 Jonathan Reem

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

#### `simd-adler32 0.3.10`

```text
MIT License

Copyright (c) [2021] [Marvin Countryman]

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

#### `tiff 0.11.3`

```text
MIT License

Copyright (c) 2018 PistonDevelopers

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

#### `tracing 0.1.44`, `tracing-core 0.1.36`

```text
Copyright (c) 2019 Tokio Contributors

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```

### Additional upstream copyright and attribution notices

#### `euclid 0.22.14`

```text
Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
<LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
option. All files in the project carrying such notice may not be
copied, modified, or distributed except according to those terms.
```

#### `unicode-segmentation 1.13.3`

```text
Licensed under the Apache License, Version 2.0
<LICENSE-APACHE or
http://www.apache.org/licenses/LICENSE-2.0> or the MIT
license <LICENSE-MIT or http://opensource.org/licenses/MIT>,
at your option. All files in the project carrying such
notice may not be copied, modified, or distributed except
according to those terms.
```

#### `unicode-width 0.2.2`

```text
Licensed under the Apache License, Version 2.0
<LICENSE-APACHE or
http://www.apache.org/licenses/LICENSE-2.0> or the MIT
license <LICENSE-MIT or http://opensource.org/licenses/MIT>,
at your option. All files in the project carrying such
notice may not be copied, modified, or distributed except
according to those terms.
```

#### `utf8_iter 1.0.4`

```text
Copyright Mozilla Foundation

Licensed under the Apache License (Version 2.0), or the MIT license,
(the "Licenses") at your option. You may not use this file except in
compliance with one of the Licenses. You may obtain copies of the
Licenses at:

   https://www.apache.org/licenses/LICENSE-2.0
   https://opensource.org/licenses/MIT

Unless required by applicable law or agreed to in writing, software
distributed under the Licenses is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the Licenses for the specific language governing permissions and
limitations under the Licenses.

--

Test code is dedicated to the Public Domain when so designated (see
the individual files for PD/CC0-dedicated sections).

--

The implementation for Utf8CharIndices was adapted from the
CharIndices implementation of the Rust standard library at revision
ab32548539ec38a939c1b58599249f3b54130026
(https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/library/core/src/str/iter.rs).

Excerpt from https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/COPYRIGHT ,
which refers to
https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/LICENSE-APACHE
and
https://github.com/rust-lang/rust/blob/ab32548539ec38a939c1b58599249f3b54130026/LICENSE-MIT
:

For full authorship information, see the version control history or
https://thanks.rust-lang.org

Except as otherwise noted (below and/or in individual files), Rust is
licensed under the Apache License, Version 2.0 <LICENSE-APACHE> or
<http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
<LICENSE-MIT> or <http://opensource.org/licenses/MIT>, at your option.
```

### epaint_default_fonts bundled font notices

`epaint_default_fonts 0.36.1` is part of the runtime dependency graph. Its package metadata is `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`; Apollo uses Apache-2.0 for the crate code. The crate also embeds font resources carrying the following upstream notices. The OFL-1.1 text in this section also covers Apollo's repository-embedded IBM Plex Sans Regular and Fira Code Regular assets.

#### Hack font notice

```text
The work in the Hack project is Copyright 2018 Source Foundry Authors and licensed under the MIT License

The work in the DejaVu project was committed to the public domain.

Bitstream Vera Sans Mono Copyright 2003 Bitstream Inc. and licensed under the Bitstream Vera License with Reserved Font Names "Bitstream" and "Vera"
MIT License

Copyright (c) 2018 Source Foundry Authors

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
BITSTREAM VERA LICENSE

Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved. Bitstream Vera is a trademark of Bitstream, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy of the fonts accompanying this license ("Fonts") and associated documentation files (the "Font Software"), to reproduce and distribute the Font Software, including without limitation the rights to use, copy, merge, publish, distribute, and/or sell copies of the Font Software, and to permit persons to whom the Font Software is furnished to do so, subject to the following conditions:

The above copyright and trademark notices and this permission notice shall be included in all copies of one or more of the Font Software typefaces.

The Font Software may be modified, altered, or added to, and in particular the designs of glyphs or characters in the Fonts may be modified and additional glyphs or characters may be added to the Fonts, only if the fonts are renamed to names not containing either the words "Bitstream" or the word "Vera".

This License becomes null and void to the extent applicable to Fonts or Font Software that has been modified and is distributed under the "Bitstream Vera" names.

The Font Software may be sold as part of a larger software package but no copy of one or more of the Font Software typefaces may be sold by itself.

THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL BITSTREAM OR THE GNOME FOUNDATION BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM OTHER DEALINGS IN THE FONT SOFTWARE.

Except as contained in this notice, the names of Gnome, the Gnome Foundation, and Bitstream Inc., shall not be used in advertising or otherwise to promote the sale, use or other dealings in this Font Software without prior written authorization from the Gnome Foundation or Bitstream Inc., respectively. For further information, contact: fonts at gnome dot org.
```

#### SIL Open Font License 1.1

```text
This Font Software is licensed under the SIL Open Font License,
Version 1.1.

This license is copied below, and is also available with a FAQ at:
http://scripts.sil.org/OFL

-----------------------------------------------------------
SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007
-----------------------------------------------------------

PREAMBLE
The goals of the Open Font License (OFL) are to stimulate worldwide
development of collaborative font projects, to support the font
creation efforts of academic and linguistic communities, and to
provide a free and open framework in which fonts may be shared and
improved in partnership with others.

The OFL allows the licensed fonts to be used, studied, modified and
redistributed freely as long as they are not sold by themselves. The
fonts, including any derivative works, can be bundled, embedded,
redistributed and/or sold with any software provided that any reserved
names are not used by derivative works. The fonts and derivatives,
however, cannot be released under any other type of license. The
requirement for fonts to remain under this license does not apply to
any document created using the fonts or their derivatives.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this license and clearly marked as such. This may
include source files, build scripts and documentation.

"Reserved Font Name" refers to any names specified as such after the
copyright statement(s).

"Original Version" refers to the collection of Font Software
components as distributed by the Copyright Holder(s).

"Modified Version" refers to any derivative made by adding to,
deleting, or substituting -- in part or in whole -- any of the
components of the Original Version, by changing formats or by porting
the Font Software to a new environment.

"Author" refers to any designer, engineer, programmer, technical
writer or other person who contributed to the Font Software.

PERMISSION & CONDITIONS
Permission is hereby granted, free of charge, to any person obtaining
a copy of the Font Software, to use, study, copy, merge, embed,
modify, redistribute, and sell modified and unmodified copies of the
Font Software, subject to the following conditions:

1) Neither the Font Software nor any of its individual components, in
Original or Modified Versions, may be sold by itself.

2) Original or Modified Versions of the Font Software may be bundled,
redistributed and/or sold with any software, provided that each copy
contains the above copyright notice and this license. These can be
included either as stand-alone text files, human-readable headers or
in the appropriate machine-readable metadata fields within text or
binary files as long as those fields can be easily viewed by the user.

3) No Modified Version of the Font Software may use the Reserved Font
Name(s) unless explicit written permission is granted by the
corresponding Copyright Holder. This restriction only applies to the
primary font name as presented to the users.

4) The name(s) of the Copyright Holder(s) or the Author(s) of the Font
Software shall not be used to promote, endorse or advertise any
Modified Version, except to acknowledge the contribution(s) of the
Copyright Holder(s) and the Author(s) or with their explicit written
permission.

5) The Font Software, modified or unmodified, in part or in whole,
must be distributed entirely under this license, and must not be
distributed under any other license. The requirement for fonts to
remain under this license does not apply to any document created using
the Font Software.

TERMINATION
This license becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT
OF COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM
OTHER DEALINGS IN THE FONT SOFTWARE.
```

#### Ubuntu Font Licence 1.0

```text
-------------------------------
UBUNTU FONT LICENCE Version 1.0
-------------------------------

PREAMBLE
This licence allows the licensed fonts to be used, studied, modified and
redistributed freely. The fonts, including any derivative works, can be
bundled, embedded, and redistributed provided the terms of this licence
are met. The fonts and derivatives, however, cannot be released under
any other licence. The requirement for fonts to remain under this
licence does not require any document created using the fonts or their
derivatives to be published under this licence, as long as the primary
purpose of the document is not to be a vehicle for the distribution of
the fonts.

DEFINITIONS
"Font Software" refers to the set of files released by the Copyright
Holder(s) under this licence and clearly marked as such. This may
include source files, build scripts and documentation.

"Original Version" refers to the collection of Font Software components
as received under this licence.

"Modified Version" refers to any derivative made by adding to, deleting,
or substituting -- in part or in whole -- any of the components of the
Original Version, by changing formats or by porting the Font Software to
a new environment.

"Copyright Holder(s)" refers to all individuals and companies who have a
copyright ownership of the Font Software.

"Substantially Changed" refers to Modified Versions which can be easily
identified as dissimilar to the Font Software by users of the Font
Software comparing the Original Version with the Modified Version.

To "Propagate" a work means to do anything with it that, without
permission, would make you directly or secondarily liable for
infringement under applicable copyright law, except executing it on a
computer or modifying a private copy. Propagation includes copying,
distribution (with or without modification and with or without charging
a redistribution fee), making available to the public, and in some
countries other activities as well.

PERMISSION & CONDITIONS
This licence does not grant any rights under trademark law and all such
rights are reserved.

Permission is hereby granted, free of charge, to any person obtaining a
copy of the Font Software, to propagate the Font Software, subject to
the below conditions:

1) Each copy of the Font Software must contain the above copyright
notice and this licence. These can be included either as stand-alone
text files, human-readable headers or in the appropriate machine-
readable metadata fields within text or binary files as long as those
fields can be easily viewed by the user.

2) The font name complies with the following:
(a) The Original Version must retain its name, unmodified.
(b) Modified Versions which are Substantially Changed must be renamed to
avoid use of the name of the Original Version or similar names entirely.
(c) Modified Versions which are not Substantially Changed must be
renamed to both (i) retain the name of the Original Version and (ii) add
additional naming elements to distinguish the Modified Version from the
Original Version. The name of such Modified Versions must be the name of
the Original Version, with "derivative X" where X represents the name of
the new work, appended to that name.

3) The name(s) of the Copyright Holder(s) and any contributor to the
Font Software shall not be used to promote, endorse or advertise any
Modified Version, except (i) as required by this licence, (ii) to
acknowledge the contribution(s) of the Copyright Holder(s) or (iii) with
their explicit written permission.

4) The Font Software, modified or unmodified, in part or in whole, must
be distributed entirely under this licence, and must not be distributed
under any other licence. The requirement for fonts to remain under this
licence does not affect any document created using the Font Software,
except any version of the Font Software extracted from a document
created using the Font Software may only be distributed under this
licence.

TERMINATION
This licence becomes null and void if any of the above conditions are
not met.

DISCLAIMER
THE FONT SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO ANY WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
COPYRIGHT, PATENT, TRADEMARK, OR OTHER RIGHT. IN NO EVENT SHALL THE
COPYRIGHT HOLDER BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
INCLUDING ANY GENERAL, SPECIAL, INDIRECT, INCIDENTAL, OR CONSEQUENTIAL
DAMAGES, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF THE USE OR INABILITY TO USE THE FONT SOFTWARE OR FROM OTHER
DEALINGS IN THE FONT SOFTWARE.
```

#### Emoji icon font MIT notice

```text
MIT License

Copyright (c) 2014 John Slegers

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
