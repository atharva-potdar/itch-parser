//! The ingest layer: reference data keyed by `stock_locate`.
//!
//! ITCH identifies securities by a 16-bit `stock_locate` on every order-book
//! message and only in the daily `R` (Stock Directory) broadcast does it send
//! the symbol. Anything that wants to display or route by symbol therefore has
//! to build a registry first. That mapping is not part of the wire format, so
//! it lives here rather than in [`crate::itch`].
//!
//! The registry is deliberately forgiving about the two things real captures
//! actually do:
//!
//! * a locate can be **reused** for a different symbol after a delisting, and
//! * a symbol can **move** to a different locate.
//!
//! Both are handled by evicting the stale reverse mapping and counting the
//! event, so a lookup can never return a symbol that was superseded. Neither
//! is an error: they are reference-data churn, not feed corruption.

use std::collections::HashMap;

use crate::itch::{
    Authenticity, FinancialStatusIndicator, IssueClassification, LuldReferencePriceTier,
    MarketCategory, ParsedMessage, StockDirectoryMessage, Symbol,
};

/// The reference-data view of one security, as announced by an `R` message.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // mirrors the wire format 1:1
pub struct Instrument {
    pub stock_locate: u16,
    pub symbol: Symbol,
    pub market_category: MarketCategory,
    pub financial_status_indicator: FinancialStatusIndicator,
    pub round_lot_size: u32,
    pub round_lots_only: bool,
    pub issue_classification: IssueClassification,
    /// Two raw ASCII characters; see [`StockDirectoryMessage::issue_subtype`].
    pub issue_subtype: [u8; 2],
    pub authenticity: Authenticity,
    pub short_sale_threshold_indicator: bool,
    pub ipo_flag: bool,
    pub luld_reference_price_tier: LuldReferencePriceTier,
    pub etp_flag: bool,
    pub etp_leverage_factor: u32,
    pub inverse_indicator: bool,
}

impl Instrument {
    /// Copies the reference-data fields out of an `R` message.
    #[must_use]
    pub const fn from_message(message: &StockDirectoryMessage) -> Self {
        Self {
            stock_locate: message.stock_locate,
            symbol: message.stock,
            market_category: message.market_category,
            financial_status_indicator: message.financial_status_indicator,
            round_lot_size: message.round_lot_size,
            round_lots_only: message.round_lots_only,
            issue_classification: message.issue_classification,
            issue_subtype: message.issue_subtype,
            authenticity: message.authenticity,
            short_sale_threshold_indicator: message.short_sale_threshold_indicator,
            ipo_flag: message.ipo_flag,
            luld_reference_price_tier: message.luld_reference_price_tier,
            etp_flag: message.etp_flag,
            etp_leverage_factor: message.etp_leverage_factor,
            inverse_indicator: message.inverse_indicator,
        }
    }

    /// The symbol with right padding removed.
    #[must_use]
    pub fn symbol_str(&self) -> &str {
        self.symbol.as_str()
    }
}

/// What changed when an `R` message was applied.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RegistryUpdate {
    /// A locate that was not known before.
    Inserted { locate: u16, symbol: Symbol },
    /// A refresh of an existing locate for the same symbol.
    Updated { locate: u16, symbol: Symbol },
    /// A known symbol moved to a new locate.
    Relocated {
        symbol: Symbol,
        previous_locate: u16,
    },
    /// A known locate was reassigned to a different symbol.
    Replaced {
        locate: u16,
        previous_symbol: Symbol,
        symbol: Symbol,
    },
}

/// Counters describing reference-data churn.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct RegistryStats {
    /// Locates seen for the first time.
    pub inserted: u64,
    /// Refresh messages for a locate already pointing at the same symbol.
    pub updated: u64,
    /// Symbols that moved to a new locate.
    pub relocated: u64,
    /// Locates reassigned to a different symbol.
    pub replaced: u64,
}

