//! The commands.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use pk_cli_auth::AuthStatus;
use pk_cli_config::ConfigStore;
use pk_cli_core::info::{AuthInfo, CliInfo};
use pk_cli_core::{dates, output, CliError, Paged, RangeArgs};
use pk_cli_documents::{fs_safe, verify_pdf, Document, DownloadBatch, SavedDocument, SkippedDocument};
use pk_cli_selfupdate::Updater;
use pk_med_records::dto::{kind, member_role, method, relationship};
use pk_med_records::{
    create_export_file, Finished, Header, Organization, PatientContext, RawResource, RecordsExport, RecordsWriter,
    Skip, Source, Trailer,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::cents::Cents;
use crate::cli::*;
use crate::eob::Issue;
use crate::fhir;
use crate::files::{self, Scan};
use crate::model::{Balances, Book, ClaimRecord, Member, StatementFile, ORGANIZATION, ORGANIZATION_NAME};

const REPO: &str = "piekstra/uhc-cli";
pub const PROFILES: &[&str] = &[pk_med_records::PROFILE, pk_cli_documents::PROFILE];
/// Where the statements come from; the stream's `organization.fhir_base`.
pub const SITE_BASE: &str = "https://member.uhc.com/myuhc";
/// `Skip.code` (an open set) for a statement read with issues.
const STATEMENT_ISSUES: &str = "statement_issues";
const CONFIG_KEYS: &str = "import_dir";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// The folder holding the downloaded EOB statement PDFs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_dir: Option<String>,
}

struct Ctx<'a> {
    cli: &'a Cli,
    store: ConfigStore,
}

impl Ctx<'_> {
    fn json(&self) -> bool {
        self.cli.common.json
    }

    fn note(&self, msg: &str) {
        if !self.cli.common.quiet {
            eprintln!("{msg}");
        }
    }

    fn import_dir(&self) -> Result<PathBuf, CliError> {
        if let Some(d) = &self.cli.dir {
            return Ok(expand(d));
        }
        let cfg: Config = self.store.load()?;
        match cfg.import_dir {
            Some(d) => Ok(expand(Path::new(&d))),
            None => Err(CliError::Usage(format!(
                "no import folder: download your EOB statements from myuhc.com into a folder, then \
                 `{BIN} config set import_dir <DIR>` (or pass --dir)"
            ))),
        }
    }

    fn scan(&self) -> Result<Scan, CliError> {
        let dir = self.import_dir()?;
        let scan = files::scan(&dir)?;
        if self.cli.common.verbose {
            eprintln!(
                "read {} statement(s) from {} PDF(s); {} skipped",
                scan.book.files.len(),
                scan.pdfs,
                scan.skipped.len()
            );
        }
        Ok(scan)
    }

    /// The statements, for a command that reads what they say. Says on
    /// stderr how many were read with issues (counts only), and fails when
    /// statements were found but none could be read: a changed layout, not
    /// an empty folder.
    fn records(&self) -> Result<Scan, CliError> {
        let scan = self.scan()?;
        let files = &scan.book.files;
        let with_issues = files.iter().filter(|f| !f.statement.issues.is_empty()).count();
        if with_issues > 0 {
            self.note(&format!(
                "note: {with_issues} statement(s) read with issues; `{BIN} import check` counts them"
            ));
        }
        let unread = |f: &&StatementFile| f.statement.claims.is_empty() && f.statement.balances.is_empty();
        if !files.is_empty() && files.iter().all(|f| unread(&f)) {
            return Err(CliError::Upstream(format!(
                "{} PDF(s) look like UnitedHealthcare EOB statements but none could be read; the statement \
                 layout may have changed (`{BIN} import check`)",
                files.len()
            )));
        }
        Ok(scan)
    }
}

fn expand(p: &Path) -> PathBuf {
    match (p.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => p.to_path_buf(),
    }
}

pub fn main() {
    // A reader that closes the pipe early (`uhc claims list | head`) ends the
    // process quietly, as it does any Unix tool, instead of a panic in println!.
    #[cfg(unix)]
    // SAFETY: called first, before any thread starts; it only resets the
    // disposition Rust's runtime set for SIGPIPE back to the default.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = <Cli as clap::Parser>::parse();
    if let Err(e) = run(&cli) {
        std::process::exit(output::fail(&e, cli.common.json));
    }
}

