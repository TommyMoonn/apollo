# Apollo font assets

The GPUI interface uses Geist Variable (Sans), from Vercel's official `geist` npm package, pinned to `1.7.2`. Its source is [vercel/geist-font](https://github.com/vercel/geist-font), licensed under SIL OFL 1.1.

The source file must be present at `assets/fonts/Geist-Variable.ttf`. From the repository root, obtain the pinned asset once:

```powershell
Invoke-WebRequest -Uri 'https://unpkg.com/geist@1.7.2/dist/fonts/geist-sans/Geist-Variable.ttf' -OutFile 'assets/fonts/Geist-Variable.ttf'
```

Commit the original, unmodified file with the phase changes. Verify its SHA-256 against the pinned package before distribution, and record the checksum in [third-party notices](../../THIRD_PARTY_NOTICES.md). The handoff does not contain the font binary; acquisition and native font verification are outstanding. No build or runtime download is performed. The Windows build embeds the font using `include_bytes!`, registers it with GPUI at startup and uses Segoe UI and DirectWrite system fallback for missing glyphs. The existing installer and portable ZIP distribute the embedded resource via `Apollo.exe`, and both include the third-party notices and OFL text.

## Retained egui assets

- `IBMPlexSans-Regular.ttf`: IBM Plex Sans Regular 3.005, [IBM Plex](https://github.com/IBM/plex), OFL 1.1.
- `FiraCode-Regular.ttf`: Fira Code Regular 6.002, [Fira Code](https://github.com/tonsky/FiraCode), OFL 1.1.

Both remain required by `src/ui/fonts.rs` and the retained egui views and regression tests. Remove them and the legacy registration only when the final egui consumer is removed. Their copyright notices, checksums and license text are in [third-party notices](../../THIRD_PARTY_NOTICES.md). Switzer is not bundled.