/// `stock_locate` → [`Instrument`], with a reverse symbol index.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    by_locate: HashMap<u16, Instrument>,
    by_symbol: HashMap<Symbol, u16>,
    stats: RegistryStats,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies an `R` message, keeping both indexes consistent.
    ///
    /// `locate` is the authoritative key — an `R` message that reuses a
    /// locate replaces whatever was there, because book messages carry the
    /// locate and would otherwise resolve to a stale symbol.
    pub fn apply(&mut self, message: &StockDirectoryMessage) -> RegistryUpdate {
        let locate = message.stock_locate;
        let symbol = message.stock;

        // If this symbol was previously registered elsewhere, that locate is
        // now stale: book messages for it would resolve to a symbol that has
        // moved away.
        let moved_from = self
            .by_symbol
            .insert(symbol, locate)
            .filter(|&previous_locate| previous_locate != locate);
        if let Some(previous_locate) = moved_from {
            self.by_locate.remove(&previous_locate);
            self.stats.relocated += 1;
        }

        let instrument = Instrument::from_message(message);
        let Some(previous) = self.by_locate.insert(locate, instrument) else {
            self.stats.inserted += 1;
            return moved_from.map_or(
                RegistryUpdate::Inserted { locate, symbol },
                |previous_locate| RegistryUpdate::Relocated {
                    symbol,
                    previous_locate,
                },
            );
        };

        if previous.symbol == symbol {
            self.stats.updated += 1;
            return RegistryUpdate::Updated { locate, symbol };
        }

        // The locate now names a different symbol. Evict the old reverse
        // mapping unless the insertion above already repointed it.
        if self.by_symbol.get(&previous.symbol) == Some(&locate) {
            self.by_symbol.remove(&previous.symbol);
        }
        self.stats.replaced += 1;
        RegistryUpdate::Replaced {
            locate,
            previous_symbol: previous.symbol,
            symbol,
        }
    }

    /// Applies `message` if it is a Stock Directory message, ignoring every
    /// other type.
    pub fn apply_message(&mut self, message: &ParsedMessage) -> Option<RegistryUpdate> {
        match message {
            ParsedMessage::StockDirectory(directory) => Some(self.apply(directory)),
            _ => None,
        }
    }

    #[must_use]
    pub fn instrument(&self, locate: u16) -> Option<&Instrument> {
        self.by_locate.get(&locate)
    }

    #[must_use]
    pub fn symbol(&self, locate: u16) -> Option<Symbol> {
        self.instrument(locate).map(|instrument| instrument.symbol)
    }

    /// The padded, on-wire symbol for `locate`.
    #[must_use]
    pub fn symbol_str(&self, locate: u16) -> Option<&str> {
        self.instrument(locate).map(Instrument::symbol_str)
    }

    #[must_use]
    pub fn locate(&self, symbol: Symbol) -> Option<u16> {
        self.by_symbol.get(&symbol).copied()
    }

    /// Every known instrument, in unspecified order.
    pub fn instruments(&self) -> impl Iterator<Item = &Instrument> {
        self.by_locate.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_locate.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_locate.is_empty()
    }

    #[must_use]
    pub const fn stats(&self) -> RegistryStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itch::{
        FinancialStatusIndicator, IssueClassification, LuldReferencePriceTier, MarketCategory,
        Symbol,
    };

    fn symbol(bytes: [u8; 8]) -> Symbol {
        Symbol::from_bytes(bytes).expect("test symbol should be ASCII")
    }

    fn directory(locate: u16, name: [u8; 8]) -> StockDirectoryMessage {
        StockDirectoryMessage {
            stock_locate: locate,
            tracking_number: 0,
            timestamp: 0,
            stock: symbol(name),
            market_category: MarketCategory::GlobalSelect,
            financial_status_indicator: FinancialStatusIndicator::Normal,
            round_lot_size: 100,
            round_lots_only: false,
            issue_classification: IssueClassification::CommonStock,
            issue_subtype: *b"  ",
            authenticity: Authenticity::Production,
            short_sale_threshold_indicator: false,
            ipo_flag: false,
            luld_reference_price_tier: LuldReferencePriceTier::Tier1,
            etp_flag: false,
            etp_leverage_factor: 0,
            inverse_indicator: false,
        }
    }

    #[test]
    fn inserts_and_resolves_both_directions() {
        let mut registry = Registry::new();
        let update = registry.apply(&directory(7, *b"AAPL    "));

        assert_eq!(
            update,
            RegistryUpdate::Inserted {
                locate: 7,
                symbol: symbol(*b"AAPL    ")
            }
        );
        assert_eq!(registry.symbol(7), Some(symbol(*b"AAPL    ")));
        assert_eq!(registry.symbol_str(7), Some("AAPL"));
        assert_eq!(registry.locate(symbol(*b"AAPL    ")), Some(7));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.stats().inserted, 1);
    }

    #[test]
    fn repeated_directory_for_the_same_locate_updates_in_place() {
        let mut registry = Registry::new();
        registry.apply(&directory(7, *b"AAPL    "));

        let mut refreshed = directory(7, *b"AAPL    ");
        refreshed.round_lot_size = 10;
        let update = registry.apply(&refreshed);

        assert_eq!(
            update,
            RegistryUpdate::Updated {
                locate: 7,
                symbol: symbol(*b"AAPL    ")
            }
        );
        assert_eq!(registry.instrument(7).unwrap().round_lot_size, 10);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.stats().updated, 1);
    }

    #[test]
    fn a_reused_locate_evicts_the_previous_symbol() {
        let mut registry = Registry::new();
        registry.apply(&directory(7, *b"OLD     "));

        let update = registry.apply(&directory(7, *b"NEW     "));

        assert_eq!(
            update,
            RegistryUpdate::Replaced {
                locate: 7,
                previous_symbol: symbol(*b"OLD     "),
                symbol: symbol(*b"NEW     ")
            }
        );
        assert_eq!(registry.symbol(7), Some(symbol(*b"NEW     ")));
        assert_eq!(
            registry.locate(symbol(*b"OLD     ")),
            None,
            "the stale reverse mapping must not survive a locate reuse"
        );
        assert_eq!(registry.stats().replaced, 1);
    }

    #[test]
    fn a_symbol_moving_locate_removes_the_old_locate() {
        let mut registry = Registry::new();
        registry.apply(&directory(7, *b"MOVE    "));

        let update = registry.apply(&directory(9, *b"MOVE    "));

        assert_eq!(
            update,
            RegistryUpdate::Relocated {
                symbol: symbol(*b"MOVE    "),
                previous_locate: 7
            }
        );
        assert_eq!(registry.symbol(7), None);
        assert_eq!(registry.symbol(9), Some(symbol(*b"MOVE    ")));
        assert_eq!(registry.locate(symbol(*b"MOVE    ")), Some(9));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn unknown_locates_resolve_to_nothing() {
        let registry = Registry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.symbol(42), None);
        assert_eq!(registry.symbol_str(42), None);
        assert_eq!(registry.locate(symbol(*b"NONE    ")), None);
    }

    #[test]
    fn apply_message_ignores_non_directory_messages() {
        let mut registry = Registry::new();
        let message = ParsedMessage::SystemEvent(crate::itch::SystemEventMessage {
            stock_locate: 1,
            tracking_number: 1,
            timestamp: 0,
            event_code: crate::itch::EventCode::StartOfMessages,
        });
        assert_eq!(registry.apply_message(&message), None);
        assert!(registry.is_empty());
    }

    #[test]
    fn instruments_lists_every_registered_security() {
        let mut registry = Registry::new();
        registry.apply(&directory(7, *b"AAPL    "));
        registry.apply(&directory(8, *b"MSFT    "));

        let mut names: Vec<&str> = registry.instruments().map(Instrument::symbol_str).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["AAPL", "MSFT"]);
    }
}