pub fn run(cli: &Cli) -> Result<(), CliError> {
    let ctx = Ctx {
        cli,
        store: ConfigStore::new(BIN).with_override(cli.config.clone()),
    };
    match &cli.command {
        Command::Auth(cmd) => auth(&ctx, cmd),
        Command::Config(cmd) => config(&ctx, cmd),
        Command::Patients(PatientsCmd::List) => patients(&ctx),
        Command::Records(RecordsCmd::Export(args)) => export(&ctx, args),
        Command::Claims(ClaimsCmd::List(args)) => claims_list(&ctx, args),
        Command::Claims(ClaimsCmd::Get { claim }) => claims_get(&ctx, claim),
        Command::Accumulators(args) => accumulators(&ctx, args),
        Command::Documents(DocumentsCmd::List(range)) => documents_list(&ctx, range),
        Command::Documents(DocumentsCmd::Download(args)) => documents_download(&ctx, args),
        Command::Import(ImportCmd::Check) => import_check(&ctx),
        Command::SelfUpdate(args) => Updater {
            repo: REPO.into(),
            binary: BIN.into(),
            target: env!("BUILD_TARGET").into(),
            current: env!("CARGO_PKG_VERSION").into(),
        }
        .run(args, cli.common.json, cli.common.quiet),
        Command::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), BIN, &mut std::io::stdout());
            Ok(())
        }
        Command::Info => {
            let info = CliInfo::new(
                BIN,
                env!("CARGO_PKG_VERSION"),
                &format!("https://github.com/{REPO}"),
                AuthInfo {
                    required: false,
                    method: "none".into(),
                    login_hint: None,
                },
                &["patients", "records", "claims", "accumulators", "documents", "import"],
            )
            .with_profiles(PROFILES);
            output::json(&serde_json::to_value(&info).expect("CliInfo serializes"));
            Ok(())
        }
    }
}

// ------------------------------------------------------------------ auth

