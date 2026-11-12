//! The repository carries no personal data; runtime output may.
//!
//! Walks every tracked file (`git ls-files`) for the *shapes* personal data
//! takes (phone numbers, long id numbers, street addresses, email
//! addresses) rather than a list of real values, which would publish what
//! it exists to keep out. Then checks the fixtures positively: every
//! identity-bearing field a statement prints holds one of the dummy shapes
//! `tests/fixtures/README.md` lists.

use std::process::Command;

fn root() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

fn tracked_files() -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files"])
        .current_dir(root())
        .output()
        .expect("git ls-files");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

fn digits_run(s: &str, min: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            cur.push(c);
        } else {
            if cur.len() >= min {
                out.push(cur.clone());
            }
            cur.clear();
        }
    }
    out
}

/// A digit run is a dummy when it is all one digit, zeros then one digit
/// (`00000001`), or a 555 phone number: `555…`, or any area code with the
/// 555-0100 to 555-0199 lines reserved for fiction.
fn is_dummy_number(n: &str) -> bool {
    let first = n.chars().next().unwrap();
    let head = &n[..n.len() - 1];
    let local = n.strip_prefix('1').filter(|r| r.len() == 10).unwrap_or(n);
    let fictional = local.len() == 10 && &local[3..8] == "55501";
    n.chars().all(|c| c == first)
        || head.chars().all(|c| c == '0')
        || n.starts_with("555")
        || n.starts_with("1555")
        || fictional
}

const PUBLIC_EMAIL_DOMAINS: &[&str] = &["example.com", "uhc.com", "optum.com", "flex.optum.com"];

/// The personal-data shapes on one line, each as `kind value`.
fn scan_line(line: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let phoneish = line.replace(['(', ')', '-', '.', ' '], "");
    for n in digits_run(line, 7)
        .into_iter()
        .chain(digits_run(&phoneish, 10).into_iter().filter(|n| n.len() <= 11))
    {
        if !is_dummy_number(&n) {
            findings.push(format!("number {n}"));
        }
    }
    for word in line.split(|c: char| c.is_whitespace() || "\"'<>(),;`".contains(c)) {
        if let Some((local, domain)) = word.split_once('@') {
            let domain = domain.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-'));
            let ok = local.is_empty()
                || !domain.contains('.')
                || PUBLIC_EMAIL_DOMAINS.contains(&domain)
                || domain.ends_with(".example")
                || domain.ends_with("users.noreply.github.com");
            if !ok {
                findings.push(format!("email {word}"));
            }
        }
    }
    let lower = line.to_ascii_lowercase();
    for suffix in [" way", " st", " ave", " blvd", " dr", " ln", " ct", " rd"] {
        if let Some(pos) = lower.find(suffix) {
            let has_number = lower[..pos]
                .split_whitespace()
                .rev()
                .take(4)
                .any(|w| w.len() >= 3 && w.chars().all(|c| c.is_ascii_digit()));
            if has_number && !lower.contains("example st") {
                findings.push(format!("address-like `{}`", line.trim()));
            }
        }
    }
    findings
}

