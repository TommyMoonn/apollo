# Release notes

Each `vX.Y.Z.md` is the complete GitHub release body for that version. Write the note in UTF-8, with `## Downloads` followed by `## Changes`. Use the release version in both asset URLs; the installer and portable ZIP filenames are fixed. Use [v0.7.0.md](v0.7.0.md) as the example.

Describe changes users can see or benefit from. Keep implementation history in [CHANGELOG.md](../CHANGELOG.md). Include at least one substantive change in a bullet list, and remove placeholders before validation. Do not add a title or a separate version/date header. The version comes from the filename and the date from the matching changelog entry.

Validate the complete candidate with `just check-release --tag vX.Y.Z --require-changelog`, substituting the actual version. Keep working drafts in `.plans/` until they are ready. Earlier published notes do not require a backfill. Preserve published history rather than revising it to match a draft.

See [Releasing](../docs/RELEASING.md) for the release procedure.