fn auth(ctx: &Ctx, cmd: &AuthCmd) -> Result<(), CliError> {
    match cmd {
        AuthCmd::Login => Err(CliError::Usage(format!(
            "nothing to sign in to: {BIN} reads the EOB statements you download from myuhc.com \
             (`{BIN} config set import_dir <DIR>`)"
        ))),
        AuthCmd::Status => {
            AuthStatus::not_required().emit(ctx.json());
            Ok(())
        }
        AuthCmd::Logout => {
            if ctx.json() {
                output::json(&json!({ "schema": "auth-logout/v1", "signed_out": [] }));
            }
            ctx.note("nothing is stored: the files source keeps no credential");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------- config

fn config(ctx: &Ctx, cmd: &ConfigCmd) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Path => {
            let path = ctx.store.path()?;
            if ctx.json() {
                output::json(&json!({ "schema": "config-path/v1", "path": path.display().to_string() }));
            } else {
                println!("{}", path.display());
            }
            Ok(())
        }
        ConfigCmd::Show => {
            let cfg: Config = ctx.store.load()?;
            let mut v = serde_json::to_value(&cfg).unwrap_or_default();
            if ctx.json() {
                v["schema"] = "config/v1".into();
                output::json(&v);
            } else if v.as_object().is_some_and(|o| o.is_empty()) {
                ctx.note("no settings (`uhc config set import_dir <DIR>`)");
            } else {
                output::render(&v);
            }
            Ok(())
        }
        ConfigCmd::Set { key, value } => {
            let mut cfg: Config = ctx.store.load()?;
            match key.as_str() {
                "import_dir" => {
                    let dir = expand(Path::new(value));
                    if !dir.is_dir() {
                        return Err(CliError::Usage(format!("{} is not a folder", dir.display())));
                    }
                    cfg.import_dir = Some(dir.display().to_string());
                }
                other => {
                    return Err(CliError::Usage(format!(
                        "unknown config key `{other}` (known: {CONFIG_KEYS})"
                    )))
                }
            }
            ctx.store.save(&cfg)?;
            ctx.note(&format!("{key} set"));
            Ok(())
        }
        ConfigCmd::Unset { key } => {
            let mut cfg: Config = ctx.store.load()?;
            match key.as_str() {
                "import_dir" => cfg.import_dir = None,
                other => {
                    return Err(CliError::Usage(format!(
                        "unknown config key `{other}` (known: {CONFIG_KEYS})"
                    )))
                }
            }
            ctx.store.save(&cfg)?;
            ctx.note(&format!("{key} unset"));
            Ok(())
        }
    }
}

// -------------------------------------------------------------- patients

/// A member by id, or by first name when only one member has it. The
/// errors name ids only, never another member's name.
fn find_member<'a>(members: &'a [Member], q: &str) -> Result<&'a Member, CliError> {
    if members.is_empty() {
        return Err(CliError::NotFound(
            "no member: the import folder holds no UnitedHealthcare EOB statement (`uhc import check`)".into(),
        ));
    }
    let q = q.trim();
    if q.is_empty() {
        return Err(CliError::Usage("the member reference is empty".into()));
    }
    if let Some(m) = members.iter().find(|m| m.id == q) {
        return Ok(m);
    }
    let hits: Vec<&Member> = members.iter().filter(|m| m.label.eq_ignore_ascii_case(q)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(CliError::NotFound(format!(
            "no member matching `{q}` (`{BIN} patients list`)"
        ))),
        many => Err(CliError::NotFound(format!(
            "`{q}` names {} members ({}); give one id from `{BIN} patients list`",
            many.len(),
            many.iter().map(|m| m.id.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// A `patient-list/v1` item: the profile's `PatientContext`, and what the
/// statements say about the member's plan.
#[derive(Serialize)]
struct PatientRow {
    #[serde(flatten)]
    context: PatientContext,
    member_id: String,
    /// Coverage's relationship to the subscriber: self, spouse, common, child.
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_relationship: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_name: Option<String>,
    claims: u64,
    statements: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_statement: Option<String>,
}

fn organization() -> Organization {
    Organization {
        id: ORGANIZATION.into(),
        name: ORGANIZATION_NAME.into(),
        fhir_base: SITE_BASE.into(),
        vendor: Some("uhc".into()),
        location: None,
        aliases: Vec::new(),
    }
}

/// The member's role on the plan, from the relationship code the statements
/// addressed to them print; none for a member no statement is addressed to.
fn role(m: &Member) -> Option<&'static str> {
    match m.relationship.as_deref() {
        Some("self") => Some(member_role::SUBSCRIBER),
        Some(_) => Some(member_role::DEPENDENT),
        None => None,
    }
}

fn context(m: &Member, name: Option<String>) -> PatientContext {
    let role = role(m);
    PatientContext {
        id: m.id.clone(),
        organization: ORGANIZATION.into(),
        label: Some(m.label.clone()),
        // The subscriber downloads the statements: their own record. A
        // dependant's claims are read through the subscriber's account.
        relationship: (role == Some(member_role::SUBSCRIBER)).then(|| relationship::SELF.into()),
        kind: Some(kind::MEMBER.into()),
        member_role: role.map(str::to_string),
        fhir_user: None,
        name,
        session: None,
    }
}

fn patients(ctx: &Ctx) -> Result<(), CliError> {
    let scan = ctx.records()?;
    let book = &scan.book;
    let rows: Vec<PatientRow> = book
        .members()
        .into_iter()
        .map(|m| PatientRow {
            context: context(&m, book.full_name(&m.id)),
            member_id: m.member_id.clone(),
            coverage_relationship: m.relationship.clone(),
            group_number: m.group_number.clone(),
            group_name: m.group_name.clone(),
            claims: m.claims,
            statements: m.statements,
            latest_statement: m.latest_statement.clone(),
        })
        .collect();
    if !ctx.json() && rows.is_empty() {
        ctx.note("no member: the import folder holds no UnitedHealthcare EOB statement (`uhc import check`)");
        return Ok(());
    }
    if ctx.json() {
        Paged::new("patient", rows).emit(true);
    } else {
        let items: Vec<Value> = rows
            .iter()
            .map(|r| serde_json::to_value(r).unwrap_or_default())
            .collect();
        output::table(&output::table_view(
            &items,
            &["label", "id", "member_role", "claims", "latest_statement"],
        ));
    }
    Ok(())
}

// --------------------------------------------------------------- records

fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    dates::fmt_rfc3339(secs)
}

fn header(m: &Member, since: Option<&str>) -> Header {
    let mut h = Header::new(
        Source::new(BIN, env!("CARGO_PKG_VERSION")),
        organization(),
        context(m, None),
        now_rfc3339(),
    );
    h.method = Some(method::PORTAL.into());
    if let Some(s) = since {
        h.since = Some(s.into());
        // Exported whatever the date: the member, the plan and the payer.
        h.since_unbounded = vec!["Coverage".into(), "Organization".into(), "Patient".into()];
    }
    h
}

/// The exit code of a stream that ended incomplete: its error's.
fn incomplete_exit(trailer: &Trailer) -> i32 {
    trailer.error.as_ref().and_then(|e| e.exit_code).unwrap_or(1)
}

/// A `YYYY-MM-DD` date, zero-padded, so it compares as a string.
fn iso(d: &str) -> Result<String, CliError> {
    Ok(dates::fmt_iso(dates::parse_iso(d.trim())?))
}

/// What one member's export carries: the member's claims (with `since`,
/// those on a statement dated that day or later: what changed since) and
/// every statement that carried one of them (with `since`, those dated
/// that day or later).
pub struct Selection<'a> {
    pub claims: Vec<ClaimRecord>,
    pub files: Vec<(&'a StatementFile, Vec<usize>)>,
}

pub fn select<'a>(book: &'a Book, m: &Member, since: Option<&str>) -> Selection<'a> {
    let on_or_after = |d: Option<&str>| match since {
        Some(s) => d.is_none_or(|d| d >= s),
        None => true,
    };
    let claims: Vec<ClaimRecord> = book
        .claims()
        .into_iter()
        .filter(|c| c.patient == m.id)
        .filter(|c| match since {
            Some(s) => c
                .statement_date
                .as_deref()
                .or(c.service_start.as_deref())
                .is_some_and(|d| d >= s),
            None => true,
        })
        .collect();
    let files = book
        .files
        .iter()
        .filter(|f| on_or_after(f.statement.statement_date.as_deref()))
        .filter_map(|f| {
            let related: Vec<usize> = (0..claims.len())
                .filter(|&i| claims[i].documents.contains(&f.id))
                .collect();
            (!related.is_empty()).then_some((f, related))
        })
        .collect();
    Selection { claims, files }
}

/// Every resource of one member's export, in stream order.
pub fn member_resources(book: &Book, m: &Member, sel: &Selection) -> Result<Vec<Value>, CliError> {
    let coverage = fhir::coverage_id(m);
    let mut out = vec![
        fhir::organization(),
        fhir::patient(m, book.full_name(&m.id).as_deref()),
        fhir::coverage(m),
    ];
    for c in &sel.claims {
        let balances = book.balances_in(&c.document, &c.claim.patient);
        let refs: Vec<&Balances> = balances.iter().collect();
        out.push(fhir::explanation_of_benefit(c, &coverage, &refs));
    }
    for (f, related) in &sel.files {
        let related: Vec<&ClaimRecord> = related.iter().map(|&i| &sel.claims[i]).collect();
        let bytes = std::fs::read(&f.path).map_err(|e| CliError::Other(format!("reading a statement again: {e}")))?;
        out.push(fhir::document_reference(f, &m.id, &related));
        out.push(fhir::binary(f, &m.id, &bytes));
    }
    Ok(out)
}

fn export(ctx: &Ctx, args: &ExportArgs) -> Result<(), CliError> {
    if args.ndjson && ctx.json() {
        return Err(CliError::Usage("--ndjson is the output; drop --json".into()));
    }
    let since = args.since.as_deref().map(iso).transpose()?;
    if !args.ndjson && args.output.is_none() {
        return Err(CliError::Usage(
            "say where the records go: --ndjson for stdout (`| medrec import --from uhc`) or -o <DIR>".into(),
        ));
    }
    let scan = ctx.records()?;
    let members = scan.book.members();
    let m = find_member(&members, &args.who.patient)?;
    let since = since.as_deref();

    if let Some(dir) = &args.output {
        // A plain file of records, so every statement in the folder must be
        // synthetic: a member's export draws on statements beyond the ones
        // carrying their claims (names, group, the statement a claim's
        // dependant appears on).
        if scan.book.files.iter().any(|f| !f.statement.synthetic) {
            return Err(CliError::Usage(
                "-o writes records to a plain file, so it is for folders of synthetic statements only".into(),
            )
            .with_remedy(format!(
                "pipe real records into medrec: `{BIN} records export --patient {} --ndjson | medrec import --from uhc`",
                m.id
            )));
        }
        // Created 0600 under a temporary name and renamed into place; a
        // symbolic link at records.ndjson is refused (exit 2).
        let (path, file) = create_export_file(dir)?;
        let finished = write_stream(std::io::BufWriter::new(file), &scan.book, m, since)?;
        let summary = RecordsExport::new(path.display().to_string(), &finished);
        if ctx.json() {
            output::json(&serde_json::to_value(&summary).unwrap_or_default());
        } else {
            ctx.note(&format!(
                "wrote {} ({} resources)",
                path.display(),
                finished.trailer.total()
            ));
        }
        if !finished.trailer.complete {
            // The summary is the one document; the trailer carries the error.
            std::process::exit(incomplete_exit(&finished.trailer));
        }
        return Ok(());
    }
    let stdout = std::io::stdout();
    match write_stream(stdout.lock(), &scan.book, m, since) {
        Ok(finished) if finished.trailer.complete => Ok(()),
        // The trailer carries the error; exit without a second document.
        Ok(finished) => std::process::exit(incomplete_exit(&finished.trailer)),
        // stdout is the stream, so a write error goes to stderr only.
        Err(e) => {
            eprintln!("error: {}", e.message());
            std::process::exit(e.exit_code());
        }
    }
}

fn write_stream<W: Write>(out: W, book: &Book, m: &Member, since: Option<&str>) -> Result<Finished, CliError> {
    let mut w = RecordsWriter::start(out, &header(m, since))?;
    let sel = select(book, m, since);
    for (f, _) in &sel.files {
        if !f.statement.issues.is_empty() {
            let codes: Vec<&str> = f.statement.issues.iter().map(|i| i.code()).collect();
            w.skip(Skip::resource(
                "ExplanationOfBenefit",
                "",
                STATEMENT_ISSUES,
                format!(
                    "statement {} was read with issues ({}); a claim on it may be incomplete",
                    fhir::document_id(&f.id, &m.id),
                    codes.join(", ")
                ),
            ));
        }
    }
    match member_resources(book, m, &sel) {
        Ok(resources) => {
            for r in &resources {
                w.resource(&RawResource::from_value(r)?)?;
            }
            w.finish(now_rfc3339(), None)
        }
        Err(e) => w.finish(now_rfc3339(), Some(&e)),
    }
}

// ---------------------------------------------------------------- claims

fn amounts_json(a: &crate::eob::Amounts) -> Map<String, Value> {
    a.fields()
        .iter()
        .map(|(k, c)| ((*k).to_string(), serde_json::to_value(c.money()).unwrap_or_default()))
        .collect()
}

fn claim_json(c: &ClaimRecord, detail: bool) -> Value {
    let mut v = Map::new();
    v.insert("id".into(), c.id.clone().into());
    if let Some(n) = &c.claim_number {
        v.insert("claim_number".into(), n.clone().into());
    }
    v.insert("patient".into(), c.patient.clone().into());
    v.insert("label".into(), c.label.clone().into());
    if let Some(p) = &c.claim.provider {
        v.insert("provider".into(), p.clone().into());
    }
    if let Some(n) = c.claim.network {
        v.insert("network".into(), n.code().into());
    }
    if let Some(s) = &c.service_start {
        v.insert("service_start".into(), s.clone().into());
    }
    if let Some(e) = &c.service_end {
        v.insert("service_end".into(), e.clone().into());
    }
    v.extend(amounts_json(&c.totals));
    if let Some(d) = &c.statement_date {
        v.insert("statement_date".into(), d.clone().into());
    }
    v.insert("document".into(), c.document.clone().into());
    v.insert("versions".into(), c.versions.into());
    if detail {
        if let Some(a) = &c.claim.account_number {
            v.insert("patient_account_number".into(), a.clone().into());
        }
        let lines: Vec<Value> = c
            .claim
            .lines
            .iter()
            .map(|l| {
                let mut lv = Map::new();
                lv.insert("description".into(), l.description.clone().into());
                if let Some(d) = &l.date {
                    lv.insert("date".into(), d.clone().into());
                }
                if let Some(d) = &l.date_end {
                    lv.insert("date_end".into(), d.clone().into());
                }
                if !l.codes.is_empty() {
                    lv.insert("codes".into(), json!(l.codes));
                }
                lv.extend(amounts_json(&l.amounts));
                Value::Object(lv)
            })
            .collect();
        v.insert("lines".into(), Value::Array(lines));
        if !c.codes.is_empty() {
            v.insert("codes".into(), json!(c.codes));
        }
    } else {
        v.insert("lines".into(), c.claim.lines.len().into());
    }
    Value::Object(v)
}

/// `--since` / `--until`, checked and zero-padded so they compare as strings.
struct Window {
    since: Option<String>,
    until: Option<String>,
}

impl Window {
    fn of(range: &RangeArgs) -> Result<Window, CliError> {
        range.validate()?;
        let w = Window {
            since: range.since.as_deref().map(iso).transpose()?,
            until: range.until.as_deref().map(iso).transpose()?,
        };
        if let (Some(s), Some(u)) = (&w.since, &w.until) {
            if s > u {
                return Err(CliError::Usage(format!("--since {s} is after --until {u}")));
            }
        }
        Ok(w)
    }

    fn contains(&self, date: Option<&str>) -> bool {
        match date {
            Some(d) => self.since.as_deref().is_none_or(|s| d >= s) && self.until.as_deref().is_none_or(|u| d <= u),
            None => self.since.is_none() && self.until.is_none(),
        }
    }
}

fn claims_list(ctx: &Ctx, args: &ClaimsListArgs) -> Result<(), CliError> {
    let window = Window::of(&args.range)?;
    let scan = ctx.records()?;
    let members = scan.book.members();
    let patient = args.patient.as_deref().map(|q| find_member(&members, q)).transpose()?;
    let claims: Vec<Value> = scan
        .book
        .claims()
        .iter()
        .filter(|c| patient.is_none_or(|m| c.patient == m.id))
        .filter(|c| window.contains(c.service_start.as_deref().or(c.statement_date.as_deref())))
        .take(args.range.limit.map(|n| n as usize).unwrap_or(usize::MAX))
        .map(|c| claim_json(c, false))
        .collect();
    if ctx.json() {
        Paged::new("claim", claims).emit(true);
    } else if claims.is_empty() {
        ctx.note("no claims");
    } else {
        output::table(&output::table_view(
            &claims,
            &[
                "service_start",
                "label",
                "provider",
                "billed",
                "plan_paid",
                "you_owe",
                "claim_number",
            ],
        ));
    }
    Ok(())
}

fn claims_get(ctx: &Ctx, q: &str) -> Result<(), CliError> {
    let scan = ctx.records()?;
    let claims = scan.book.claims();
    let q = q.trim();
    let c = claims
        .iter()
        .find(|c| c.id == q || c.claim_number.as_deref() == Some(q))
        .ok_or_else(|| CliError::NotFound(format!("no claim {q} in the statements (`uhc claims list`)")))?;
    output::emit_one(ctx.json(), "claim", claim_json(c, true));
    Ok(())
}

// ---------------------------------------------------------- accumulators

fn accumulators(ctx: &Ctx, args: &AccumulatorsArgs) -> Result<(), CliError> {
    let scan = ctx.records()?;
    let members = scan.book.members();
    let patient = args.patient.as_deref().map(|q| find_member(&members, q)).transpose()?;
    let items: Vec<Value> = scan
        .book
        .balances()
        .iter()
        .filter(|b| args.year.is_none_or(|y| b.plan_year == Some(y)))
        .filter(|b| match patient {
            Some(m) => b.patient == m.id || (b.family && b.member_id == m.member_id),
            None => true,
        })
        .map(balance_json)
        .collect();
    if ctx.json() {
        Paged::new("accumulator", items).emit(true);
    } else if items.is_empty() {
        ctx.note("no plan balances in the statements");
    } else {
        output::table(&output::table_view(
            &items,
            &[
                "plan_year",
                "label",
                "network",
                "limit",
                "used",
                "maximum",
                "remaining",
                "met",
                "as_of",
            ],
        ));
    }
    Ok(())
}

fn balance_json(b: &Balances) -> Value {
    let mut v = Map::new();
    v.insert("patient".into(), b.patient.clone().into());
    v.insert("label".into(), b.label.clone().into());
    v.insert("family".into(), b.family.into());
    if let Some(y) = b.plan_year {
        v.insert("plan_year".into(), y.into());
    }
    if let Some(d) = &b.as_of {
        v.insert("as_of".into(), d.clone().into());
    }
    v.insert("network".into(), b.network.code().into());
    v.insert("limit".into(), b.limit.code().into());
    let money = |c: Cents| serde_json::to_value(c.money()).unwrap_or_default();
    v.insert("maximum".into(), money(b.maximum));
    v.insert("used".into(), money(b.used));
    v.insert("remaining".into(), money(b.remaining));
    v.insert("met".into(), b.met.into());
    v.insert("document".into(), b.document.clone().into());
    Value::Object(v)
}

// ------------------------------------------------------------- documents

/// The member's own file name for a statement, made safe, as a PDF.
fn file_name(f: &StatementFile) -> Option<String> {
    f.path
        .file_stem()
        .and_then(|n| n.to_str())
        .map(|stem| format!("{}.pdf", fs_safe(stem)))
}

fn document_of(f: &StatementFile, id: String, label: Option<&str>) -> Document {
    let mut name = match &f.statement.statement_date {
        Some(date) => format!("UnitedHealthcare EOB {date}"),
        None => "UnitedHealthcare EOB".to_string(),
    };
    if let Some(label) = label {
        name.push_str(&format!(", {label}"));
    }
    let mut d = Document::new(id, name);
    d.date = f.statement.statement_date.clone();
    d.category = Some("eob".into());
    d.file = file_name(f);
    d
}

/// Each statement's members, as `Book::document_members` gives them.
type DocumentMembers = std::collections::HashMap<String, Vec<(String, String)>>;

/// A statement's `documents/v1` rows: one per `DocumentReference` the export
/// writes for it, so `Document.id` is the `DocumentReference.id`. A
/// statement no export carries is listed once under its statement id, so no
/// row names a `DocumentReference` that is never written.
fn documents_of(members: &DocumentMembers, f: &StatementFile) -> Vec<Document> {
    let Some(members) = members.get(&f.id).filter(|m| !m.is_empty()) else {
        return vec![document_of(f, f.id.clone(), None)];
    };
    members
        .iter()
        .map(|(id, label)| document_of(f, fhir::document_id(&f.id, id), Some(label)))
        .collect()
}

fn documents_list(ctx: &Ctx, range: &RangeArgs) -> Result<(), CliError> {
    let window = Window::of(range)?;
    let scan = ctx.scan()?;
    let members = scan.book.document_members();
    let docs: Vec<Document> = scan
        .book
        .files
        .iter()
        .rev()
        .filter(|f| window.contains(f.statement.statement_date.as_deref()))
        .flat_map(|f| documents_of(&members, f))
        .take(range.limit.map(|n| n as usize).unwrap_or(usize::MAX))
        .collect();
    if !ctx.json() && docs.is_empty() {
        ctx.note("no statements in the import folder (`uhc import check`)");
        return Ok(());
    }
    Paged::new("document", docs).emit(ctx.json());
    Ok(())
}

/// The statement a document id names: a `documents list` id, all of it or
/// a prefix of it, or a prefix of the statement id `claims list` shows.
fn find_document<'a>(book: &'a Book, q: &str) -> Result<(&'a StatementFile, Document), CliError> {
    let q = q.trim().to_ascii_lowercase();
    if q.len() < 6 {
        return Err(CliError::Usage("give at least 6 characters of the document id".into()));
    }
    let members = book.document_members();
    let mut hits: Vec<(&StatementFile, Document)> = Vec::new();
    for f in &book.files {
        let docs = documents_of(&members, f);
        if let Some(d) = docs.iter().find(|d| d.id == q) {
            return Ok((f, d.clone()));
        }
        // Two rows of one statement are the same file: the first serves.
        if let Some(d) = docs.iter().find(|d| d.id.starts_with(&q)) {
            hits.push((f, d.clone()));
        } else if f.id.starts_with(&q) {
            hits.push((f, docs[0].clone()));
        }
    }
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => Err(CliError::NotFound(format!("no document {q} (`uhc documents list`)"))),
        n => Err(CliError::NotFound(format!(
            "{q} matches {n} documents; give more of the id"
        ))),
    }
}

