//! Statements mapped into the FHIR shapes `medrec` reads: CARIN Blue Button
//! `ExplanationOfBenefit` and `Coverage`, with `Patient`, the payer
//! `Organization`, and each statement PDF as `DocumentReference` + `Binary`.
//!
//! The mapping follows CARIN BB 2.0's element choices and code systems where
//! the statement carries the value. It does not claim the profiles
//! (`meta.profile`): a statement does not print the claim type or the
//! diagnoses CARIN requires. Every resource says where it came from in
//! `meta.source` and a `meta.tag`.

use base64::Engine;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::cents::Cents;
use crate::eob::{Amounts, Limit, Network};
use crate::model::{hex, Balances, ClaimRecord, Member, StatementFile, ORGANIZATION, ORGANIZATION_NAME};

pub const META_SOURCE: &str = "uhc:eob-pdf";
pub const TAG_SYSTEM: &str = "https://github.com/piekstra/uhc-cli/fhir/tag";
pub const TAG_MAPPED: &str = "mapped-from-eob-statement";
pub const MEMBER_ID_SYSTEM: &str = "https://www.uhc.com/fhir/member-id";
pub const CLAIM_NUMBER_SYSTEM: &str = "https://www.uhc.com/fhir/claim-number";
pub const BENEFIT_TYPE_SYSTEM: &str = "https://github.com/piekstra/uhc-cli/fhir/benefit-type";

const HL7_ADJUDICATION: &str = "http://terminology.hl7.org/CodeSystem/adjudication";
const C4BB_ADJUDICATION: &str = "http://hl7.org/fhir/us/carin-bb/CodeSystem/C4BBAdjudication";
const C4BB_SUPPORTING_INFO: &str = "http://hl7.org/fhir/us/carin-bb/CodeSystem/C4BBSupportingInfoType";
const C4BB_CONTRACTING: &str = "http://hl7.org/fhir/us/carin-bb/CodeSystem/C4BBPayerProviderContractingStatus";

fn meta() -> Value {
    json!({
        "source": META_SOURCE,
        "tag": [{ "system": TAG_SYSTEM, "code": TAG_MAPPED, "display": "Mapped by uhc from a UnitedHealthcare EOB statement" }],
    })
}

fn coding(system: &str, code: &str) -> Value {
    json!({ "coding": [{ "system": system, "code": code }] })
}

fn money(c: Cents) -> Value {
    json!({ "value": c.fhir_value(), "currency": "USD" })
}

pub fn organization() -> Value {
    json!({
        "resourceType": "Organization",
        "id": ORGANIZATION,
        "meta": meta(),
        "active": true,
        "type": [coding("http://terminology.hl7.org/CodeSystem/organization-type", "pay")],
        "name": ORGANIZATION_NAME,
    })
}

/// `full_name` is the "Member/Patient" a statement addressed to this member
/// prints, when there is one.
pub fn patient(m: &Member, full_name: Option<&str>) -> Value {
    let mut name = Map::new();
    if let Some(full) = full_name {
        name.insert("text".into(), full.into());
    }
    name.insert("given".into(), json!([m.label]));
    json!({
        "resourceType": "Patient",
        "id": m.id,
        "meta": meta(),
        "identifier": [{
            "type": coding("http://terminology.hl7.org/CodeSystem/v2-0203", "MB"),
            "system": MEMBER_ID_SYSTEM,
            "value": m.member_id,
        }],
        "name": [Value::Object(name)],
    })
}

pub fn coverage_id(m: &Member) -> String {
    let key = format!("{}|{}|{}", m.member_id, m.id, m.group_number.as_deref().unwrap_or(""));
    format!("cov-{}", &hex(&Sha256::digest(key.as_bytes()))[..12])
}

pub fn coverage(m: &Member) -> Value {
    let mut v = json!({
        "resourceType": "Coverage",
        "id": coverage_id(m),
        "meta": meta(),
        "status": "active",
    });
    if m.group_number.is_some() {
        v["type"] = json!({ "text": "Group health plan" });
    }
    v["subscriberId"] = m.member_id.clone().into();
    v["beneficiary"] = json!({ "reference": format!("Patient/{}", m.id) });
    if let Some(rel) = &m.relationship {
        v["relationship"] = coding("http://terminology.hl7.org/CodeSystem/subscriber-relationship", rel);
    }
    v["payor"] = json!([{ "reference": format!("Organization/{ORGANIZATION}"), "display": ORGANIZATION_NAME }]);
    if let Some(group) = &m.group_number {
        let mut class = json!({
            "type": coding("http://terminology.hl7.org/CodeSystem/coverage-class", "group"),
            "value": group,
        });
        if let Some(name) = &m.group_name {
            class["name"] = name.clone().into();
        }
        v["class"] = json!([class]);
    }
    v
}

