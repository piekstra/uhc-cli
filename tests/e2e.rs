//! The binary end to end over synthetic statement PDFs: the fixtures in
//! `tests/fixtures/eob/` written as PDFs at test time, read back through the
//! real `pdftotext`. Skipped (with a note) where poppler is not installed;
//! CI installs it.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use base64::Engine;
use serde_json::Value;

fn have_pdftotext() -> bool {
    std::process::Command::new("pdftotext").arg("-v").output().is_ok()
}

macro_rules! need_pdftotext {
    () => {
        if !have_pdftotext() {
            eprintln!("skipped: pdftotext (poppler) is not installed");
            return;
        }
    };
}

fn tmp(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "uhc-e2e-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/eob/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The synthetic statements as PDFs in a fresh folder.
fn statements(names: &[&str]) -> PathBuf {
    let dir = tmp("in");
    for name in names {
        let pdf = uhc::synth::text_pdf(&fixture_text(name));
        std::fs::write(dir.join(name.replace(".txt", ".pdf")), pdf).unwrap();
    }
    dir
}

const GOOD: &[&str] = &[
    "2026-03-14-statement.txt",
    "2026-04-30-statement.txt",
    "2026-05-08-statement.txt",
];

fn uhc(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("uhc").unwrap();
    cmd.env("XDG_CONFIG_HOME", tmp("cfg"))
        .env_remove("NO_COLOR")
        .env_remove("UHC_PDFTOTEXT")
        .arg("--dir")
        .arg(dir);
    cmd
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.arg("--json").assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).unwrap()
}

fn member_id(dir: &Path, label: &str) -> String {
    let v = json_of(uhc(dir).args(["patients", "list"]));
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["label"] == label)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn import_check_counts_statements_skips_and_issues() {
    need_pdftotext!();
    let dir = statements(&[GOOD, &["premium-notice.txt", "damaged-statement.txt"]].concat());
    std::fs::write(dir.join("not-really.pdf"), b"<html>an error page</html>").unwrap();
    let v = json_of(uhc(&dir).args(["import", "check"]));
    assert_eq!(v["schema"], "import-check/v1");
    assert_eq!(v["pdfs"], 6);
    assert_eq!(v["statements"], 4);
    assert_eq!(v["synthetic_statements"], 4);
    assert_eq!(v["skipped"]["not_eob"], 1);
    assert_eq!(v["skipped"]["not_pdf"], 1);
    assert_eq!(v["claims"], 5);
    assert_eq!(v["statements_with_issues"], 1);
    assert_eq!(v["issues"]["claim_total_mismatch"], 1);
    assert_eq!(v["ok"], false);
    // Counts only: nothing the statements print reaches the check.
    let text = v.to_string();
    for value in ["ALEX", "Alex", "SYNM", "SYNC", "EXAMPLE", "$"] {
        assert!(!text.contains(value), "import check printed {value}");
    }
}

#[test]
fn patients_are_the_subscriber_and_dependents() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let v = json_of(uhc(&dir).args(["patients", "list"]));
    assert_eq!(v["schema"], "patient-list/v1");
    let items = v["items"].as_array().unwrap();
    let labels: Vec<&str> = items.iter().map(|m| m["label"].as_str().unwrap()).collect();
    assert_eq!(labels, ["Alex", "Jordan", "Sam"]);
    let alex = &items[0];
    assert_eq!(alex["relationship"], "self");
    assert_eq!(alex["organization"], "uhc");
    assert_eq!(alex["claims"], 2);
    assert_eq!(items[2]["relationship"], "spouse");
    assert!(alex["id"].as_str().unwrap().starts_with('m'));
}