/// Open `path` for writing, owner-only (0600). An existing file is a usage
/// error unless `replace`; a replaced file (or a symlink in its place) is
/// removed first, so the new one gets the private mode and nothing is
/// written through a link.
fn open_private(path: &Path, replace: bool) -> Result<std::fs::File, CliError> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.is_dir() {
            return Err(CliError::Usage(format!("{} is a folder", path.display())));
        }
        if !replace {
            return Err(CliError::Usage(format!(
                "{} exists; pass --overwrite to replace it",
                path.display()
            )));
        }
        std::fs::remove_file(path).map_err(|e| CliError::Other(format!("replacing {}: {e}", path.display())))?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            CliError::Usage(format!("{} exists; pass --overwrite to replace it", path.display()))
        } else {
            CliError::Other(format!("writing {}: {e}", path.display()))
        }
    })
}

fn write_private(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), CliError> {
    open_private(path, overwrite)?
        .write_all(bytes)
        .map_err(|e| CliError::Other(format!("writing {}: {e}", path.display())))
}

/// Whether the file at `path` holds exactly `bytes`.
fn same_content(path: &Path, bytes: &[u8]) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == bytes.len() as u64)
        && std::fs::read(path).is_ok_and(|b| b == bytes)
}

fn saved_name(f: &StatementFile) -> String {
    match file_name(f) {
        Some(name) if !name.is_empty() => name,
        _ => format!(
            "{}_uhc-eob_{}.pdf",
            f.statement.statement_date.as_deref().unwrap_or("undated"),
            f.id
        ),
    }
}

