# UnitedHealthcare: how claims and coverage are reached

UnitedHealthcare (UHC) offers a member three ways to their claims, coverage and
plan balances. Findings are from UHC's and Optum's public pages, read on
2026-11-12 without signing in. Quotes are UHC's or Optum's own wording.

| | Patient Access API (Optum FLEX) | Member site (myuhc.com) | Files the member downloads |
|---|---|---|---|
| What | CARIN Blue Button `ExplanationOfBenefit` and `Coverage`, US Core clinical resources, from 2016-01-01 | Claims, EOB PDFs, ID cards, coverage, deductible and out-of-pocket progress | EOB PDFs; the claims list as a spreadsheet |
| Who it serves | The plans under the CMS Patient Access rule; employer plans most likely not (below) | Every UHC member with a HealthSafe ID | Every member |
| Registration | An organization and an app at `flex.optum.com/portal`, each reviewed (5 business days each) | None | None |
| Terms | "Terms and Conditions (Patient Data API License)" | HealthSafe ID Terms of Use, which forbid software access ("scrape," "crawl," "spider") | Ordinary member use |
| `uhc` source | `api` (SMART standalone launch) | `portal` (`pk-cli-browser`), only if the household accepts the terms | `import` (local files, no network) |

## Route

Open: the household's decision, recorded in the uhc-cli twapp session. The
household's plan is an employer plan. The evidence below says the Patient Access
API serves Medicare, Medicaid, CHIP and ACA marketplace members, so the member
site or the member's own downloads are the realistic sources.

Built in v0.1.0, whatever the decision: the files source. It reads the EOB
statement PDFs the member downloads from the site (layout below) and never
contacts UHC. The member site source waits on the decision.

## Patient Access API (Optum FHIR Layer EXchange, "FLEX")

### Who it serves

UHC's own pages do not name the plan types. They say:

- "The patient access API lets UnitedHealthcare members securely access their
  health data through applications of their choice. ... it meets CMS
  interoperability requirements" (`uhc.com/legal/interoperability-apis`).
- "The Interoperability and Patient Access rule (CMS-9115-F) ... makes it easier
  for you to access and share your health data. ... This shared data is found
  with certain insurance plans." (`uhc.com/legal/share-my-health-data-notice`)
- "UnitedHealthcare gives eligible members secure ways to view certain health
  information using third-party apps" (UHC's IAL2 member guide, a PDF linked
  from the patient access page).

CMS-9115-F binds Medicare Advantage, Medicaid, CHIP and federally facilitated
marketplace plans; employer-sponsored plans are outside it. One health-data
aggregator with production access to this API publishes its supported lines of
business per payer (Flexpa's endpoint directory, entry "United Healthcare",
updated 2026-11-06): Patient Access API "Medicaid, Medicare, ACA (On Exchange),
ACA (Off Exchange), CHIP"; employer plans only through its own member-portal
integration, and only "Fully Funded". It is cited here as evidence only; the
family's rules forbid using aggregators.

Conclusion: an employer-plan member most likely gets no claims through this
API. Only a signed-in attempt with a registered production app settles it.

### Endpoints (fetched unauthenticated, 2026-11-12)

| Purpose | URL |
|---|---|
| FHIR base, production | `https://flex.optum.com/fhir/hsid/R4/` |
| FHIR base, sandbox | `https://flex.optum.com/fhir/sandbox/R4/` |
| SMART configuration | `<base>.well-known/smart-configuration` |
| Authorize | `https://flex.optum.com/authz/<payer>/oauth/authorize` |
| Token | `https://flex.optum.com/authz/<payer>/oauth/token` |
| OpenID configuration | `https://flex.optum.com/authz/<payer>/.well-known/openid-configuration` |
| Public directory (no auth) | `https://flex.optum.com/fhirpublic/R4/` |
| Developer and member portal | `https://flex.optum.com/portal/homepage` |

`<payer>` is `hsid` for all four UHC payers in UHC's table (UnitedHealthcare,
Sierra Health and Life, Rocky Mountain, People's Health) and `sandbox` for the
sandbox. The capability statement reports software `optum-flex-server-default`
4.6.1.7, FHIR 4.0.1.

### Authorization

From the SMART configuration: `launch-standalone`,
`client-confidential-symmetric`, `context-standalone-patient`,
`permission-patient`; `code_challenge_methods_supported: ["S256"]`; grants
`authorization_code` and `client_credentials`. The OpenID configuration lists
`token_endpoint_auth_methods_supported: ["client_secret_post", "private_key_jwt"]`.

UHC's page:

- "A registered application is given a client ID and a client secret. The
  secret should only be used if it can be kept confidential ... For insecure
  implementations, such as mobile apps, PKCE (Proof Key for Code Exchange) is
  available." The token request carries either `client_secret` or
  `code_verifier`.
- Redirects: "The redirect_uri supports any URI protocol ... The https://
  format ... is required for all applications in the production environment
  unless the application is using the Mobile OAuth method for handling
  callbacks." The custom scheme form is reverse-domain
  (`Top-level.domain(TLD).domain-name[.sub-domain][.app_name]`). A loopback
  `http://127.0.0.1` redirect is therefore not accepted in production; a native
  CLI needs either a custom scheme handler or an https page that hands the code
  to a loopback listener.
- Scopes, which "must be explicitly declared; wildcards are not supported":
  `patient/Condition.read`, `patient/Coverage.read`, `patient/Encounter.read`,
  `patient/ExplanationOfBenefit.read`, `patient/Immunization.read`,
  `patient/MedicationDispense.read`, `patient/MedicationRequest.read`,
  `patient/Observation.read`, `patient/Patient.read`,
  `patient/Procedure.read`. The member can decline any scope on the consent
  screen.
