//! Measure Tidy's matching accuracy against a folder you've already sorted.
//!
//!   cargo run --release --example accuracy -- [FOLDER] [--with-learned] [--misses N]
//!
//! FOLDER defaults to your configured destination (layout: FOLDER/<Subject>/…/file).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(tidy_lib::eval::run(&args));
}
