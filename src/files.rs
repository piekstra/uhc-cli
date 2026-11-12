//! The files source: the EOB statement PDFs the member downloaded from
//! myuhc.com into one folder. Nothing here touches the network; text comes
//! from the local `pdftotext` (poppler), bounded in time and size.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pk_cli_core::CliError;

use crate::eob;
use crate::model::{Book, StatementFile};

/// Largest file read.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Largest text taken from `pdftotext`.
const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;
const PDFTOTEXT_TIMEOUT: Duration = Duration::from_secs(60);
/// Folders below the import folder that are searched (one level).
const MAX_DEPTH: usize = 1;

/// A file in the folder that is not read as a statement.
#[derive(Debug, Clone)]
pub struct SkippedFile {
    pub path: PathBuf,
    /// `not_pdf`, `not_eob`, `too_large`, `unreadable`, `no_text`.
    pub code: &'static str,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub book: Book,
    pub skipped: Vec<SkippedFile>,
    /// PDFs found, statements or not.
    pub pdfs: u64,
}

/// The `pdftotext` to run: `UHC_PDFTOTEXT`, else the one on `PATH`.
fn pdftotext() -> String {
    std::env::var("UHC_PDFTOTEXT")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "pdftotext".into())
}

/// Read every statement in `dir`.
pub fn scan(dir: &Path) -> Result<Scan, CliError> {
    let meta = std::fs::metadata(dir)
        .map_err(|e| CliError::NotFound(format!("the import folder {} cannot be read: {e}", dir.display())))?;
    if !meta.is_dir() {
        return Err(CliError::Usage(format!("{} is not a folder", dir.display())));
    }
    let mut paths = Vec::new();
    collect(dir, 0, &mut paths)?;
    paths.sort();

    let tool = pdftotext();
    let mut scan = Scan::default();
    let mut files = Vec::new();
    for path in paths {
        scan.pdfs += 1;
        let skip = |code| SkippedFile {
            path: path.clone(),
            code,
        };
        let size = std::fs::symlink_metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE_BYTES {
            scan.skipped.push(skip("too_large"));
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            scan.skipped.push(skip("unreadable"));
            continue;
        };
        if pk_cli_documents::verify_pdf(&bytes).is_err() {
            scan.skipped.push(skip("not_pdf"));
            continue;
        }
        let text = match extract_text(&tool, &path)? {
            Some(t) if !t.trim().is_empty() => t,
            _ => {
                scan.skipped.push(skip("no_text"));
                continue;
            }
        };
        if !eob::is_uhc_eob(&text) {
            scan.skipped.push(skip("not_eob"));
            continue;
        }
        let statement = eob::parse(&text);
        files.push(StatementFile::new(path, &bytes, statement));
    }
    // The same PDF saved twice is one statement.
    files.sort_by(|a, b| a.id.cmp(&b.id).then(a.path.cmp(&b.path)));
    files.dedup_by(|a, b| a.sha256 == b.sha256);
    scan.book = Book::new(files);
    Ok(scan)
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), CliError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| CliError::NotFound(format!("the import folder {} cannot be read: {e}", dir.display())))?;
    for entry in entries.flatten() {
        let path = entry.path();
        // Symlinks are not followed: the folder is the member's downloads.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if depth < MAX_DEPTH && !hidden {
                collect(&path, depth + 1, out)?;
            }
        } else if meta.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        {
            out.push(path);
        }
    }
    Ok(())
}

/// The text of one PDF. `Ok(None)` when `pdftotext` cannot read it;
/// an error when `pdftotext` cannot be run at all.
fn extract_text(tool: &str, path: &Path) -> Result<Option<String>, CliError> {
    let mut child = Command::new(tool)
        .args(["-layout", "-enc", "UTF-8", "-q"])
        .arg(path)
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            CliError::Other(format!(
                "cannot run `{tool}` ({e}); it reads the statements' text. Install poppler (`brew install poppler`) \
                 or point UHC_PDFTOTEXT at a pdftotext"
            ))
        })?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stdout).take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + PDFTOTEXT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let buf = reader.join().unwrap_or_default();
    match status {
        Some(s) if s.success() && buf.len() <= MAX_TEXT_BYTES => Ok(Some(String::from_utf8_lossy(&buf).into_owned())),
        _ => Ok(None),
    }
}
