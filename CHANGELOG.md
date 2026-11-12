# Changelog

## v0.1.0

The first release. `uhc` reads the UnitedHealthcare Explanation of Benefits
statement PDFs downloaded from myuhc.com into a folder (`docs/api.md` has
why: UHC's Patient Access API does not serve employer plans).

- `records export --patient <id> --ndjson`: `records-stream/v1` with
  `Patient`, `Organization`, `Coverage`, one `ExplanationOfBenefit` per
  claim and each statement as `DocumentReference` + `Binary`
  (`docs/fhir.md`). `--since D` is what changed since D.
- `patients list`, `claims list`, `claims get`, `accumulators`,
  `documents list`, `documents download [--all]`, `import check`.
- `auth status` reports no sign-in needed; `auth login` is a usage error.
- Built on cli-common v0.31.0 and medical-common v0.1.0: the stream,
  `records-export/v1`, `patient-list/v1` (`PatientContext`) and
  `documents/v1` are the `health-records/v1` profile's. A `documents list`
  id is the export's `DocumentReference` id.
- Read-only: nothing writes to UnitedHealthcare, and nothing is sent
  anywhere. Text comes from the local `pdftotext`.
