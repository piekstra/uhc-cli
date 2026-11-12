//! The `records-stream/v1` writer: a header line, one FHIR resource per
//! line, and a `records-stream-end/v1` trailer with counts and skips.
//!
//! Field for field the format medrec-cli reads and medical-common's
//! `pk_med_records` defines, plus the optional header `method` agreed with
//! both (`"portal"`: resources a CLI mapped from a member site or files
//! downloaded there). This module goes when that crate is pinned.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;

use pk_cli_core::CliError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STREAM_SCHEMA: &str = "records-stream/v1";
pub const END_SCHEMA: &str = "records-stream-end/v1";
pub const FHIR_VERSION: &str = "4.0.1";
/// Longest resource line the reader accepts.
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub schema: String,
    pub source: SourceCli,
    pub organization: StreamOrganization,
    pub patient: StreamPatient,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    pub pulled_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<String>,
    /// How the resources were obtained; absent means `fhir-api`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCli {
    pub cli: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOrganization {
    pub id: String,
    pub name: String,
    pub fhir_base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamPatient {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
}

/// One thing the export could not carry, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skip {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    pub reason: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trailer {
    pub schema: String,
    pub complete: bool,
    pub counts: BTreeMap<String, u64>,
    pub skipped: Vec<Skip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

pub struct RecordsWriter<W: Write> {
    out: W,
    counts: BTreeMap<String, u64>,
    skipped: Vec<Skip>,
    seen: HashSet<(String, String)>,
}

fn io(e: std::io::Error) -> CliError {
    CliError::Other(format!("writing the records stream: {e}"))
}

impl<W: Write> RecordsWriter<W> {
    /// Write the header line.
    pub fn new(mut out: W, header: &Header) -> Result<Self, CliError> {
        let line = serde_json::to_string(header).map_err(|e| CliError::Other(e.to_string()))?;
        writeln!(out, "{line}").map_err(io)?;
        Ok(RecordsWriter {
            out,
            counts: BTreeMap::new(),
            skipped: Vec::new(),
            seen: HashSet::new(),
        })
    }

    /// Write one resource unless one of its type and id was written already.
    /// Returns whether it was written.
    pub fn resource(&mut self, r: &Value) -> Result<bool, CliError> {
        let rtype = r["resourceType"].as_str().unwrap_or_default().to_string();
        let id = r["id"].as_str().unwrap_or_default().to_string();
        if rtype.is_empty() {
            return Err(CliError::Other("a resource without resourceType".into()));
        }
        if !id.is_empty() && !self.seen.insert((rtype.clone(), id.clone())) {
            return Ok(false);
        }
        let line = serde_json::to_string(r).map_err(|e| CliError::Other(e.to_string()))?;
        if line.len() > MAX_LINE_BYTES {
            self.skip(Skip {
                resource_type: Some(rtype),
                id: Some(id).filter(|i| !i.is_empty()),
                search: None,
                reason: format!("{} bytes, over the {MAX_LINE_BYTES}-byte line limit", line.len()),
                code: "too_large".into(),
            });
            return Ok(false);
        }
        self.out.write_all(line.as_bytes()).map_err(io)?;
        self.out.write_all(b"\n").map_err(io)?;
        *self.counts.entry(rtype).or_insert(0) += 1;
        Ok(true)
    }

    pub fn skip(&mut self, s: Skip) {
        if !self.skipped.contains(&s) {
            self.skipped.push(s);
        }
    }

    /// Write the trailer. `error` makes the stream incomplete.
    pub fn finish(mut self, finished_at: String, error: Option<&CliError>) -> Result<Trailer, CliError> {
        let trailer = Trailer {
            schema: END_SCHEMA.into(),
            complete: error.is_none(),
            counts: std::mem::take(&mut self.counts),
            skipped: std::mem::take(&mut self.skipped),
            finished_at: Some(finished_at),
            error: error
                .map(|e| serde_json::json!({ "code": e.code(), "message": e.message(), "exit_code": e.exit_code() })),
        };
        let line = serde_json::to_string(&trailer).map_err(|e| CliError::Other(e.to_string()))?;
        writeln!(self.out, "{line}").map_err(io)?;
        self.out.flush().map_err(io)?;
        Ok(trailer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Header {
        Header {
            schema: STREAM_SCHEMA.into(),
            source: SourceCli {
                cli: "uhc".into(),
                version: "0.0.0".into(),
            },
            organization: StreamOrganization {
                id: "uhc".into(),
                name: "UnitedHealthcare".into(),
                fhir_base: "https://member.uhc.com/myuhc".into(),
                vendor: Some("uhc".into()),
            },
            patient: StreamPatient {
                id: "m000000000001".into(),
                organization: Some("uhc".into()),
                label: None,
                relationship: Some("self".into()),
            },
            since: None,
            pulled_at: "2026-01-01T00:00:00Z".into(),
            fhir_version: Some(FHIR_VERSION.into()),
            method: Some("portal".into()),
        }
    }

    #[test]
    fn a_stream_is_header_resources_trailer_with_duplicates_dropped() {
        let mut buf = Vec::new();
        let mut w = RecordsWriter::new(&mut buf, &header()).unwrap();
        let eob = serde_json::json!({ "resourceType": "ExplanationOfBenefit", "id": "c1" });
        assert!(w.resource(&eob).unwrap());
        assert!(!w.resource(&eob).unwrap());
        let t = w.finish("2026-01-01T00:01:00Z".into(), None).unwrap();
        assert!(t.complete);
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        let head: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(head["schema"], STREAM_SCHEMA);
        assert_eq!(head["method"], "portal");
        let trailer: Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(trailer["schema"], END_SCHEMA);
        assert_eq!(trailer["counts"]["ExplanationOfBenefit"], 1);
        assert!(trailer.get("error").is_none());
    }

    #[test]
    fn a_fatal_error_ends_the_stream_incomplete() {
        let mut buf = Vec::new();
        let w = RecordsWriter::new(&mut buf, &header()).unwrap();
        let e = CliError::Upstream("pdftotext failed".into());
        let t = w.finish("2026-01-01T00:01:00Z".into(), Some(&e)).unwrap();
        assert!(!t.complete);
        let text = String::from_utf8(buf).unwrap();
        let trailer: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(trailer["complete"], false);
        assert_eq!(trailer["error"]["exit_code"], 5);
    }
}