/// The adjudication entries of a line or a claim, CARIN's categories.
fn adjudication(a: &Amounts) -> Vec<Value> {
    let rows: [(&str, &str, Cents); 9] = [
        (HL7_ADJUDICATION, "submitted", a.billed),
        (C4BB_ADJUDICATION, "discount", a.saved),
        (HL7_ADJUDICATION, "eligible", a.allowed),
        (HL7_ADJUDICATION, "benefit", a.plan_paid),
        (HL7_ADJUDICATION, "deductible", a.deductible),
        (HL7_ADJUDICATION, "copay", a.copay),
        (C4BB_ADJUDICATION, "coinsurance", a.coinsurance),
        (C4BB_ADJUDICATION, "noncovered", a.not_covered),
        (C4BB_ADJUDICATION, "memberliability", a.you_owe),
    ];
    rows.iter()
        .map(|(system, code, amount)| json!({ "category": coding(system, code), "amount": money(*amount) }))
        .collect()
}

pub fn document_id(file_id: &str) -> String {
    format!("doc-{file_id}")
}

pub fn binary_id(file_id: &str) -> String {
    format!("bin-{file_id}")
}

/// One claim. `balances` are the plan balances its statement printed for
/// this patient and the family.
pub fn explanation_of_benefit(c: &ClaimRecord, coverage: &str, balances: &[&Balances]) -> Value {
    let mut v = json!({
        "resourceType": "ExplanationOfBenefit",
        "id": c.id,
        "meta": meta(),
    });
    if let Some(num) = &c.claim_number {
        v["identifier"] = json!([{
            "type": coding("http://hl7.org/fhir/us/carin-bb/CodeSystem/C4BBIdentifierType", "uc"),
            "system": CLAIM_NUMBER_SYSTEM,
            "value": num,
        }]);
    }
    v["status"] = "active".into();
    v["type"] = json!({ "text": "Claim (the statement does not print its type)" });
    v["use"] = "claim".into();
    v["patient"] = json!({ "reference": format!("Patient/{}", c.patient) });
    if let Some(start) = &c.service_start {
        let mut period = json!({ "start": start });
        if let Some(end) = &c.service_end {
            period["end"] = end.clone().into();
        }
        v["billablePeriod"] = period;
    }
    if let Some(created) = &c.statement_date {
        v["created"] = created.clone().into();
    }
    v["insurer"] = json!({ "reference": format!("Organization/{ORGANIZATION}"), "display": ORGANIZATION_NAME });
    if let Some(p) = &c.claim.provider {
        v["provider"] = json!({ "display": p });
    }
    v["outcome"] = "complete".into();

    let mut info = vec![json!({
        "sequence": 1,
        "category": coding("http://terminology.hl7.org/CodeSystem/claiminformationcategory", "info"),
        "valueReference": { "reference": format!("DocumentReference/{}", document_id(&c.document)) },
    })];
    if let Some(network) = c.claim.network {
        let status = match network {
            Network::In => "contracted",
            Network::Out => "non-contracted",
        };
        info.push(json!({
            "sequence": info.len() + 1,
            "category": coding(C4BB_SUPPORTING_INFO, "performingnetworkcontractingstatus"),
            "code": coding(C4BB_CONTRACTING, status),
        }));
    }
    if let Some(acct) = &c.claim.account_number {
        info.push(json!({
            "sequence": info.len() + 1,
            "category": coding(C4BB_SUPPORTING_INFO, "patientaccountnumber"),
            "valueString": acct,
        }));
    }
    v["supportingInfo"] = Value::Array(info);
    v["insurance"] = json!([{ "focal": true, "coverage": { "reference": format!("Coverage/{coverage}") } }]);

    let notes: Vec<&String> = c.codes.keys().collect();
    let items: Vec<Value> = c
        .claim
        .lines
        .iter()
        .enumerate()
        .map(|(n, l)| {
            let mut item = json!({ "sequence": n + 1, "productOrService": { "text": l.description } });
            match (&l.date, &l.date_end) {
                (Some(d), Some(e)) => item["servicedPeriod"] = json!({ "start": d, "end": e }),
                (Some(d), None) => item["servicedDate"] = d.clone().into(),
                _ => {}
            }
            let numbers: Vec<usize> = l
                .codes
                .iter()
                .filter_map(|code| notes.iter().position(|k| *k == code).map(|p| p + 1))
                .collect();
            if !numbers.is_empty() {
                item["noteNumber"] = json!(numbers);
            }
            item["net"] = money(l.amounts.billed);
            item["adjudication"] = Value::Array(adjudication(&l.amounts));
            item
        })
        .collect();
    if !items.is_empty() {
        v["item"] = Value::Array(items);
    }
    v["total"] = Value::Array(adjudication(&c.totals));
    if !notes.is_empty() {
        v["processNote"] = Value::Array(
            c.codes
                .iter()
                .enumerate()
                .map(|(n, (code, text))| json!({ "number": n + 1, "type": "printed", "text": format!("{code} -- {text}") }))
                .collect(),
        );
    }
    if !balances.is_empty() {
        v["benefitBalance"] = Value::Array(benefit_balance(balances));
    }
    v
}

