# Apollo font assets

Apollo's typography owner expects these repository-controlled font files in this directory:

```text
IBMPlexSans-Regular.ttf
FiraCode-Regular.ttf
```

Use the upstream regular TrueType releases:

- IBM Plex Sans Regular, embedded version `3.005`, from the [IBM Plex project](https://github.com/IBM/plex). It is licensed under `OFL-1.1`; the embedded asset's copyright, trademark, checksum, and license text are recorded in [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md#ibm-plex-sans-regular).
- Fira Code Regular, embedded version `6.002`, from the [Fira Code project](https://github.com/tonsky/FiraCode). It is licensed under `OFL-1.1`; the embedded asset's copyright, trademark, checksum, and license text are recorded in [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md#fira-code-regular).

The versions above come from the metadata embedded in the repository font files. Both files are application resources, not operating-system font dependencies. Apollo deliberately fails font setup when either required asset is absent instead of silently changing typography based on fonts installed on the machine.

Do not rename the files without updating `src/ui/fonts.rs`.