- The token response carries `patient`, `expires_in` and `refresh_token`.
  "The access token will be short-lived, typically 5 minutes or less." On
  refresh, "A new refresh token will not be issued": refresh tokens do not
  rotate.
- Sensitive data (substance use disorder, HIV/AIDS, psychotherapy,
  reproductive health, communicable disease) is released only with the
  member's separate permission (share-my-health-data notice).
- Release 4.6, dated 2026-09-22, added a path for apps that verified identity at
  IAL2/AAL2 through CLEAR; the HealthSafe ID sign-in remains the standard path.

### Registration

1. Create a One Healthcare ID; sign in to the App Owner portal at
   `flex.optum.com/portal`.
2. Register an organization: "An organization must register as a user ... Our
   security and compliance team will review your organization registration and
   approve or deny your ability to register applications." Estimated 5
   business days.
3. Register a sandbox app: immediate, "mock data".
4. Register a production app: "Production application requests for Patient
   Access APIs will require review from our security and compliance team".
   Estimated 5 business days.
5. A public privacy policy URL is required for the organization and the app.

The terms shown on the portal are "Terms and Conditions ( Patient Data API
License )". Points a household should know: the recipient obtains and keeps
member consent; the app's privacy policy and terms of use "must be made
publicly available"; breach notification under the FTC Health Breach
Notification Rule; UHG may limit or revoke access at its discretion; "UHG
liability capped at $50"; the recipient indemnifies UHG; "Recipient waives
right to participate in class action lawsuits"; "Mandatory binding arbitration
required"; New York law; "Continued use of the site is the acknowledgement and
acceptance of changes ... – no notifications will be made."

Support: `flexvendorsupport@optum.com`, Monday to Friday 9am to 4pm CST.

### Data

- `ExplanationOfBenefit` (CARIN BB 2.0.0; institutional, professional, oral,
  vision, pharmacy): "Returns records dated back to 2016-01-01, inclusive."
  Search parameters `_id`, `_count`, `_lastUpdated`, `identifier`,
  `service-date`, `type`, `patient`, `use`; paging by `_getpagesoffset`.
- `Coverage` (CARIN BB): "the coverage data that was effective as of the date of
  service of the claim." Search `_id`, `_count`, `_lastUpdated`, `patient`.
- Claims appear "1 business day from adjudication".
- Implementation guides: US Core 6.1.0, CARIN BB 2.0.0, PDex 1.0.0, Plan-Net
  1.1.0, US Drug Formulary 1.0.1.

### Dependents

A token carries one `patient`. UHC does not document whether a subscriber's
sign-in can authorize a dependent's record. Across CMS-9115 payers the common
pattern is one authorization per member, with the signed-in person choosing
self or a dependent on the consent screen.

## Member site (myuhc.com)

- Entry `https://www.myuhc.com/` redirects to `https://member.uhc.com/myuhc`.
  Sign-in is HealthSafe ID at `www.healthsafe-id.com`.
- Terms: the HealthSafe ID Terms of Use (effective September 28, 2022), linked
  from the sign-in page. Under "Restrictions on Use of Online Services ... You
  Will Not": "Copy, modify or harvest data, Content, or materials from the
  Online Services"; "Share any password with any third parties"; "use software
  or other means to access, "scrape," "crawl," or "spider," any web pages or
  other services from the Online Services. If you are blocked ... you agree not
  to implement any measures to circumvent such blocking"; "Attempt to reverse
  engineer any of the software used to provide the Online Services." The
  license is "personal, non-commercial ... to view the Content". "If you breach
  any of these Terms, your license to the Online Services and its Content
  terminates immediately." Disputes go to binding arbitration with a class
  action waiver; disputes "about the existence or extent of your health
  benefits or coverage" are excluded from arbitration.
- What it shows: claims for the member and covered family, EOB PDFs ("Download
  Explanation of Benefits (EOB)"), ID cards, plan balances. An employer's HR page
  says members "can download claims to a spreadsheet".
- Endpoints: not yet observed. Learning them needs the household's ruling on
  the terms and a real sign-in, with request logs only (method, host, path,
  status) and shapes, never values.

## EOB statement layout (UHC's public sample)

UHC's explainer `uhc.com/content/dam/uhcdotcom/en/general/understanding-your-eob-flier.pdf`
shows an employer-plan EOB ("Group name", "Group number", relationship "EE"):

- Header: member/patient, member ID, relationship, group name and number.
- "Claim summary": the service date range, "Provider billed", "Your total amount
  owed".
- "Plan balances", "As of MM/DD/YYYY for plan year YYYY", one table per person
  and one for "FAMILY": rows "Network" and "Out-of-Network", each with
  "Deductible" and "Out-of-pocket limit", columns "Maximum amount", "Progress so
  far", "Remaining amount" (a met limit reads "Met").
- Then claim details per claim and line, and definitions.

These balances are the accumulators `uhc accumulators` reports when the source
is an EOB file.

## Traps

- `uhc.com` returns 403 to `curl` for its PDFs (bot protection); fetch them in a
  browser.
- UHC's authorize example writes `Host: https://flex.optum.com/authz/[payer]/oauth/token`;
  the authorize endpoint is `.../oauth/authorize`, as the SMART configuration
  says.
- The SMART configuration's `introspection_endpoint` points at a `jwks.json`,
  and the OpenID configuration's `jwks_uri` is the relative `/jwks`.
- The SMART configuration does not list `client-public`, although UHC's page
  offers PKCE for public clients.
- Access tokens last about 5 minutes: refresh before each batch of requests,
  not once per run.
