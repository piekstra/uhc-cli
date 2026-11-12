//! What the household's statements add up to: the members they name, each
//! claim once (the latest statement's version), and the latest plan balances
//! per person and plan year.

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cents::Cents;
use crate::eob::{Amounts, Claim, ClaimLine, Limit, Network, Statement, Whom};

/// One statement file and what was read from it.
#[derive(Debug, Clone)]
pub struct StatementFile {
    /// The document id: the first 16 hex digits of the file's SHA-256.
    pub id: String,
    pub sha256: String,
    pub path: std::path::PathBuf,
    pub bytes: u64,
    pub statement: Statement,
}

impl StatementFile {
    pub fn new(path: std::path::PathBuf, content: &[u8], statement: Statement) -> Self {
        let sha256 = hex(&Sha256::digest(content));
        StatementFile {
            id: sha256[..16].to_string(),
            sha256,
            path,
            bytes: content.len() as u64,
            statement,
        }
    }
}

/// A member the statements name: the statement's addressee, or a family
/// member with a claim or balances on one.
#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub id: String,
    pub organization: &'static str,
    pub organization_name: &'static str,
    /// The first name as the statements print it, in title case.
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship_code: Option<String>,
    pub member_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
    pub claims: u64,
    pub statements: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_statement: Option<String>,
}

pub const ORGANIZATION: &str = "uhc";
pub const ORGANIZATION_NAME: &str = "UnitedHealthcare";

/// A claim as of the latest statement that carries it.
#[derive(Debug, Clone)]
pub struct ClaimRecord {
    pub id: String,
    pub claim_number: Option<String>,
    pub patient: String,
    pub label: String,
    pub member_id: String,
    pub claim: Claim,
    pub totals: Amounts,
    pub service_start: Option<String>,
    pub service_end: Option<String>,
    pub statement_date: Option<String>,
    pub document: String,
    /// How many statements carry this claim (more than one: reprocessed).
    pub versions: u64,
    pub codes: BTreeMap<String, String>,
}

/// The latest plan balances for one person (or the family) and plan year.
#[derive(Debug, Clone)]
pub struct Balances {
    /// A member id, or `family:<member id>`.
    pub patient: String,
    pub label: String,
    pub member_id: String,
    pub family: bool,
    pub plan_year: Option<i32>,
    pub as_of: Option<String>,
    pub network: Network,
    pub limit: Limit,
    pub maximum: Cents,
    pub used: Cents,
    pub remaining: Cents,
    pub met: bool,
    pub document: String,
}

/// The patient id for a member: stable across statements, carries no name.
pub fn patient_id(member_id: &str, first_name: &str) -> String {
    let key = format!(
        "{}|{}",
        member_id.trim().to_ascii_uppercase(),
        first_name.trim().to_ascii_uppercase()
    );
    format!("m{}", &hex(&Sha256::digest(key.as_bytes()))[..12])
}

