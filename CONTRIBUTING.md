# Contributing

1. `make verify` must pass: fmt-check, clippy with warnings denied, the
   tests, the smoke test. Install poppler (`pdftotext`) to run the
   end-to-end tests locally.
2. Fixtures are synthetic. A new statement layout becomes a new file in
   `tests/fixtures/eob/` built from the shapes in `tests/fixtures/README.md`,
   never from a real statement. Open it with the synthetic marker line.
3. No personal data anywhere: code, docs, commit messages, pull requests,
   issues. `tests/no_personal_data.rs` enforces the shapes; review the rest.
4. A parser change comes with a fixture that shows the layout it reads, and
   an `Issue` code for each way that layout can fail.
5. Squash merges only, with an approval and green CI.