#[test]
fn records_export_is_a_complete_stream_whose_references_resolve() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let alex = member_id(&dir, "Alex");
    let out = uhc(&dir)
        .args(["records", "export", "--patient", &alex, "--ndjson"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();

    let header = &lines[0];
    assert_eq!(header["schema"], "records-stream/v1");
    assert_eq!(header["method"], "portal");
    assert_eq!(header["patient"]["id"], alex.as_str());
    assert_eq!(header["organization"]["id"], "uhc");
    let trailer = lines.last().unwrap();
    assert_eq!(trailer["schema"], "records-stream-end/v1");
    assert_eq!(trailer["complete"], true);
    assert_eq!(trailer["counts"]["ExplanationOfBenefit"], 2);
    assert_eq!(trailer["counts"]["Coverage"], 1);
    assert_eq!(trailer["counts"]["DocumentReference"], 2);
    assert_eq!(trailer["counts"]["Binary"], 2);

    let resources = &lines[1..lines.len() - 1];
    let ids: std::collections::HashSet<String> = resources
        .iter()
        .map(|r| format!("{}/{}", r["resourceType"].as_str().unwrap(), r["id"].as_str().unwrap()))
        .collect();
    let mut refs = Vec::new();
    collect_refs(&Value::Array(resources.to_vec()), &mut refs);
    for r in &refs {
        assert!(ids.contains(r), "{r} does not resolve in the stream");
    }
    for r in resources {
        assert_eq!(r["meta"]["source"], "uhc:eob-pdf");
    }

    let eob = resources.iter().find(|r| r["id"] == "cSYNC00000003").unwrap();
    assert_eq!(eob["patient"]["reference"], format!("Patient/{alex}"));
    let owed = eob["total"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["category"]["coding"][0]["code"] == "memberliability")
        .unwrap();
    assert_eq!(owed["amount"]["value"], 955.0);
    assert_eq!(owed["category"]["coding"][0]["display"], "Member liability");
    assert_eq!(eob["type"]["text"], "Medical claim");
    assert!(eob["benefitBalance"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["unit"]["coding"][0]["code"] == "family"));

    // The Binary carries the statement PDF byte for byte.
    let bin = resources.iter().find(|r| r["resourceType"] == "Binary").unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bin["data"].as_str().unwrap())
        .unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let on_disk: Vec<Vec<u8>> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| std::fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert!(on_disk.contains(&bytes));
}

fn collect_refs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let binary_url = k == "url" && x.as_str().is_some_and(|s| s.starts_with("Binary/"));
                if k == "reference" || binary_url {
                    out.push(x.as_str().unwrap().to_string());
                } else {
                    collect_refs(x, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_refs(x, out)),
        _ => {}
    }
}

#[test]
fn records_export_since_is_what_changed_since() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let alex = member_id(&dir, "Alex");
    let export = |since: &str| {
        let out = uhc(&dir)
            .args(["records", "export", "--patient", &alex, "--since", since, "--ndjson"])
            .assert()
            .success();
        let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        lines
    };
    // The 04/30 statement's claim was for service on 04/01: a statement on
    // or after the day counts, not the day of service.
    let lines = export("2026-04-15");
    let trailer = lines.last().unwrap();
    assert_eq!(trailer["counts"]["ExplanationOfBenefit"], 1);
    assert_eq!(trailer["counts"]["DocumentReference"], 1);
    assert!(lines.iter().any(|r| r["id"] == "cSYNC00000003"));
    assert_eq!(lines[0]["since"], "2026-04-15");
    // An unpadded date means the same day.
    let lines = export("2026-4-15");
    assert_eq!(lines[0]["since"], "2026-04-15");
    assert_eq!(lines.last().unwrap()["counts"]["ExplanationOfBenefit"], 1);
    // After the last statement, nothing changed.
    let lines = export("2026-05-01");
    assert!(lines.last().unwrap()["counts"].get("ExplanationOfBenefit").is_none());
}

#[test]
fn records_export_to_a_folder_is_for_synthetic_statements_only() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let alex = member_id(&dir, "Alex");
    let out = tmp("out");
    let v = json_of(
        uhc(&dir)
            .args(["records", "export", "--patient", &alex, "-o"])
            .arg(&out),
    );
    assert_eq!(v["schema"], "records-export/v1");
    assert!(out.join("records.ndjson").is_file());

    // The same statement without the synthetic marker is refused.
    let unmarked = tmp("unmarked");
    let text = fixture_text("2026-05-08-statement.txt").replacen(uhc::eob::SYNTHETIC_MARKER, "", 1);
    std::fs::write(unmarked.join("s.pdf"), uhc::synth::text_pdf(&text)).unwrap();
    let sam = member_id(&unmarked, "Sam");
    let target = tmp("refused");
    let err = uhc(&unmarked)
        .args(["records", "export", "--patient", &sam, "--json", "-o"])
        .arg(&target)
        .assert()
        .code(2);
    let e: Value = serde_json::from_slice(&err.get_output().stdout).unwrap();
    assert_eq!(e["error"]["code"], "usage");
    assert!(!target.join("records.ndjson").exists());
}

