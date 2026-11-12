//! Write the synthetic EOB fixtures as PDFs, for dogfooding `uhc` on
//! synthetic data: `cargo run --example synth-eob -- <DIR>`.

use std::path::Path;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: synth-eob <DIR>");
        std::process::exit(2);
    });
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eob");
    std::fs::create_dir_all(&out).expect("create the output directory");
    let mut n = 0;
    for entry in std::fs::read_dir(&src).expect("read the fixtures") {
        let path = entry.expect("a fixture").path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read a fixture");
        let name = path.file_stem().and_then(|s| s.to_str()).expect("a file name");
        std::fs::write(Path::new(&out).join(format!("{name}.pdf")), uhc::synth::text_pdf(&text)).expect("write a PDF");
        n += 1;
    }
    eprintln!("wrote {n} synthetic PDFs to {out}");
}