pub fn family_id(member_id: &str) -> String {
    format!("family-{}", &patient_id(member_id, "")[1..])
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `ALEX` to `Alex`.
pub fn title(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut c = lower.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// UHC's relationship code to FHIR's subscriber-relationship code.
pub fn relationship(code: &str) -> Option<&'static str> {
    match code.to_ascii_uppercase().as_str() {
        "EE" | "SUB" | "SELF" => Some("self"),
        "SP" | "SPO" | "SPOUSE" => Some("spouse"),
        "DP" => Some("common"),
        "CH" | "CHD" | "DEP" | "SN" | "DT" | "SON" | "DAU" => Some("child"),
        _ => None,
    }
}

/// Everything the statements say.
#[derive(Debug, Clone, Default)]
pub struct Book {
    pub files: Vec<StatementFile>,
}

impl Book {
    /// Statements oldest first, ties by document id, so "latest" is the last.
    pub fn new(mut files: Vec<StatementFile>) -> Self {
        files.sort_by(|a, b| {
            (a.statement.statement_date.as_deref(), &a.id).cmp(&(b.statement.statement_date.as_deref(), &b.id))
        });
        Book { files }
    }

    fn member_id(f: &StatementFile) -> String {
        f.statement
            .member
            .member_id
            .clone()
            .unwrap_or_else(|| format!("unknown-{}", &f.id[..8]))
    }

    pub fn members(&self) -> Vec<Member> {
        let mut out: BTreeMap<String, Member> = BTreeMap::new();
        let claims = self.claims();
        for f in &self.files {
            let st = &f.statement;
            let mid = Self::member_id(f);
            let addressee = st
                .member
                .name
                .as_deref()
                .and_then(|n| n.split_whitespace().next())
                .map(str::to_string);
            let mut names: Vec<String> = addressee.iter().cloned().collect();
            names.extend(st.claims.iter().map(|c| c.patient.clone()));
            names.extend(st.balances.iter().filter_map(|b| match &b.whom {
                Whom::Person(n) => Some(n.clone()),
                Whom::Family => None,
            }));
            for name in names {
                let id = patient_id(&mid, &name);
                let m = out.entry(id.clone()).or_insert_with(|| Member {
                    id: id.clone(),
                    organization: ORGANIZATION,
                    organization_name: ORGANIZATION_NAME,
                    label: title(&name),
                    relationship: None,
                    relationship_code: None,
                    member_id: mid.clone(),
                    group_number: None,
                    group_name: None,
                    claims: 0,
                    statements: 0,
                    latest_statement: None,
                });
                m.group_number = st.member.group_number.clone().or(m.group_number.take());
                m.group_name = st.member.group_name.clone().or(m.group_name.take());
                if addressee.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&name)) {
                    if let Some(code) = &st.member.relationship {
                        m.relationship_code = Some(code.clone());
                        m.relationship = relationship(code).map(str::to_string);
                    }
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            for name in st
                .claims
                .iter()
                .map(|c| c.patient.clone())
                .chain(st.balances.iter().filter_map(|b| match &b.whom {
                    Whom::Person(n) => Some(n.clone()),
                    Whom::Family => None,
                }))
                .chain(addressee.clone())
            {
                let id = patient_id(&mid, &name);
                if seen.insert(id.clone()) {
                    if let Some(m) = out.get_mut(&id) {
                        m.statements += 1;
                        if st.statement_date > m.latest_statement {
                            m.latest_statement = st.statement_date.clone();
                        }
                    }
                }
            }
        }
        for c in &claims {
            if let Some(m) = out.get_mut(&c.patient) {
                m.claims += 1;
            }
        }
        out.into_values().collect()
    }

    /// Each claim once, as its latest statement prints it, newest service first.
    pub fn claims(&self) -> Vec<ClaimRecord> {
        let mut by_key: BTreeMap<String, ClaimRecord> = BTreeMap::new();
        for f in &self.files {
            let st = &f.statement;
            let mid = Self::member_id(f);
            for (n, c) in st.claims.iter().enumerate() {
                let key = match &c.claim_number {
                    Some(num) => format!("c{}", sanitize(num)),
                    None => format!("x{}{n:02}", &f.id[..10]),
                };
                let totals = c
                    .total
                    .unwrap_or_else(|| c.lines.iter().map(|l| l.amounts).fold(Amounts::default(), |a, b| a + b));
                let (start, end) = service_range(&c.lines);
                let versions = by_key.get(&key).map(|r| r.versions + 1).unwrap_or(1);
                let codes = c
                    .lines
                    .iter()
                    .flat_map(|l| l.codes.iter())
                    .filter_map(|code| st.codes.get(code).map(|t| (code.clone(), t.clone())))
                    .collect();
                by_key.insert(
                    key.clone(),
                    ClaimRecord {
                        id: key,
                        claim_number: c.claim_number.clone(),
                        patient: patient_id(&mid, &c.patient),
                        label: title(&c.patient),
                        member_id: mid.clone(),
                        claim: c.clone(),
                        totals,
                        service_start: start,
                        service_end: end,
                        statement_date: st.statement_date.clone(),
                        document: f.id.clone(),
                        versions,
                        codes,
                    },
                );
            }
        }
        let mut out: Vec<ClaimRecord> = by_key.into_values().collect();
        out.sort_by(|a, b| (b.service_start.as_deref(), &b.id).cmp(&(a.service_start.as_deref(), &a.id)));
        out
    }

    /// The balances one statement printed for `first_name` and the family.
    pub fn balances_in(&self, document: &str, first_name: &str) -> Vec<Balances> {
        let Some(f) = self.files.iter().find(|f| f.id == document) else {
            return Vec::new();
        };
        let mid = Self::member_id(f);
        let mut out = Vec::new();
        for t in &f.statement.balances {
            let (patient, label, family) = match &t.whom {
                Whom::Person(n) if n.eq_ignore_ascii_case(first_name) => (patient_id(&mid, n), title(n), false),
                Whom::Person(_) => continue,
                Whom::Family => (family_id(&mid), "Family".to_string(), true),
            };
            for r in &t.rows {
                out.push(Balances {
                    patient: patient.clone(),
                    label: label.clone(),
                    member_id: mid.clone(),
                    family,
                    plan_year: t.plan_year,
                    as_of: t.as_of.clone(),
                    network: r.network,
                    limit: r.limit,
                    maximum: r.maximum,
                    used: r.used,
                    remaining: r.remaining,
                    met: r.met,
                    document: f.id.clone(),
                });
            }
        }
        out
    }

    /// The full name a statement addressed to this member prints.
    pub fn full_name(&self, patient: &str) -> Option<String> {
        self.files.iter().rev().find_map(|f| {
            let name = f.statement.member.name.as_deref()?;
            let first = name.split_whitespace().next()?;
            (patient_id(&Self::member_id(f), first) == patient).then(|| name.to_string())
        })
    }

    /// The latest balances per person (and family), plan year, network and limit.
    pub fn balances(&self) -> Vec<Balances> {
        let mut latest: BTreeMap<(String, Option<i32>, Network, Limit), Balances> = BTreeMap::new();
        for f in &self.files {
            let mid = Self::member_id(f);
            for t in &f.statement.balances {
                let (patient, label, family) = match &t.whom {
                    Whom::Person(n) => (patient_id(&mid, n), title(n), false),
                    Whom::Family => (family_id(&mid), "Family".to_string(), true),
                };
                for r in &t.rows {
                    let key = (patient.clone(), t.plan_year, r.network, r.limit);
                    let newer = latest.get(&key).is_none_or(|b| t.as_of >= b.as_of);
                    if newer {
                        latest.insert(
                            key,
                            Balances {
                                patient: patient.clone(),
                                label: label.clone(),
                                member_id: mid.clone(),
                                family,
                                plan_year: t.plan_year,
                                as_of: t.as_of.clone(),
                                network: r.network,
                                limit: r.limit,
                                maximum: r.maximum,
                                used: r.used,
                                remaining: r.remaining,
                                met: r.met,
                                document: f.id.clone(),
                            },
                        );
                    }
                }
            }
        }
        let mut out: Vec<Balances> = latest.into_values().collect();
        out.sort_by(|a, b| {
            (b.plan_year, a.family, &a.label, a.network, a.limit).cmp(&(
                a.plan_year,
                b.family,
                &b.label,
                b.network,
                b.limit,
            ))
        });
        out
    }
}

fn service_range(lines: &[ClaimLine]) -> (Option<String>, Option<String>) {
    let starts = lines.iter().filter_map(|l| l.date.clone());
    let ends = lines
        .iter()
        .filter_map(|l| l.date_end.clone().or_else(|| l.date.clone()));
    (starts.min(), ends.max())
}

/// Keep ids to `[A-Za-z0-9.-]`, as FHIR ids require.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
        .take(60)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patient_ids_are_stable_and_carry_no_name() {
        let a = patient_id("SYNM000001", "ALEX");
        assert_eq!(a, patient_id(" synm000001 ", "alex"));
        assert_ne!(a, patient_id("SYNM000001", "SAM"));
        assert_eq!(a.len(), 13);
        assert!(!a.to_ascii_uppercase().contains("ALEX"));
        assert!(family_id("SYNM000001").starts_with("family-"));
    }

    #[test]
    fn relationship_codes_map_to_fhir() {
        assert_eq!(relationship("EE"), Some("self"));
        assert_eq!(relationship("sp"), Some("spouse"));
        assert_eq!(relationship("CH"), Some("child"));
        assert_eq!(relationship("ZZ"), None);
    }

    #[test]
    fn names_title_case() {
        assert_eq!(title("JORDAN"), "Jordan");
        assert_eq!(title(""), "");
    }
}