#[test]
fn a_first_name_finds_a_member_and_an_unknown_one_is_not_found() {
    need_pdftotext!();
    let dir = statements(GOOD);
    uhc(&dir)
        .args(["records", "export", "--patient", "sam", "--ndjson"])
        .assert()
        .success();
    uhc(&dir)
        .args(["records", "export", "--patient", "nobody", "--ndjson"])
        .assert()
        .code(4);
    uhc(&dir)
        .args(["claims", "list", "--patient", "nobody"])
        .assert()
        .code(4);
}

#[test]
fn claims_list_and_get() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let v = json_of(uhc(&dir).args(["claims", "list"]));
    assert_eq!(v["schema"], "claim-list/v1");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    let starts: Vec<&str> = items.iter().map(|c| c["service_start"].as_str().unwrap()).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(starts, sorted, "newest service first");
    assert_eq!(
        items[0]["you_owe"],
        serde_json::json!({ "amount": "25.00", "currency": "USD" })
    );

    let v = json_of(uhc(&dir).args(["claims", "list", "--patient", "alex", "--limit", "1"]));
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    let v = json_of(uhc(&dir).args(["claims", "list", "--since", "2026-04-01", "--until", "2026-04-30"]));
    assert_eq!(v["items"].as_array().unwrap().len(), 1);

    let c = json_of(uhc(&dir).args(["claims", "get", "SYNC00000003"]));
    assert_eq!(c["schema"], "claim/v1");
    assert_eq!(c["lines"].as_array().unwrap().len(), 2);
    assert_eq!(c["lines"][0]["codes"], serde_json::json!(["2Q", "B1"]));
    uhc(&dir).args(["claims", "get", "SYNC99999999"]).assert().code(4);
}

#[test]
fn accumulators_take_the_latest_balances() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let v = json_of(uhc(&dir).args(["accumulators", "--patient", "alex"]));
    assert_eq!(v["schema"], "accumulator-list/v1");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 8, "Alex's four rows and the family's four");
    let ded = items
        .iter()
        .find(|b| b["label"] == "Alex" && b["network"] == "in" && b["limit"] == "deductible")
        .unwrap();
    assert_eq!(ded["met"], true);
    assert_eq!(ded["as_of"], "2026-04-30");
    let v = json_of(uhc(&dir).args(["accumulators", "--year", "2025"]));
    assert!(v["items"].as_array().unwrap().is_empty());
}

