//! S&P 500 membership, used to restrict the dashboard's market-movers lists to
//! recognizable large-cap names rather than the whole-market micro-caps Yahoo's
//! predefined screeners return (a user request: "market movers has a lot of
//! companies I've literally never heard of").
//!
//! The constituent list lives in `universe/sp500.txt` (one ticker per line, in
//! Yahoo symbology so `BRK.B` is `BRK-B`; `#` comments and blank lines allowed)
//! and is embedded at compile time, so there is no runtime file IO and it ships
//! inside the binary. Refresh by editing that file and redeploying (the file's
//! header documents the one-liner that regenerates it).

use std::collections::HashSet;
use std::sync::LazyLock;

/// The raw constituent list, embedded at build time.
const SP500_RAW: &str = include_str!("../universe/sp500.txt");

/// The membership set, parsed once on first use. Tickers are uppercased so the
/// lookup is case-insensitive; blank lines and `#` comments are skipped.
static SP500: LazyLock<HashSet<String>> = LazyLock::new(|| {
    SP500_RAW
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_uppercase)
        .collect()
});

/// Whether `ticker` is an S&P 500 constituent (case-insensitive).
pub fn is_member(ticker: &str) -> bool {
    SP500.contains(&ticker.to_uppercase())
}

/// How many constituents are loaded — for a boot log / sanity check.
pub fn count() -> usize {
    SP500.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_full_roster() {
        // The S&P 500 hovers around 500-505 names; guard against an empty or
        // truncated embed (e.g. a botched refresh).
        assert!(count() >= 490, "expected ~500 constituents, got {}", count());
    }

    #[test]
    fn known_members_and_non_members() {
        assert!(is_member("AAPL"));
        assert!(is_member("aapl"), "lookup is case-insensitive");
        assert!(is_member("BRK-B"), "dotted tickers stored in Yahoo dash form");
        assert!(is_member("JPM"));
        // A real ticker that is not in the index, and obvious junk.
        assert!(!is_member("SPCX"), "a delisted micro-cap fund is not a member");
        assert!(!is_member("NOTATICKER"));
    }
}
