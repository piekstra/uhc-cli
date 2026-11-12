# Test fixtures

Every file here is **synthetic**. Nothing was captured from UnitedHealthcare
or from a real member, and no real statement was used to write them.

- `eob/*.txt` are what `pdftotext -layout` makes of a UnitedHealthcare
  Explanation of Benefits statement, written by hand after the layout of UHC's
  public explainer (`understanding-your-eob-flier.pdf`, see `docs/api.md`). Each
  starts with `SYNTHETIC TEST DATA`, the marker `uhc` treats as synthetic.
- `damaged-statement.txt` is deliberately incomplete: no member id, no claim
  number, a balances row short of values, a service line with eight amounts
  and a total that does not add up. It exercises the reader's issue codes.
- `premium-notice.txt` is a UHC letter that is not an EOB.

Dummy shapes, enforced by `tests/no_personal_data.rs`:

| Field | Shape |
|---|---|
| Member ID | `SYNM` + 6 digits |
| Group number | `SYNG` + 4 digits |
| Claim number | `SYNC` + 8 digits |
| Patient account number | `SYNA` + 4 digits |
| People | `ALEX`, `SAM`, `JORDAN`, surname `EXAMPLE` |
| Providers, employer | `EXAMPLE …`, `SAMPLE …`, `TEST …` |
| Phone numbers | `555-01xx` |