#[test]
fn documents_list_and_download() {
    need_pdftotext!();
    let dir = statements(GOOD);
    let v = json_of(uhc(&dir).args(["documents", "list"]));
    assert_eq!(v["schema"], "document-list/v1");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["date"], "2026-05-08", "newest first");
    let id = items[0]["id"].as_str().unwrap().to_string();

    let out = tmp("dl");
    let saved = json_of(uhc(&dir).args(["documents", "download", &id[..8], "-o"]).arg(&out));
    assert_eq!(saved["schema"], "document-download/v1");
    let path = PathBuf::from(saved["path"].as_str().unwrap());
    assert_eq!(path.file_name().unwrap(), "2026-05-08-statement.pdf");
    assert_eq!(items[0]["file"], "2026-05-08-statement.pdf");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(dir.join("2026-05-08-statement.pdf")).unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // A second download refuses to replace the file unless asked.
    uhc(&dir)
        .args(["documents", "download", &id, "-o"])
        .arg(&out)
        .assert()
        .code(2);
    uhc(&dir)
        .args(["documents", "download", &id, "--overwrite", "-o"])
        .arg(&out)
        .assert()
        .success();
    // To stdout, the PDF itself.
    let raw = uhc(&dir)
        .args(["documents", "download", &id, "-o", "-"])
        .assert()
        .success();
    assert!(raw.get_output().stdout.starts_with(b"%PDF-"));

    let all = tmp("all");
    let batch = json_of(uhc(&dir).args(["documents", "download", "--all", "-o"]).arg(&all));
    assert_eq!(batch["schema"], "document-download-batch/v1");
    assert_eq!(batch["count"], 3);
    // A re-run skips what is already saved. A file of the same name holding
    // something else is left alone and the statement saved beside it.
    let again = json_of(uhc(&dir).args(["documents", "download", "--all", "-o"]).arg(&all));
    assert_eq!(again["count"], 0);
    assert_eq!(again["skipped"].as_array().unwrap().len(), 3);
    assert_eq!(again["skipped"][0]["code"], "exists");
    let kept = all.join("2026-05-08-statement.pdf");
    std::fs::write(&kept, b"%PDF-1.4 edited by hand").unwrap();
    let again = json_of(uhc(&dir).args(["documents", "download", "--all", "-o"]).arg(&all));
    assert_eq!(again["count"], 1);
    assert_eq!(again["skipped"].as_array().unwrap().len(), 2);
    let beside = PathBuf::from(again["items"][0]["path"].as_str().unwrap());
    assert!(beside
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("_2026-05-08-statement.pdf"));
    assert_eq!(std::fs::read(&kept).unwrap(), b"%PDF-1.4 edited by hand");
    let replaced = json_of(
        uhc(&dir)
            .args(["documents", "download", "--all", "--overwrite", "-o"])
            .arg(&all),
    );
    assert_eq!(replaced["count"], 3);
    assert_eq!(
        std::fs::read(&kept).unwrap(),
        std::fs::read(dir.join("2026-05-08-statement.pdf")).unwrap()
    );

    uhc(&dir).args(["documents", "download", "abc"]).assert().code(2);
    uhc(&dir)
        .args(["documents", "download", "ffffffffffff"])
        .assert()
        .code(4);
}

#[test]
fn a_missing_pdftotext_says_how_to_install_it() {
    let dir = statements(&["2026-03-14-statement.txt"]);
    let out = uhc(&dir)
        .env("UHC_PDFTOTEXT", "/nonexistent/pdftotext")
        .args(["import", "check", "--json"])
        .assert()
        .code(1);
    let e: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(e["error"]["message"].as_str().unwrap().contains("poppler"));
}

#[cfg(unix)]
#[test]
fn a_pdftotext_that_fails_skips_the_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = statements(&["2026-03-14-statement.txt"]);
    let bin = tmp("bin").join("pdftotext");
    std::fs::write(&bin, "#!/bin/sh\nexit 3\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let v = json_of(uhc(&dir).env("UHC_PDFTOTEXT", &bin).args(["import", "check"]));
    assert_eq!(v["statements"], 0);
    assert_eq!(v["skipped"]["no_text"], 1);
    assert_eq!(v["ok"], false, "nothing read is not ok");
    // With no statement there is no member.
    uhc(&dir)
        .env("UHC_PDFTOTEXT", &bin)
        .args(["records", "export", "--patient", "alex", "--ndjson"])
        .assert()
        .code(4);
}

#[test]
fn an_empty_folder_lists_nothing() {
    let dir = tmp("empty");
    let v = json_of(uhc(&dir).args(["patients", "list"]));
    assert!(v["items"].as_array().unwrap().is_empty());
    let v = json_of(uhc(&dir).args(["documents", "list"]));
    assert!(v["items"].as_array().unwrap().is_empty());
    uhc(&dir).args(["claims", "list"]).assert().success().stdout("");
}

#[test]
fn a_statement_read_with_issues_is_noted_in_the_stream() {
    need_pdftotext!();
    let dir = statements(&["damaged-statement.txt"]);
    let v = json_of(uhc(&dir).args(["patients", "list"]));
    let id = v["items"][0]["id"].as_str().unwrap().to_string();
    let out = uhc(&dir)
        .args(["records", "export", "--patient", &id, "--ndjson"])
        .assert()
        .success();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("1 statement(s) read with issues"), "{stderr}");
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let trailer: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(trailer["complete"], true);
    let skip = &trailer["skipped"][0];
    assert_eq!(skip["code"], "statement_issues");
    assert!(skip["reason"].as_str().unwrap().contains("claim_total_mismatch"));
}

