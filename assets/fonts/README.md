# Apollo font assets

GPUI uses the two repository-embedded fonts below. Startup registers both assets through the same text system; the executable does not depend on installed copies or network access. The installer and portable ZIP distribute them inside `Apollo.exe` and include the third-party notices.

| Asset | Embedded version | Role | License |
| --- | --- | --- | --- |
| `Geist-Variable.ttf` | 1.800 | Geist Variable UI text (400, 500, 600) | OFL-1.1 |
| `FiraCode-Regular.ttf` | 6.002 | Lowercase `apollo` wordmark (400) | OFL-1.1 |
| `IBMPlexSans-Regular.ttf` | 3.005 | Retained egui views and tests | OFL-1.1 |

The Geist file's `wght` axis spans 100 to 900, with a default of 400. Its documented acquisition source is [the official Geist npm package 1.7.2](https://www.npmjs.com/package/geist/v/1.7.2); the font's internal version is separate from the package version. To restore the original asset from the repository's pinned acquisition URL:

```powershell
Invoke-WebRequest -Uri 'https://unpkg.com/geist@1.7.2/dist/fonts/geist-sans/Geist-Variable.ttf' -OutFile 'assets/fonts/Geist-Variable.ttf'
```

Verify replacement files against the SHA-256 values in [third-party notices](../../THIRD_PARTY_NOTICES.md). The current embedded files' metadata and hashes have been inspected; an independent comparison of Geist against the upstream npm package remains pending. No font binaries were modified in U01.7.

Fira Code ([upstream](https://github.com/tonsky/FiraCode)) must remain embedded after the egui migration. IBM Plex Sans ([upstream](https://github.com/IBM/plex)) can be removed only after its last legacy consumer is replaced. All three assets carry SIL OFL 1.1; copyright notices and the license text are in the third-party notices. Switzer is not bundled.

See [the UI design guide](../../docs/DESIGN.md) for typography roles, fallback behavior and native acceptance. Windows face selection and packaged offline rendering remain unverified.
