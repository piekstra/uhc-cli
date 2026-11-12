# uhc

Unofficial; not affiliated with or endorsed by UnitedHealthcare or Optum.

`uhc` reads a UnitedHealthcare member's coverage, claims, Explanation of
Benefits and deductible and out-of-pocket progress, and hands them to the
household's medical-records system as FHIR (`records-stream/v1`, read by
`medrec import --from uhc`). It is read-only: nothing it does writes to UHC.

It reads the EOB statement PDFs you download from myuhc.com into a folder.
It never contacts UnitedHealthcare. Text comes from the local `pdftotext`
(poppler); nothing is sent anywhere. `docs/api.md` records why: UHC's
Patient Access API serves Medicare, Medicaid, CHIP and marketplace members,
not employer plans, and the member site's terms forbid automated access.

## Install

```sh
brew install poppler            # pdftotext
cargo install --git https://github.com/piekstra/uhc-cli --locked
# or download a release: uhc-<target>.tar.gz, then `uhc self-update` later
```

## Use

```sh
# Download each "Claim EOB" PDF from myuhc.com (Claims & Accounts) into one folder.
uhc config set import_dir ~/Documents/UHC-EOBs

uhc import check                 # counts and issue codes only; safe to share
uhc patients list                # subscriber and dependents
uhc claims list --since 2026-01-01
uhc claims get <claim number>
uhc accumulators --year 2026     # deductible and out-of-pocket progress
uhc documents list
uhc documents download <id> -o ~/Desktop
uhc documents download --all -o ~/Archive/UHC   # re-runs save only new statements

uhc records export --patient <id> --ndjson | medrec import --from uhc
```

Every command takes `--json` and prints one schema-tagged document.

| Command | Output schema |
| --- | --- |
| `patients list` | `patient-list/v1` |
| `records export` | `records-stream/v1` (NDJSON) |
| `claims list` / `claims get` | `claim-list/v1` / `claim/v1` |
| `accumulators` | `accumulator-list/v1` |
| `documents list` / `download` | `document-list/v1` / `document-download/v1`, `document-download-batch/v1` |
| `import check` | `import-check/v1` |
| `auth status` | `auth-status/v1` |
| `info` | `cli-info/v1`, profiles `health-records/v1` and `documents/v1` |

Exit codes: 0 ok, 1 other (pdftotext missing), 2 usage, 4 not found,
5 the statements could not be read (layout changed).

`documents download` never replaces a file without `--overwrite`. For one
document an existing file is a usage error (exit 2). With `--all`, files
already there are left alone and listed in `skipped[]` with code `exists`,
so re-running it into an archive folder saves only the new statements.

### What the export carries

Per member: `Patient`, the payer `Organization`, `Coverage`, one
`ExplanationOfBenefit` per claim (CARIN Blue Button categories for every
amount, processing codes as notes, the plan balances the statement printed as
`benefitBalance`), and each statement PDF as `DocumentReference` + `Binary`.
`docs/fhir.md` has the mapping.

`-o DIR` writes the stream to a file instead, for synthetic statements only;
real records go straight into medrec.

## Try it on synthetic data

```sh
make synth                       # target/synth/*.pdf, from tests/fixtures/eob/
uhc --dir target/synth import check
```

## Development

`make verify` runs fmt-check, clippy, the tests and the smoke test. Tests
are offline and use synthetic statements only (`tests/fixtures/README.md`).
See `AGENTS.md` and `CONTRIBUTING.md`.

## License

MIT or Apache-2.0, at your option.