#[test]
fn statements_that_cannot_be_read_are_an_upstream_error() {
    need_pdftotext!();
    let dir = tmp("layout");
    let text = "SYNTHETIC TEST DATA - NOT A REAL STATEMENT\nUnitedHealthcare    Explanation of Benefits\n\nSTD-EOB\nPage 1 of 1\n";
    std::fs::write(dir.join("s.pdf"), uhc::synth::text_pdf(text)).unwrap();
    let out = uhc(&dir).args(["claims", "list", "--json"]).assert().code(5);
    let e: Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("layout may have changed"));
    // The check and the documents still work: they are how to look into it.
    let v = json_of(uhc(&dir).args(["import", "check"]));
    assert_eq!(v["issues"]["no_claims"], 1);
    let v = json_of(uhc(&dir).args(["documents", "list"]));
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
}

#[test]
fn a_reprocessed_claim_is_its_latest_statement() {
    need_pdftotext!();
    let dir = statements(&["2026-05-08-statement.txt"]);
    let later = fixture_text("2026-05-08-statement.txt")
        .replace("05/08/2026", "06/15/2026")
        .replace("$25.00", "$30.00");
    std::fs::write(dir.join("reprocessed.pdf"), uhc::synth::text_pdf(&later)).unwrap();
    let v = json_of(uhc(&dir).args(["claims", "list"]));
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["versions"], 2);
    assert_eq!(items[0]["statement_date"], "2026-06-15");
    assert_eq!(items[0]["copay"]["amount"], "30.00");
    let check = json_of(uhc(&dir).args(["import", "check"]));
    assert_eq!(check["reprocessed_claims"], 1);
    // The stream carries the claim once, and both statements.
    let out = uhc(&dir)
        .args(["records", "export", "--patient", "sam", "--ndjson"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let trailer: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(trailer["counts"]["ExplanationOfBenefit"], 1);
    assert_eq!(trailer["counts"]["DocumentReference"], 2);
    assert_eq!(trailer["counts"]["Binary"], 2);
    let older = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["resourceType"] == "DocumentReference" && r["date"] == "2026-05-08T00:00:00Z")
        .expect("the superseded statement is carried");
    assert_eq!(
        older["context"]["related"][0]["reference"],
        "ExplanationOfBenefit/cSYNC00000004"
    );
}

#[test]
fn the_same_pdf_saved_twice_is_one_statement() {
    need_pdftotext!();
    let dir = statements(&["2026-05-08-statement.txt"]);
    std::fs::copy(
        dir.join("2026-05-08-statement.pdf"),
        dir.join("2026-05-08-statement (1).pdf"),
    )
    .unwrap();
    let v = json_of(uhc(&dir).args(["import", "check"]));
    assert_eq!(v["pdfs"], 2);
    assert_eq!(v["statements"], 1);
}

#[test]
fn one_real_statement_in_the_folder_refuses_every_export_to_a_file() {
    need_pdftotext!();
    // Addressed to Alex, unmarked, carrying only Sam's claim: Alex has no
    // claim on it, yet his export would draw on it.
    let dir = statements(GOOD);
    let text = fixture_text("2026-05-08-statement.txt")
        .replacen(uhc::eob::SYNTHETIC_MARKER, "", 1)
        .replace("SAM EXAMPLE", "ALEX EXAMPLE")
        .replace("Relationship:     SP", "Relationship:     EE");
    std::fs::write(dir.join("unmarked.pdf"), uhc::synth::text_pdf(&text)).unwrap();
    let alex = member_id(&dir, "Alex");
    let target = tmp("refused");
    uhc(&dir)
        .args(["records", "export", "--patient", &alex, "-o"])
        .arg(&target)
        .assert()
        .code(2);
    assert!(!target.join("records.ndjson").exists());
}

#[cfg(unix)]
#[test]
fn an_export_file_is_private_and_replaced_safely() {
    use std::os::unix::fs::PermissionsExt;
    need_pdftotext!();
    let dir = statements(GOOD);
    let alex = member_id(&dir, "Alex");
    let out = tmp("private");
    let path = out.join("records.ndjson");
    // A world-readable file, and then a link, where the export goes.
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    uhc(&dir)
        .args(["records", "export", "--patient", &alex, "-o"])
        .arg(&out)
        .assert()
        .success();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let elsewhere = tmp("elsewhere").join("target.txt");
    std::fs::write(&elsewhere, "untouched").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    uhc(&dir)
        .args(["records", "export", "--patient", &alex, "-o"])
        .arg(&out)
        .assert()
        .success();
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "untouched");
    assert!(!std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
}

