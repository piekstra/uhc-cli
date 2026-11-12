//! A minimal PDF writer for synthetic statements: monospaced text, one line
//! per row, so `pdftotext -layout` gives back the columns it was written with.
//! Tests and `examples/synth-eob.rs` use it to turn the synthetic fixtures
//! into PDFs; nothing in the CLI writes a PDF.

const LINES_PER_PAGE: usize = 100;
const FONT_SIZE: f32 = 4.5;
const LEADING: f32 = 7.5;

/// A PDF holding `text` in Courier, ASCII only (anything else becomes `?`).
pub fn text_pdf(text: &str) -> Vec<u8> {
    let lines: Vec<String> = text.lines().map(escape).collect();
    let pages: Vec<&[String]> = if lines.is_empty() {
        vec![&[][..]]
    } else {
        lines.chunks(LINES_PER_PAGE).collect()
    };

    let mut objects: Vec<String> = Vec::new();
    // 1 catalog, 2 pages, 3 font, then (page, content) pairs.
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into());
    for (i, page) in pages.iter().enumerate() {
        let mut content = format!("BT /F1 {FONT_SIZE} Tf {LEADING} TL 12 780 Td\n");
        for line in page.iter() {
            content.push_str(&format!("({line}) Tj T*\n"));
        }
        content.push_str("ET\n");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        ));
        objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()));
    }

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (n, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", n + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn escape(line: &str) -> String {
    line.chars()
        .map(|c| match c {
            '(' => "\\(".to_string(),
            ')' => "\\)".to_string(),
            '\\' => "\\\\".to_string(),
            c if c.is_ascii() && !c.is_ascii_control() => c.to_string(),
            _ => "?".to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pdf_is_well_formed_enough_to_verify() {
        let pdf = text_pdf("hello (world)\nsecond line");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let s = String::from_utf8(pdf).unwrap();
        assert!(s.contains("(hello \\(world\\)) Tj"));
        assert!(s.contains("/Count 1"));
    }
}
