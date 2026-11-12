//! Reading a UnitedHealthcare Explanation of Benefits statement from the text
//! `pdftotext -layout` makes of it.
//!
//! The layout is UHC's standard EOB ("STD-EOB" in its page footer), as UHC's
//! public explainer shows it (docs/api.md): a header naming the member, a
//! claim summary, "Plan balances" tables (one per person and one for the
//! family, in and out of network, deductible and out-of-pocket limit), then
//! "Claim detail for <first name>" blocks whose service lines carry nine
//! amounts, and the claim processing codes. The reader is line-based and
//! tolerant: what it cannot read becomes an [`Issue`] (a code, never a value),
//! so a check over real statements can report what failed without printing
//! anything from them.

use std::collections::BTreeMap;

use crate::cents::Cents;

/// One statement, as read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statement {
    /// The date printed in the page footer, else the plan balances' "As of".
    pub statement_date: Option<String>,
    pub member: Member,
    /// "Services in this statement occurred between <date> - <date>".
    pub service_period: Option<(String, String)>,
    pub provider_billed: Option<Cents>,
    pub total_owed: Option<Cents>,
    pub balances: Vec<BalanceTable>,
    pub claims: Vec<Claim>,
    /// Claim processing code to its explanation.
    pub codes: BTreeMap<String, String>,
    /// The statement says it is synthetic test data (fixtures only).
    pub synthetic: bool,
    pub issues: Vec<Issue>,
}

/// The statement's header: the person it is addressed to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Member {
    pub name: Option<String>,
    pub member_id: Option<String>,
    /// UHC's relationship code: `EE` (employee, the subscriber), `SP`, `CH`, …
    pub relationship: Option<String>,
    pub group_name: Option<String>,
    pub group_number: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Network {
    In,
    Out,
}

impl Network {
    pub fn code(self) -> &'static str {
        match self {
            Network::In => "in",
            Network::Out => "out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Limit {
    Deductible,
    OutOfPocket,
}

impl Limit {
    pub fn code(self) -> &'static str {
        match self {
            Limit::Deductible => "deductible",
            Limit::OutOfPocket => "out_of_pocket",
        }
    }
}