#[test]
fn an_ambiguous_first_name_names_ids_only() {
    need_pdftotext!();
    let dir = statements(&["2026-03-14-statement.txt", "damaged-statement.txt"]);
    let out = uhc(&dir)
        .args(["records", "export", "--patient", "alex", "--ndjson"])
        .assert()
        .code(4);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("names 2 members"), "{stderr}");
    assert!(!stderr.contains("Alex ("), "{stderr}");
}

#[cfg(unix)]
#[test]
fn a_closed_pipe_ends_the_export_quietly() {
    use std::os::unix::process::ExitStatusExt;
    need_pdftotext!();
    let dir = statements(GOOD);
    let alex = member_id(&dir, "Alex");
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("uhc"))
        .env("XDG_CONFIG_HOME", tmp("cfg"))
        .arg("--dir")
        .arg(&dir)
        .args(["records", "export", "--patient", &alex, "--ndjson"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert_ne!(
        out.status.code(),
        Some(101),
        "panicked: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success() || out.status.signal() == Some(13) || out.status.code() == Some(1));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
}

#[test]
fn a_statement_missing_required_values_says_so_in_fhir() {
    need_pdftotext!();
    let dir = tmp("absent");
    let text = fixture_text("damaged-statement.txt")
        .replace("   Provider: EXAMPLE CLINIC", "")
        .replace("As of 06/01/2026", "")
        .replace("05/20/2026", "");
    std::fs::write(dir.join("s.pdf"), uhc::synth::text_pdf(&text)).unwrap();
    let v = json_of(uhc(&dir).args(["patients", "list"]));
    let id = v["items"][0]["id"].as_str().unwrap().to_string();
    let out = uhc(&dir)
        .args(["records", "export", "--patient", &id, "--ndjson"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let eob = lines
        .iter()
        .find(|r| r["resourceType"] == "ExplanationOfBenefit")
        .unwrap();
    let absent = "http://hl7.org/fhir/StructureDefinition/data-absent-reason";
    assert_eq!(eob["provider"]["extension"][0]["url"], absent);
    assert_eq!(eob["_created"]["extension"][0]["valueCode"], "unknown");
    // No member ID printed: no UHC identifier is made up.
    let patient = lines.iter().find(|r| r["resourceType"] == "Patient").unwrap();
    assert!(patient.get("identifier").is_none());
    let coverage = lines.iter().find(|r| r["resourceType"] == "Coverage").unwrap();
    assert!(coverage.get("subscriberId").is_none());
}

#[test]
fn a_statement_with_two_members_claims_is_a_document_of_each() {
    need_pdftotext!();
    let dir = statements(&["2026-03-14-statement.txt"]);
    let doc_of = |label: &str| {
        let id = member_id(&dir, label);
        let out = uhc(&dir)
            .args(["records", "export", "--patient", &id, "--ndjson"])
            .assert()
            .success();
        let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
        let doc: Value = text
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|r| r["resourceType"] == "DocumentReference")
            .unwrap();
        (id, doc)
    };
    let (alex, a) = doc_of("Alex");
    let (jordan, j) = doc_of("Jordan");
    assert_ne!(a["id"], j["id"]);
    assert_eq!(a["subject"]["reference"], format!("Patient/{alex}"));
    assert_eq!(j["subject"]["reference"], format!("Patient/{jordan}"));
}

#[test]
fn processing_notes_and_network_status_use_valid_codes() {
    need_pdftotext!();
    let dir = statements(&["2026-04-30-statement.txt"]);
    let alex = member_id(&dir, "Alex");
    let out = uhc(&dir)
        .args(["records", "export", "--patient", &alex, "--ndjson"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let eob: Value = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["resourceType"] == "ExplanationOfBenefit")
        .unwrap();
    for note in eob["processNote"].as_array().unwrap() {
        assert_eq!(note["type"], "print");
    }
    let status = &eob["adjudication"][0];
    assert_eq!(status["category"]["coding"][0]["code"], "benefitpaymentstatus");
    assert_eq!(status["reason"]["coding"][0]["code"], "outofnetwork");
}
