# Apollo font assets

Apollo embeds these repository-controlled font files into the application executable at compile time:

```text
IBMPlexSans-Regular.ttf
FiraCode-Regular.ttf
```

Use the upstream regular TrueType releases:

- IBM Plex Sans Regular, embedded version `3.005`, from the [IBM Plex project](https://github.com/IBM/plex). It is licensed under `OFL-1.1`; the embedded asset's copyright, trademark, checksum, and license text are recorded in [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md#ibm-plex-sans-regular).
- Fira Code Regular, embedded version `6.002`, from the [Fira Code project](https://github.com/tonsky/FiraCode). It is licensed under `OFL-1.1`; the embedded asset's copyright, trademark, checksum, and license text are recorded in [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md#fira-code-regular).

Both files are application resources, not operating-system font dependencies or separate runtime payload files. A missing asset is therefore a build-time error rather than an installed-app startup failure, and Apollo does not silently change typography based on fonts installed on the machine.

Do not rename the files without updating `src/ui/fonts.rs`.