/// Whose balances a table holds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Whom {
    /// A person, by the first name the statement prints.
    Person(String),
    Family,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BalanceTable {
    pub as_of: Option<String>,
    pub plan_year: Option<i32>,
    pub whom: Whom,
    pub rows: Vec<BalanceRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BalanceRow {
    pub network: Network,
    pub limit: Limit,
    pub maximum: Cents,
    pub used: Cents,
    pub remaining: Cents,
    pub met: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Claim {
    /// The first name after "Claim detail for".
    pub patient: String,
    pub provider: Option<String>,
    pub account_number: Option<String>,
    pub network: Option<Network>,
    pub claim_number: Option<String>,
    pub lines: Vec<ClaimLine>,
    pub total: Option<Amounts>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimLine {
    pub description: String,
    pub codes: Vec<String>,
    /// Date of service, ISO; a range keeps its first day in `date` and its
    /// last in `date_end`.
    pub date: Option<String>,
    pub date_end: Option<String>,
    pub amounts: Amounts,
}

/// The nine amounts of a service line or a claim total, in the statement's
/// column order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Amounts {
    pub billed: Cents,
    pub saved: Cents,
    pub allowed: Cents,
    pub plan_paid: Cents,
    pub deductible: Cents,
    pub copay: Cents,
    pub coinsurance: Cents,
    pub not_covered: Cents,
    pub you_owe: Cents,
}

impl Amounts {
    fn from_slice(v: &[Cents]) -> Option<Amounts> {
        let [billed, saved, allowed, plan_paid, deductible, copay, coinsurance, not_covered, you_owe] =
            <[Cents; 9]>::try_from(v).ok()?;
        Some(Amounts {
            billed,
            saved,
            allowed,
            plan_paid,
            deductible,
            copay,
            coinsurance,
            not_covered,
            you_owe,
        })
    }

    pub fn fields(&self) -> [(&'static str, Cents); 9] {
        [
            ("billed", self.billed),
            ("saved", self.saved),
            ("allowed", self.allowed),
            ("plan_paid", self.plan_paid),
            ("deductible", self.deductible),
            ("copay", self.copay),
            ("coinsurance", self.coinsurance),
            ("not_covered", self.not_covered),
            ("you_owe", self.you_owe),
        ]
    }
}

impl std::ops::Add for Amounts {
    type Output = Amounts;
    fn add(self, o: Amounts) -> Amounts {
        Amounts {
            billed: self.billed + o.billed,
            saved: self.saved + o.saved,
            allowed: self.allowed + o.allowed,
            plan_paid: self.plan_paid + o.plan_paid,
            deductible: self.deductible + o.deductible,
            copay: self.copay + o.copay,
            coinsurance: self.coinsurance + o.coinsurance,
            not_covered: self.not_covered + o.not_covered,
            you_owe: self.you_owe + o.you_owe,
        }
    }
}

/// Something the reader could not read. Codes only: an issue never carries a
/// value from the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Issue {
    NoStatementDate,
    NoMemberId,
    NoClaims,
    ClaimWithoutNumber,
    ClaimWithoutLines,
    ClaimWithoutTotal,
    /// The service lines do not add up to the claim's printed total.
    ClaimTotalMismatch,
    /// A line in a claim carries amounts, but not nine of them.
    AmountsUnreadable,
    LineWithoutDate,
    BalanceRowUnreadable,
    NoPlanYear,
}

impl Issue {
    pub fn code(self) -> &'static str {
        match self {
            Issue::NoStatementDate => "no_statement_date",
            Issue::NoMemberId => "no_member_id",
            Issue::NoClaims => "no_claims",
            Issue::ClaimWithoutNumber => "claim_without_number",
            Issue::ClaimWithoutLines => "claim_without_lines",
            Issue::ClaimWithoutTotal => "claim_without_total",
            Issue::ClaimTotalMismatch => "claim_total_mismatch",
            Issue::AmountsUnreadable => "amounts_unreadable",
            Issue::LineWithoutDate => "line_without_date",
            Issue::BalanceRowUnreadable => "balance_row_unreadable",
            Issue::NoPlanYear => "no_plan_year",
        }
    }
}

/// The marker synthetic fixtures carry; real statements never print it.
pub const SYNTHETIC_MARKER: &str = "SYNTHETIC TEST DATA";

/// Whether `text` is a UnitedHealthcare EOB statement: UHC's name, the
/// words "Explanation of Benefits", and a statement's structure (a claim
/// detail, a balances table or the STD-EOB footer). A letter that only
/// mentions EOBs is not one.
pub fn is_uhc_eob(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let uhc = lower.contains("unitedhealthcare") || lower.contains("united healthcare") || lower.contains("myuhc.com");
    let structure = text.contains("Claim detail for") || text.contains("Balances for") || text.contains("STD-EOB");
    lower.contains("explanation of benefits") && (uhc || text.contains("STD-EOB")) && structure
}

enum Section {
    Header,
    Balances,
    Claim,
    Other,
}

/// Read one statement's text.
pub fn parse(text: &str) -> Statement {
    let mut st = Statement {
        synthetic: text.contains(SYNTHETIC_MARKER),
        ..Statement::default()
    };
    // Byte offsets below assume one-byte spaces: any other whitespace
    // (U+00A0, U+1680, U+2000…) becomes an ASCII space first.
    let text: String = text
        .chars()
        .map(|c| {
            if c.is_whitespace() && !matches!(c, '\n' | '\x0c') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let lines: Vec<&str> = text.split(['\n', '\x0c']).collect();
    let mut section = Section::Header;
    let mut as_of: Option<String> = None;
    let mut plan_year: Option<i32> = None;
    let mut footer_date_due = 0usize; // lines left in which a date is the page footer's
    let mut footer_date: Option<String> = None;
    let mut pending_date: Option<usize> = None; // index into the current claim's lines
    let mut row_network: Option<Network> = None;
    let mut claim_margin = 0usize;
    // A repeated "Claim detail for" header of a claim with no total yet: the
    // claim goes on (a page break) unless the block names another claim.
    let mut continuing = false;
    // Plain text since the last service line: a description printed above
    // its amounts, or wrapped onto further lines.
    let mut pending_text: Vec<String> = Vec::new();

    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if is_page_footer(trimmed) {
            footer_date_due = 3;
            continue;
        }
        if let Some((d, end)) = date_only(trimmed) {
            if footer_date_due > 0 && end.is_none() {
                footer_date_due = 0;
                footer_date.get_or_insert(d);
                continue;
            }
            if let (Some(idx), Some(claim)) = (pending_date, st.claims.last_mut()) {
                if let Some(l) = claim.lines.get_mut(idx) {
                    l.date = Some(d);
                    l.date_end = end;
                }
                pending_date = None;
            }
            continue;
        }
        footer_date_due = footer_date_due.saturating_sub(1);

        // Header fields can appear anywhere before the claims.
        if st.member.member_id.is_none() {
            st.member.member_id = field(line, "Member ID:").map(|v| first_word(&v));
        }
        if st.member.name.is_none() {
            st.member.name = field(line, "Member/Patient:");
        }
        if st.member.group_number.is_none() {
            st.member.group_number = field(line, "Group number:").map(|v| first_word(&v));
        }
        if st.member.group_name.is_none() {
            st.member.group_name = field(line, "Group name:");
        }
        if st.member.relationship.is_none() && line.contains("Relationship:") {
            st.member.relationship = field(line, "Relationship:")
                .map(|v| first_word(&v))
                .or_else(|| next_code_line(&lines[i + 1..]));
        }
        if st.service_period.is_none() {
            if let Some(p) = service_period(line) {
                st.service_period = Some(p);
            }
        }
        if st.provider_billed.is_none() && line.contains("Provider billed") {
            st.provider_billed = money_after(line, "Provider billed");
        }
        if st.total_owed.is_none() && line.contains("Your total amount owed") {
            st.total_owed = money_after(line, "Your total amount owed");
        }
        for (code, text) in processing_codes(line) {
            st.codes.entry(code).or_insert(text);
        }

        if let Some((date, year)) = as_of_line(trimmed) {
            as_of = Some(date);
            plan_year = year;
            continue;
        }
        if let Some(whom) = field(line, "Balances for") {
            let whom = first_word(&whom);
            let whom = if whom.eq_ignore_ascii_case("FAMILY") {
                Whom::Family
            } else {
                Whom::Person(whom)
            };
            st.balances.push(BalanceTable {
                as_of: as_of.clone(),
                plan_year,
                whom,
                rows: Vec::new(),
            });
            section = Section::Balances;
            row_network = None;
            continue;
        }
        if let Some(who) = field(line, "Claim detail for") {
            let patient = first_word(&who);
            continuing = st
                .claims
                .last()
                .is_some_and(|c| c.total.is_none() && !c.lines.is_empty() && c.patient.eq_ignore_ascii_case(&patient));
            if !continuing {
                st.claims.push(Claim {
                    patient,
                    ..Claim::default()
                });
                pending_date = None;
            }
            claim_margin = line.len() - line.trim_start().len();
            section = Section::Claim;
            pending_text.clear();
            continue;
        }
        if trimmed.starts_with("Definitions of Key Terms") || trimmed.starts_with("Explanation of your claim") {
            section = Section::Other;
            continue;
        }

        match section {
            Section::Balances => {
                let lower = line.to_ascii_lowercase();
                let limit = if lower.contains("deductible") {
                    Limit::Deductible
                } else if lower.contains("out-of-pocket") {
                    Limit::OutOfPocket
                } else {
                    continue;
                };
                if lower.contains("out-of-network") {
                    row_network = Some(Network::Out);
                } else if lower.contains("network") {
                    row_network = Some(Network::In);
                }
                let table = st.balances.last_mut().expect("a balances section has a table");
                match (row_network, balance_amounts(line)) {
                    (Some(network), Some((maximum, used, remaining, met))) => table.rows.push(BalanceRow {
                        network,
                        limit,
                        maximum,
                        used,
                        remaining,
                        met,
                    }),
                    _ => st.issues.push(Issue::BalanceRowUnreadable),
                }
            }
            Section::Claim => {
                let provider = field(line, "Provider:");
                let number = field(line, "Claim number:").map(|v| first_word(&v));
                if continuing {
                    // A continued block that names a different claim is that claim.
                    let last = st.claims.last().expect("a claim section has a claim");
                    let differs = |new: &Option<String>, old: &Option<String>| matches!((new, old), (Some(n), Some(o)) if !n.eq_ignore_ascii_case(o));
                    if differs(&number, &last.claim_number) || differs(&provider, &last.provider) {
                        let patient = last.patient.clone();
                        st.claims.push(Claim {
                            patient,
                            ..Claim::default()
                        });
                        pending_date = None;
                        continuing = false;
                    }
                }
                let claim = st.claims.last_mut().expect("a claim section has a claim");
                if claim.provider.is_none() {
                    claim.provider = provider;
                }
                if claim.account_number.is_none() {
                    claim.account_number = field(line, "Patient account number:").map(|v| first_word(&v));
                }
                if claim.claim_number.is_none() {
                    claim.claim_number = number;
                }
                if claim.network.is_none() {
                    if let Some(s) = field(line, "Status:") {
                        let s = s.to_ascii_lowercase();
                        claim.network = if s.starts_with("out-of-network") {
                            Some(Network::Out)
                        } else if s.starts_with("network") || s.starts_with("in-network") {
                            Some(Network::In)
                        } else {
                            None
                        };
                    }
                }
                let amounts = money_tokens(line);
                if amounts.is_empty() {
                    if is_description_text(trimmed) {
                        pending_text.push(trimmed.split_whitespace().collect::<Vec<_>>().join(" "));
                    } else {
                        pending_text.clear();
                    }
                    continue;
                }
                continuing = false;
                let values: Vec<Cents> = amounts.iter().map(|(_, c)| *c).collect();
                let prefix = &line[..amounts[0].0];
                if prefix.trim_start().starts_with("Total amount") {
                    // Text after the last line's amounts is the rest of its description.
                    if let Some(last) = claim.lines.last_mut() {
                        for t in pending_text.drain(..) {
                            last.description = format!("{} {t}", last.description).trim().to_string();
                        }
                    }
                    pending_text.clear();
                    match Amounts::from_slice(&values) {
                        Some(a) => claim.total = Some(a),
                        None => st.issues.push(Issue::AmountsUnreadable),
                    }
                    continue;
                }
                let Some(a) = Amounts::from_slice(&values) else {
                    pending_text.clear();
                    st.issues.push(Issue::AmountsUnreadable);
                    continue;
                };
                let (inline, codes) = describe(prefix, claim_margin);
                let mut parts: Vec<String> = std::mem::take(&mut pending_text);
                if !inline.is_empty() {
                    parts.push(inline);
                }
                let description = parts.join(" ");
                let (description, date, date_end) = take_inline_date(&description);
                claim.lines.push(ClaimLine {
                    description,
                    codes,
                    date: date.clone(),
                    date_end,
                    amounts: a,
                });
                pending_date = if date.is_none() {
                    Some(claim.lines.len() - 1)
                } else {
                    None
                };
            }
            Section::Header | Section::Other => {}
        }
    }

    st.statement_date = footer_date.or_else(|| st.balances.iter().find_map(|b| b.as_of.clone()));
    check(&mut st);
    st
}

fn check(st: &mut Statement) {
    let mut issues = Vec::new();
    if st.statement_date.is_none() {
        issues.push(Issue::NoStatementDate);
    }
    if st.member.member_id.is_none() {
        issues.push(Issue::NoMemberId);
    }
    if st.claims.is_empty() {
        issues.push(Issue::NoClaims);
    }
    if st.balances.iter().any(|b| b.plan_year.is_none()) {
        issues.push(Issue::NoPlanYear);
    }
    for c in &st.claims {
        if c.claim_number.is_none() {
            issues.push(Issue::ClaimWithoutNumber);
        }
        if c.lines.is_empty() {
            issues.push(Issue::ClaimWithoutLines);
        }
        if c.lines.iter().any(|l| l.date.is_none()) {
            issues.push(Issue::LineWithoutDate);
        }
        match c.total {
            None => issues.push(Issue::ClaimWithoutTotal),
            Some(t) => {
                let sum = c.lines.iter().map(|l| l.amounts).fold(Amounts::default(), |a, b| a + b);
                if !c.lines.is_empty() && sum != t {
                    issues.push(Issue::ClaimTotalMismatch);
                }
            }
        }
    }
    st.issues.extend(issues);
}

/// `Page 2 of 4`.
fn is_page_footer(t: &str) -> bool {
    let mut w = t.split_whitespace();
    matches!(
        (w.next(), w.next(), w.next(), w.next(), w.next()),
        (Some("Page"), Some(a), Some("of"), Some(b), None)
            if a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
    )
}

/// A line holding only a date (`09/01/2023`) or a range (`09/01/2023 - 09/03/2023`).
fn date_only(t: &str) -> Option<(String, Option<String>)> {
    if let Some(d) = mdy(t) {
        return Some((d, None));
    }
    let (a, b) = t.split_once('-')?;
    Some((mdy(a.trim())?, Some(mdy(b.trim())?)))
}

/// `MM/DD/YYYY` to ISO.
pub fn mdy(s: &str) -> Option<String> {
    let mut p = s.split('/');
    let (m, d, y) = (p.next()?, p.next()?, p.next()?);
    if p.next().is_some() || m.len() != 2 || d.len() != 2 || y.len() != 4 {
        return None;
    }
    let (m, d, y): (u32, u32, u32) = (m.parse().ok()?, d.parse().ok()?, y.parse().ok()?);
    valid(y, m, d)
}

fn valid(y: u32, m: u32, d: u32) -> Option<String> {
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => return None,
    };
    if !(1900..=2200).contains(&y) || d == 0 || d > days {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// `Sep. 1, 2023`, `Sep 1, 2023` or `September 1, 2023` to ISO.
pub fn long_date(s: &str) -> Option<String> {
    let mut w = s.split_whitespace();
    let (mon, day, year) = (w.next()?, w.next()?, w.next()?);
    if w.next().is_some() {
        return None;
    }
    let mon = mon.trim_end_matches('.').to_ascii_lowercase();
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let m = MONTHS
        .iter()
        .position(|p| mon.starts_with(p) && (mon.len() == 3 || mon.len() >= 4))? as u32
        + 1;
    let d: u32 = day.trim_end_matches(',').parse().ok()?;
    let y: u32 = year.parse().ok()?;
    valid(y, m, d)
}

/// The value after `label` on `line`, up to a gap of two spaces.
fn field(line: &str, label: &str) -> Option<String> {
    let at = line.find(label)?;
    let rest = line[at + label.len()..].trim_start();
    let value = match rest.find("  ") {
        Some(end) => &rest[..end],
        None => rest,
    }
    .trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn first_word(s: &str) -> String {
    s.split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches([',', '.'])
        .to_string()
}

/// A relationship code printed alone on a following line.
fn next_code_line(rest: &[&str]) -> Option<String> {
    rest.iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(6)
        .find(|l| (2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_uppercase()))
        .map(str::to_string)
}

fn service_period(line: &str) -> Option<(String, String)> {
    let at = line.find("between ")?;
    let rest = &line[at + "between ".len()..];
    let rest = match rest.find("  ") {
        Some(end) => &rest[..end],
        None => rest,
    };
    let (a, b) = rest.split_once(" - ")?;
    Some((
        long_date(a.trim()).or_else(|| mdy(a.trim()))?,
        long_date(b.trim()).or_else(|| mdy(b.trim()))?,
    ))
}

fn money_after(line: &str, label: &str) -> Option<Cents> {
    let at = line.find(label)?;
    money_tokens(&line[at + label.len()..]).first().map(|(_, c)| *c)
}

/// Every amount on a line, with the byte offset it starts at.
fn money_tokens(line: &str) -> Vec<(usize, Cents)> {
    let mut out = Vec::new();
    let mut start = None;
    // A lone `-` before an amount (`- $85.00`) is its sign.
    let mut minus: Option<usize> = None;
    for (i, ch) in line.char_indices().chain(std::iter::once((line.len(), ' '))) {
        match (ch.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                let tok = &line[s..i];
                if tok == "-" {
                    minus = Some(s);
                } else {
                    if tok.contains('$') {
                        if let Some(c) = Cents::parse(tok) {
                            match minus {
                                Some(m) => out.push((m, Cents(-c.0))),
                                None => out.push((s, c)),
                            }
                        }
                    }
                    minus = None;
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// A balances row's three values: maximum, progress so far, remaining ("Met"
/// for a limit reached).
fn balance_amounts(line: &str) -> Option<(Cents, Cents, Cents, bool)> {
    let money = money_tokens(line);
    let met = line.split_whitespace().any(|w| w == "Met");
    match (money.as_slice(), met) {
        ([(_, max), (_, used), (_, rem)], false) => Some((*max, *used, *rem, rem.0 == 0)),
        ([(_, max), (_, used)], true) => Some((*max, *used, Cents(0), true)),
        _ => None,
    }
}

/// `As of 09/21/2023 for plan year 2023`.
fn as_of_line(t: &str) -> Option<(String, Option<i32>)> {
    let rest = t.strip_prefix("As of ")?;
    let date = mdy(rest.split_whitespace().next()?)?;
    let year = rest
        .find("plan year ")
        .and_then(|at| rest[at + "plan year ".len()..].split_whitespace().next())
        .and_then(|y| y.parse().ok());
    Some((date, year))
}

/// `UG -- YOU HAVE RECEIVED A DISCOUNT ...`, possibly two per line (two columns).
fn processing_codes(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut search = 0;
    while let Some(rel) = line[search..].find(" -- ") {
        let at = search + rel;
        let code_start = line[..at]
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map(|(p, c)| p + c.len_utf8())
            .unwrap_or(0);
        let code = &line[code_start..at];
        let gap_before = code_start == 0 || line[..code_start].ends_with("  ") || line[..code_start].trim().is_empty();
        let text_start = at + " -- ".len();
        let text_end = line[text_start..]
            .find("   ")
            .map(|e| text_start + e)
            .unwrap_or(line.len());
        if gap_before
            && (2..=4).contains(&code.len())
            && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            out.push((code.to_string(), line[text_start..text_end].trim().to_string()));
        }
        search = text_end.max(text_start);
    }
    out
}

/// Split a service line's text into its description and its processing codes,
/// which sit in their own column. A segment of codes alone on the line (the
/// description printed above) is told from a short description by where it
/// starts: descriptions start at the claim's left margin (`margin`, the
/// indentation of its "Claim detail for" line).
fn describe(prefix: &str, margin: usize) -> (String, Vec<String>) {
    let mut segments: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for part in prefix.split("  ") {
        if !part.trim().is_empty() {
            let lead = part.len() - part.trim_start().len();
            segments.push((at + lead, part.trim()));
        }
        at += part.len() + 2;
    }
    let is_codes = |s: &str| {
        s.split([' ', ','])
            .filter(|t| !t.is_empty())
            .all(|t| (2..=4).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
    };
    let mut description = Vec::new();
    let mut codes = Vec::new();
    for (n, (start, s)) in segments.iter().enumerate() {
        let in_code_column = n > 0 || *start >= margin + 12;
        if in_code_column && is_codes(s) {
            codes.extend(s.split([' ', ',']).filter(|t| !t.is_empty()).map(str::to_string));
        } else {
            description.push(*s);
        }
    }
    (description.join(" "), codes)
}

/// A description that ends in its date of service.
fn take_inline_date(description: &str) -> (String, Option<String>, Option<String>) {
    let words: Vec<&str> = description.split_whitespace().collect();
    if let Some(last) = words.last() {
        if let Some(d) = mdy(last) {
            return (words[..words.len() - 1].join(" "), Some(d), None);
        }
    }
    (words.join(" "), None, None)
}

/// Whether a line in a claim block without amounts is description text:
/// not a labelled field, a page footer's form code, or a row of column
/// headings.
fn is_description_text(t: &str) -> bool {
    if t.contains(':') || t == "STD-EOB" || mdy(t).is_some() {
        return false;
    }
    // The footer's form code, e.g. `000000000000000-A-S-PS-E`.
    if !t.contains(' ') && t.len() >= 15 && t.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    const HEADINGS: [&str; 12] = [
        "amount billed",
        "your plan discount",
        "amount allowed",
        "plan paid",
        "deductible",
        "copay",
        "coinsurance",
        "not covered",
        "you owe",
        "date of service",
        "description",
        "processing code",
    ];
    HEADINGS.iter().filter(|h| lower.contains(*h)).count() < 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_both_ways() {
        assert_eq!(mdy("09/21/2023").as_deref(), Some("2023-09-21"));
        assert_eq!(mdy("02/30/2023"), None);
        assert_eq!(mdy("9/21/2023"), None);
        assert_eq!(long_date("Sep. 1, 2023").as_deref(), Some("2023-09-01"));
        assert_eq!(long_date("September 30, 2026").as_deref(), Some("2026-09-30"));
        assert_eq!(long_date("Sept. 3, 2026").as_deref(), Some("2026-09-03"));
        assert_eq!(long_date("Foo 3, 2026"), None);
    }

    #[test]
    fn fields_stop_at_a_column_gap() {
        let line = "   Provider: EXAMPLE CLINIC        Patient account number: ACCT-0001";
        assert_eq!(field(line, "Provider:").as_deref(), Some("EXAMPLE CLINIC"));
        assert_eq!(field(line, "Patient account number:").as_deref(), Some("ACCT-0001"));
        assert_eq!(field("Relationship:", "Relationship:"), None);
    }

    #[test]
    fn service_lines_split_description_codes_and_nine_amounts() {
        let line = "   OFFICE VISIT                 UG 2Q   $100.00  $40.00  $60.00  $30.00  $0.00  $30.00  $0.00  $0.00  $30.00";
        let money = money_tokens(line);
        assert_eq!(money.len(), 9);
        let (d, codes) = describe(&line[..money[0].0], 3);
        assert_eq!(d, "OFFICE VISIT");
        assert_eq!(codes, vec!["UG", "2Q"]);
        let above = "                                     2Q B1  $1.00";
        let (d, codes) = describe(&above[..above.find('$').unwrap()], 3);
        assert_eq!(d, "");
        assert_eq!(codes, vec!["2Q", "B1"]);
        let (d, codes) = describe("   MRI   ", 3);
        assert_eq!(d, "MRI");
        assert!(codes.is_empty());
    }

    #[test]
    fn processing_codes_read_two_columns() {
        let line = "  2Q -- THE AMOUNT ALLOWED WAS REDUCED.            UG -- YOU RECEIVED A NETWORK DISCOUNT.";
        let codes = processing_codes(line);
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].0, "2Q");
        assert_eq!(codes[1], ("UG".into(), "YOU RECEIVED A NETWORK DISCOUNT.".into()));
    }

    #[test]
    fn balance_rows_read_met_and_remaining() {
        assert_eq!(
            balance_amounts("  Network   Deductible   $1,000.00   $1,000.00   Met"),
            Some((Cents(100_000), Cents(100_000), Cents(0), true))
        );
        assert_eq!(
            balance_amounts("     Out-of-pocket limit   $6,500.00   $4,996.35   $1,503.65"),
            Some((Cents(650_000), Cents(499_635), Cents(150_365), false))
        );
        assert_eq!(balance_amounts("Deductible: The amount you could owe"), None);
    }

    #[test]
    fn page_footers_are_recognized() {
        assert!(is_page_footer("Page 2 of 4"));
        assert!(!is_page_footer("Page two of four"));
        assert!(!is_page_footer("Page 2 of 4 continued"));
    }

    #[test]
    fn only_uhc_eobs_are_taken() {
        assert!(is_uhc_eob(
            "UnitedHealthcare\nExplanation of Benefits\nClaim detail for ALEX\n"
        ));
        assert!(!is_uhc_eob(
            "UnitedHealthcare\nThis notice is not an Explanation of Benefits statement.\n"
        ));
        assert!(is_uhc_eob(
            "THIS IS NOT A BILL. This Explanation of Benefits (EOB)\nSTD-EOB\n"
        ));
        assert!(!is_uhc_eob("Explanation of Benefits\nSome Other Insurer\n"));
        assert!(!is_uhc_eob("UnitedHealthcare premium invoice\n"));
    }
}