fn benefit_balance(balances: &[&Balances]) -> Vec<Value> {
    let mut groups: Vec<((Network, bool), Vec<Value>)> = Vec::new();
    for b in balances {
        let key = (b.network, b.family);
        let financial = json!({
            "type": match b.limit {
                Limit::Deductible => json!({ "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/benefit-type", "code": "deductible" }], "text": "Deductible" }),
                Limit::OutOfPocket => json!({ "coding": [{ "system": BENEFIT_TYPE_SYSTEM, "code": "out-of-pocket-maximum" }], "text": "Out-of-pocket limit" }),
            },
            "allowedMoney": money(b.maximum),
            "usedMoney": money(b.used),
        });
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, f)) => f.push(financial),
            None => groups.push((key, vec![financial])),
        }
    }
    groups
        .into_iter()
        .map(|((network, family), financial)| {
            json!({
                "category": coding("http://terminology.hl7.org/CodeSystem/ex-benefitcategory", "1"),
                "network": coding("http://terminology.hl7.org/CodeSystem/benefit-network", network.code()),
                "unit": coding("http://terminology.hl7.org/CodeSystem/benefit-unit", if family { "family" } else { "individual" }),
                "term": coding("http://terminology.hl7.org/CodeSystem/benefit-term", "annual"),
                "financial": financial,
            })
        })
        .collect()
}

/// The statement PDF as a document of `patient`, related to its claims.
pub fn document_reference(f: &StatementFile, patient: &str, claims: &[&ClaimRecord]) -> Value {
    let st = &f.statement;
    let mut v = json!({
        "resourceType": "DocumentReference",
        "id": document_id(&f.id),
        "meta": meta(),
        "status": "current",
        "type": { "text": "Explanation of Benefits" },
        "category": [{ "text": "EOB" }],
        "subject": { "reference": format!("Patient/{patient}") },
    });
    if let Some(d) = &st.statement_date {
        v["date"] = format!("{d}T00:00:00Z").into();
    }
    v["description"] = match &st.statement_date {
        Some(d) => format!("UnitedHealthcare Explanation of Benefits, {d}"),
        None => "UnitedHealthcare Explanation of Benefits".to_string(),
    }
    .into();
    let mut attachment = json!({
        "contentType": "application/pdf",
        "url": format!("Binary/{}", binary_id(&f.id)),
        "size": f.bytes,
        "title": match &st.statement_date {
            Some(d) => format!("UHC EOB {d}.pdf"),
            None => format!("UHC EOB {}.pdf", f.id),
        },
    });
    if let Some(d) = &st.statement_date {
        attachment["creation"] = d.clone().into();
    }
    v["content"] = json!([{ "attachment": attachment }]);
    let mut context = json!({
        "related": claims.iter().map(|c| json!({ "reference": format!("ExplanationOfBenefit/{}", c.id) })).collect::<Vec<_>>(),
    });
    if let Some((s, e)) = &st.service_period {
        context["period"] = json!({ "start": s, "end": e });
    }
    v["context"] = context;
    v
}

pub fn binary(f: &StatementFile, content: &[u8]) -> Value {
    json!({
        "resourceType": "Binary",
        "id": binary_id(&f.id),
        "meta": meta(),
        "contentType": "application/pdf",
        "data": base64::engine::general_purpose::STANDARD.encode(content),
    })
}