fn documents_download(ctx: &Ctx, args: &DownloadArgs) -> Result<(), CliError> {
    let scan = ctx.scan()?;
    if args.all {
        let dir = args.output.clone().unwrap_or_else(|| PathBuf::from("."));
        if dir.as_os_str() == "-" {
            return Err(CliError::Usage("--all writes a folder, not stdout".into()));
        }
        std::fs::create_dir_all(&dir).map_err(|e| CliError::Other(format!("creating {}: {e}", dir.display())))?;
        let mut saved = Vec::new();
        let mut skipped = Vec::new();
        let mut files_written = 0usize;
        let members = scan.book.document_members();
        // Names taken in this batch, case-folded: APFS and NTFS ignore case.
        let mut used = std::collections::HashSet::new();
        for f in &scan.book.files {
            let bytes = std::fs::read(&f.path).map_err(|e| CliError::Other(format!("reading a statement: {e}")))?;
            verify_pdf(&bytes)?;
            // The member's file name, else one with the document id. A name
            // already in the folder counts as this statement only when the
            // file there holds the same bytes; otherwise the next name is tried.
            let plain = saved_name(f);
            let candidates = [plain.clone(), format!("{}_{plain}", f.id)];
            let mut outcome = None;
            for name in candidates.iter() {
                if used.contains(&name.to_lowercase()) {
                    continue;
                }
                let path = dir.join(name);
                if same_content(&path, &bytes) && !args.overwrite {
                    used.insert(name.to_lowercase());
                    outcome = Some(Err("exists"));
                    break;
                }
                if std::fs::symlink_metadata(&path).is_ok() && !args.overwrite {
                    continue;
                }
                used.insert(name.to_lowercase());
                outcome = Some(Ok(path));
                break;
            }
            if let Some(Ok(path)) = &outcome {
                write_private(path, &bytes, args.overwrite)?;
                files_written += 1;
            }
            // Every row of the statement shares its one file.
            for doc in documents_of(&members, f) {
                match &outcome {
                    Some(Ok(path)) => saved.push(SavedDocument::from_document(
                        &doc,
                        path.display().to_string(),
                        bytes.len() as u64,
                    )),
                    Some(Err(code)) => skipped.push(
                        SkippedDocument::new(doc.id.clone(), "already saved; --overwrite replaces it").with_code(*code),
                    ),
                    None => skipped.push(
                        SkippedDocument::new(doc.id.clone(), "other files hold its names; --overwrite replaces them")
                            .with_code("name_taken"),
                    ),
                }
            }
        }
        let count = |code: &str| skipped.iter().filter(|k| k.code.as_deref() == Some(code)).count();
        let (kept, taken) = (count("exists"), count("name_taken"));
        let batch = DownloadBatch::new(dir.display().to_string(), saved).with_skipped(skipped);
        if ctx.json() {
            output::json(&serde_json::to_value(&batch).unwrap_or_default());
        } else {
            let mut note = format!("saved {} document(s) to {}", batch.count, batch.dir);
            if files_written as u64 != batch.count {
                note.push_str(&format!(" in {files_written} file(s)"));
            }
            if kept > 0 {
                note.push_str(&format!("; {kept} already there"));
            }
            if taken > 0 {
                note.push_str(&format!(
                    "; {taken} not saved, other files hold their names (--overwrite)"
                ));
            }
            ctx.note(&note);
        }
        return Ok(());
    }
    let id = args.id.as_deref().expect("clap requires an id without --all");
    let (f, doc) = find_document(&scan.book, id)?;
    let bytes = std::fs::read(&f.path).map_err(|e| CliError::Other(format!("reading a statement: {e}")))?;
    verify_pdf(&bytes)?;
    match args.output.as_deref() {
        Some(p) if p.as_os_str() == "-" => {
            std::io::stdout()
                .write_all(&bytes)
                .map_err(|e| CliError::Other(format!("writing stdout: {e}")))?;
            Ok(())
        }
        other => {
            let path = match other {
                Some(p) if p.is_dir() => p.join(saved_name(f)),
                Some(p) => p.to_path_buf(),
                None => PathBuf::from(saved_name(f)),
            };
            write_private(&path, &bytes, args.overwrite)?;
            let saved = SavedDocument::from_document(&doc, path.display().to_string(), bytes.len() as u64);
            if ctx.json() {
                output::json(&serde_json::to_value(&saved).unwrap_or_default());
            } else {
                ctx.note(&format!("saved {}", saved.path));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------- import

fn import_check(ctx: &Ctx) -> Result<(), CliError> {
    let scan = ctx.scan()?;
    let book = &scan.book;
    let mut skipped: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for s in &scan.skipped {
        *skipped.entry(s.code).or_insert(0) += 1;
    }
    let mut issues: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    let mut with_issues = 0u64;
    for f in &book.files {
        if !f.statement.issues.is_empty() {
            with_issues += 1;
        }
        for i in &f.statement.issues {
            *issues.entry(Issue::code(*i)).or_insert(0) += 1;
        }
    }
    let claims = book.claims();
    let mut years: Vec<i32> = book
        .files
        .iter()
        .flat_map(|f| f.statement.balances.iter().filter_map(|b| b.plan_year))
        .collect();
    years.sort_unstable();
    years.dedup();
    let dto = json!({
        "schema": "import-check/v1",
        "pdfs": scan.pdfs,
        "statements": book.files.len(),
        "synthetic_statements": book.files.iter().filter(|f| f.statement.synthetic).count(),
        "skipped": skipped,
        "members": book.members().len(),
        "claims": claims.len(),
        "claims_without_number": claims.iter().filter(|c| c.claim_number.is_none()).count(),
        "reprocessed_claims": claims.iter().filter(|c| c.versions > 1).count(),
        "claim_lines": claims.iter().map(|c| c.claim.lines.len()).sum::<usize>(),
        "balance_rows": book.files.iter().flat_map(|f| f.statement.balances.iter()).map(|b| b.rows.len()).sum::<usize>(),
        "plan_years": years,
        "statements_with_issues": with_issues,
        "issues": issues,
        // Every PDF read as a statement without an issue, and something read.
        // A PDF that is not an EOB (a letter saved alongside) is not a problem.
        "ok": !book.files.is_empty() && with_issues == 0 && skipped.keys().all(|k| *k == "not_eob"),
    });
    output::emit_one(ctx.json(), "import-check/v1", dto);
    Ok(())
}
