fn main() {
    if let Err(error) = cash_counter::plugin::run_stdio() {
        eprintln!("Cash Ledger MCP server: {error}");
        std::process::exit(1);
    }
}
