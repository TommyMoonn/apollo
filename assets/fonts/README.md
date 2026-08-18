# Apollo font assets

Apollo's typography owner expects these repository-controlled font files in this directory:

```text
IBMPlexSans-Regular.ttf
FiraCode-Regular.ttf
```

Use the upstream regular TrueType releases:

- IBM Plex Sans Regular from the IBM Plex project.
- Fira Code Regular from the Fira Code project.

Both files are application resources, not operating-system font dependencies. Apollo deliberately
fails font setup when either required asset is absent instead of silently changing typography based
on fonts installed on the machine.

Do not rename the files without updating `src/ui/fonts.rs`.