#[test]
fn tracked_files_carry_no_personal_data() {
    let files = tracked_files();
    if files.is_empty() {
        // Not a git checkout (a packaged crate): nothing to scan.
        return;
    }
    let mut findings = Vec::new();
    for file in files {
        if file.ends_with(".lock") || file.starts_with("LICENSE") || file == "tests/no_personal_data.rs" {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(format!("{}/{file}", root())) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for f in scan_line(line) {
                findings.push(format!("{file}:{}: {f}", i + 1));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "personal-data shapes in tracked files:\n{}",
        findings.join("\n")
    );
}

#[test]
fn no_tracked_file_is_a_raw_capture_or_a_statement() {
    for file in tracked_files() {
        let lower = file.to_ascii_lowercase();
        for ext in [".pdf", ".har", ".csv", ".xlsx", ".raw.json", ".raw.html", ".ndjson"] {
            assert!(
                !lower.ends_with(ext),
                "{file} is tracked; real statements and captures never are"
            );
        }
    }
}

const PEOPLE: &[&str] = &["ALEX", "SAM", "JORDAN"];

fn is_dummy_name(full: &str) -> bool {
    match full.split_whitespace().collect::<Vec<_>>().as_slice() {
        [first, "EXAMPLE"] | [first] => PEOPLE.contains(first),
        _ => false,
    }
}

fn is_dummy_org(name: &str) -> bool {
    ["EXAMPLE", "SAMPLE", "TEST"]
        .iter()
        .any(|w| name.split_whitespace().any(|x| x == *w))
}

/// `prefix` then exactly `digits` digits.
fn shaped(value: &str, prefix: &str, digits: usize) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|d| d.len() == digits && d.chars().all(|c| c.is_ascii_digit()))
}

/// The value printed after `label` on a line, up to a two-space gap.
fn after<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let i = line.find(label)?;
    let rest = line[i + label.len()..].trim_start();
    Some(rest.split("  ").next().unwrap_or("").trim())
}

#[test]
fn identity_fields_in_fixtures_use_dummy_values() {
    let dir = format!("{}/tests/fixtures/eob", root());
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.display().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.trim_start()
                .starts_with("SYNTHETIC TEST DATA - NOT A REAL STATEMENT"),
            "{name} does not open with the synthetic marker"
        );
        for line in text.lines() {
            let mut check = |label: &str, ok: &dyn Fn(&str) -> bool| {
                if let Some(v) = after(line, label) {
                    if !v.is_empty() {
                        assert!(ok(v), "{name}: `{label}` holds `{v}`, not a dummy");
                        checked += 1;
                    }
                }
            };
            check("Member/Patient:", &is_dummy_name);
            check("Member ID:", &|v| shaped(v, "SYNM", 6));
            check("Group number:", &|v| shaped(v, "SYNG", 4));
            check("Group name:", &is_dummy_org);
            check("Claim number:", &|v| shaped(v, "SYNC", 8));
            check("Patient account number:", &|v| shaped(v, "SYNA", 4));
            check("Provider:", &is_dummy_org);
            check("Claim detail for", &|v| {
                PEOPLE.contains(&v.trim_end_matches(" (continued)"))
            });
            check("Balances for", &|v| PEOPLE.contains(&v) || v == "FAMILY");
            check("Hi,", &|v| PEOPLE.contains(&v.trim_end_matches('.')));
        }
        for n in digits_run(&text.replace(['(', ')', '-', ' '], ""), 10) {
            assert!(n.len() > 11 || is_dummy_number(&n), "{name}: phone-like {n}");
        }
    }
    assert!(
        checked >= 30,
        "only {checked} identity fields found; did the fixture layout change?"
    );
}

#[test]
fn the_scanner_flags_real_shapes_and_passes_dummies() {
    for real in [
        "call (772) 404-1234",
        "call 888-555-0200",
        "member id 9234567812",
        "mail someone@gmail.com",
        "lives at 1234 NW Oak St, Somewhere",
        "claim 7342918",
    ] {
        assert!(!scan_line(real).is_empty(), "missed: {real}");
    }
    for dummy in [
        "call (555) 010-0001",
        "customer service 888-555-0100",
        "SYNM000001 SYNC00000003",
        "000000000000000-A-S-PS-E",
        "pat@example.com",
        "100 Example St, Exampleton",
        "$16,000.00",
    ] {
        assert!(
            scan_line(dummy).is_empty(),
            "false positive: {dummy}: {:?}",
            scan_line(dummy)
        );
    }
    assert!(is_dummy_name("ALEX EXAMPLE"));
    assert!(!is_dummy_name("ALEX SMITH"));
    assert!(shaped("SYNC00000004", "SYNC", 8));
    assert!(!shaped("SYNC0000004", "SYNC", 8));
}
