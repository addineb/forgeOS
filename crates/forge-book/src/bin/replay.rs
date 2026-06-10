//! `forge-book-replay` - fold a *.forge stream into an OrderBook and print the
//! resulting top of book plus a deterministic state hash. Used to validate book
//! reconstruction against real data on the box.
//!
//! Usage: forge-book-replay <path.forge> [progress_every]

use std::process::ExitCode;

use forge_book::OrderBook;
use forge_data::ForgeReader;

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: forge-book-replay <path.forge> [progress_every]")?;

    let reader = ForgeReader::open(&path).map_err(|e| format!("open {path}: {e}"))?;
    let mut book = OrderBook::new();
    let mut decoded: u64 = 0;
    for rec in reader.records() {
        let ev = rec.to_event().map_err(|e| format!("decode at #{decoded}: {e}"))?;
        book.apply(&ev).map_err(|e| format!("apply at #{decoded}: {e}"))?;
        decoded += 1;
    }

    println!("file            {path}");
    println!("events          {decoded}");
    println!("level mutations {}", book.applied());
    println!("crossed trims   {}", book.crossed_trims());
    println!("bid levels      {}", book.bid_levels());
    println!("ask levels      {}", book.ask_levels());
    match (book.best_bid(), book.best_ask()) {
        (Some((b, bq)), Some((a, aq))) => {
            println!("best bid        {} x {}", b.to_f64(), bq.to_f64());
            println!("best ask        {} x {}", a.to_f64(), aq.to_f64());
            if let Some(s) = book.spread() {
                println!("spread          {}", s.to_f64());
            }
            if let Some(m) = book.mid() {
                println!("mid             {}", m.to_f64());
            }
        }
        _ => println!("top of book     (one side empty)"),
    }
    println!("crossed?        {}", book.is_crossed());
    println!("last_ts (ns)    {}", book.last_ts());
    println!("state_hash      {:#018x}", book.state_hash());
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}