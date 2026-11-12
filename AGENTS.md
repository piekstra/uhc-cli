# AGENTS.md — uhc-cli

`uhc` is a read-only CLI for a UnitedHealthcare member's coverage, claims,
EOBs and deductible and out-of-pocket progress. It reads the EOB statement
PDFs the member downloads from myuhc.com and emits FHIR for the household's
medical-records system (`records-stream/v1`, `medrec import --from uhc`).
It conforms to piekstra-cli/1 and the `health-records/v1` and
`documents/v1` profiles, on the `pk-cli-*` crates from
[cli-common](https://github.com/piekstra/cli-common).

## Layout

| File | Owns |
|---|---|
| `src/cli.rs` | the clap tree |
| `src/app.rs` | the commands, argument checks, output |
| `src/files.rs` | the import folder: which PDFs, `pdftotext` bounded in time and size |
| `src/eob.rs` | the statement parser (UHC's STD-EOB layout) and its issue codes |
| `src/cents.rs` | statement amounts as integer cents |
| `src/model.rs` | statements combined: members, claims (latest version wins), balances |
| `src/fhir.rs` | the CARIN-shaped FHIR mapping (`docs/fhir.md`) |
| `src/stream.rs` | the `records-stream/v1` writer, until `pk-med-records` is pinned |
| `src/synth.rs` | a minimal text PDF writer, for the synthetic statements only |
| `docs/api.md` | how UHC data can be reached, with UHC's own wording, and the traps |

## Medical data rules

These come from the household's plan and are not negotiable.

- The repository carries no personal data, in code, fixtures, docs, commits,
  pull requests or issues. Fixtures are synthetic (`tests/fixtures/README.md`);
  `tests/no_personal_data.rs` scans every tracked file and checks every
  identity field in the fixtures against its dummy shape.
- An agent session is a third-party service. Never read, print or summarize a
  real statement, record or member name in one. Develop and dogfood on the
  synthetic statements (`make synth`). A check against real statements is
  `uhc import check`: counts and issue codes only, never a value.
- `records export -o DIR` refuses statements without the synthetic marker;
  real records go straight into medrec through a pipe.
- No FHIR, SMART or PDF libraries, no aggregators, no cloud OCR or AI.
  `pdftotext` is a local subprocess; general-purpose crates are fine.
- Read-only. Nothing writes to UHC. A write would need SPEC §1.3's
  confirmation gate and a change to this paragraph in the same commit.

## Statements

- A PDF counts as a statement only when it says "Explanation of Benefits",
  names UnitedHealthcare, and has a statement's structure (`eob::is_uhc_eob`).
- Every problem the parser meets is an `Issue` with a code, never a panic and
  never a silent drop. `import check` counts them, the stream's trailer lists
  them as `statement_issues`, and a folder of statements none of which reads
  is exit 5 ("layout may have changed").
- The same claim on several statements is the latest statement's version.
- The same PDF saved twice is one statement (SHA-256).

## Sources

The files source is the only one. The member site source (`auth login
--browser` through `pk-cli-browser`) waits on the household's ruling on
myuhc.com's terms; `docs/api.md` § Route. When it lands: one keychain item
per credential set, a `UHC_KEYCHAIN_SERVICE` test-only override, every
`make` target signs, request logs through `Capture::request_log` (method,
host, path, status; no bodies or queries), shapes only from real responses.

## Tests are offline, always

`cargo test` reads no keychain, contacts nothing, and reads no real
statement:

- `tests/cli_surface.rs`: every `--help`, exit codes, JSON errors, argument
  checks before the folder is read, `NO_COLOR`, a per-test `XDG_CONFIG_HOME`.
- `tests/fixture_shapes.rs`: the parser over `tests/fixtures/eob/*.txt`.
- `tests/e2e.rs`: the binary over the fixtures written as PDFs at test time
  and read back through the real `pdftotext` (skipped where poppler is not
  installed; CI installs it).
- `tests/no_personal_data.rs`.

`make verify` = fmt-check, clippy, test, smoke.
