# From a statement to FHIR

`uhc records export` maps what the EOB statements print into FHIR R4
(4.0.1). It follows CARIN Blue Button 2.0's element choices and code systems
where a statement carries the value, and does not claim the CARIN profiles
(`meta.profile`): a statement prints no claim type, diagnosis or procedure
codes, which the profiles require.

Every resource carries `meta.source` `uhc:eob-pdf` and the tag
`https://github.com/piekstra/uhc-cli/fhir/tag|mapped-from-eob-statement`.

## Identity

| Thing | Id | Why |
| --- | --- | --- |
| Member | `m` + 12 hex of SHA-256(member id \| first name) | one per person on the plan; the same across statements; no name in it |
| Family | `family-` + 12 hex of SHA-256(member id \| "") | the family rows of the plan balances |
| Coverage | `cov-` + 12 hex of SHA-256(member id \| member \| group) | one per member and group |
| Claim | `c` + the claim number | stable across reprocessing |
| Claim without a number | `x` + the first 10 hex of the document id + its index | a damaged statement |
| Statement | first 16 hex of the PDF's SHA-256 | the same file saved twice is one statement |

A claim printed on more than one statement (reprocessed) is the latest
statement's version; `versions` in `claims list` counts them.

## Resources

| Resource | From | Notes |
| --- | --- | --- |
| `Organization/uhc` | — | `type` `pay` |
| `Patient` | the header and "Claim detail for" | `identifier` the member id (`MB`); `name.given` the first name; `name.text` the full name when a statement is addressed to that member |
| `Coverage` | the header | `subscriberId`, `relationship` (EE self, SP spouse, DP common, CH child), `class` group number and name, `payor` |
| `ExplanationOfBenefit` | a "Claim detail for" block | below |
| `DocumentReference` | the statement PDF | `type.text` "Explanation of Benefits", `context.related` its claims, `context.period` the statement's service period |
| `Binary` | the statement PDF | base64; the `DocumentReference` points at it with `content[].attachment.url` `Binary/<id>` |

### ExplanationOfBenefit

| Element | From |
| --- | --- |
| `identifier` (`uc`) | Claim number |
| `type.text` | "Medical claim": the statement prints no claim type, so there is no `claim-type` code |
| `billablePeriod` | the first service line's date to the last |
| `created` | the statement date |
| `provider.display` | Provider |
| `supportingInfo` | `info` with `valueReference` to the statement's `DocumentReference`; `performingnetworkcontractingstatus` (contracted / non-contracted) from Status; `patientaccountnumber` |
| `insurance` | the member's `Coverage` |
| `item[]` | each service line: `productOrService.text`, `servicedDate` or `servicedPeriod`, `noteNumber` for its codes, `net` the billed amount, `adjudication` |
| `total[]` | the "Total amount" row |
| `processNote[]` | "XX -- text", numbered in code order |
| `benefitBalance[]` | the plan balances on the claim's statement: one per network (in, out) and unit (individual, family), term annual; `financial[]` deductible and out-of-pocket limit with `allowedMoney` (the maximum) and `usedMoney` (progress so far) |

The nine amounts and their adjudication categories, each coding with its
code system's display text:

| Statement column | Category |
| --- | --- |
| Amount billed | `submitted` (HL7) |
| Your plan discount / Saved | `discount` (CARIN) |
| Amount allowed | `eligible` (HL7) |
| Plan paid | `benefit` (HL7) |
| Deductible | `deductible` (HL7) |
| Copay | `copay` (HL7) |
| Coinsurance | `coinsurance` (CARIN) |
| Not covered | `noncovered` (CARIN) |
| You owe | `memberliability` (CARIN) |

The out-of-pocket limit has no HL7 benefit-type code, so it carries
`https://github.com/piekstra/uhc-cli/fhir/benefit-type|out-of-pocket-maximum`
and the text "Out-of-pocket limit".

## The stream

`records-stream/v1`: a header with `method` `portal`, the organization
`{id: "uhc", name: "UnitedHealthcare", fhir_base: "https://member.uhc.com/myuhc", vendor: "uhc"}`
and the patient `{id, organization, label, relationship}`; then the
resources in the order above; then `records-stream-end/v1` with counts. A
statement read with issues adds a `skipped[]` entry with code
`statement_issues` naming the document and the issue codes.
