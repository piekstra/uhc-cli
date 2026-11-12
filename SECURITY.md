# Security

Report a vulnerability privately through GitHub's "Report a vulnerability"
(Security tab), not in an issue.

## What uhc touches

- Reads the PDFs in the configured import folder (one level of subfolders,
  symlinks not followed, 50 MB per file) and runs the local `pdftotext` on
  each (60-second limit, 20 MB of text). No shell is involved.
- Writes only where asked: `documents download` (owner-only 0600 files,
  never replacing one without `--overwrite`) and `records export -o`
  (synthetic statements only).
- Contacts no server except GitHub, for `self-update`.
- Stores no credential. The config file holds the import folder's path.

## What uhc never does

- Writes to UnitedHealthcare.
- Prints names, amounts or ids in `import check`, which exists to be shared.
- Sends a statement or its contents anywhere: the output goes to stdout or a
  file you name.
