//! A deliberately simple, honest demo alpha.
//!
//! The signal is top-of-book **order-flow imbalance**:
//!
//! ```text
//! imbalance = (bid_size - ask_size) / (bid_size + ask_size)
//! ```
//!
//! It is a real microstructure signal, not a random-number generator. Resting
//! size is a proxy for the next trade's direction: when the bid is heavy, the
//! next market order is more likely to lift the offer. The
//! [`TopOfBook::microprice`](crate::book::TopOfBook::microprice) is the same
//! idea expressed in price terms, so the signal reports the edge it thinks it
//! has in basis points.
//!
//! The strategy is intentionally small enough to read in one sitting. It is
//! **not** a profitable trading system and it does not pretend to be:
//!
//! * it never looks at anything below the top of book,
//! * it has no volatility, news, or regime awareness,
//! * it pays the spread whenever `cross_spread` is enabled, which is what
//!   makes it trade at all in a replay.
//!
//! What it *does* have is inventory awareness: it will not exceed
//! `max_position`, it releases a position once the imbalance stops supporting
//! it, and it observes a per-symbol cooldown so a single quote cannot produce
//! a burst of orders.

use std::collections::HashMap;

use crate::book::TopOfBook;
use crate::itch::{BuySell, Price};

/// Tunables for [`ImbalanceAlpha`].
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct AlphaConfig {
    /// Minimum combined displayed size at the touch. Below this, imbalance is
    /// noise: a 1x0 book is not a signal.
    pub min_top_size: u64,
    /// Absolute imbalance required to open a position, in `(0, 1]`.
    pub imbalance_threshold: f64,
    /// Absolute imbalance at or below which an existing position is released.
    /// Zero means "get out as soon as the imbalance is no longer in my
    /// favour".
    pub exit_imbalance: f64,
    /// Shares per child order.
    pub order_shares: u32,
    /// Maximum absolute position, in shares.
    pub max_position: i64,
    /// Minimum time between two signals for the same symbol, in nanoseconds.
    pub cooldown_ns: u64,
    /// Whether to cross the spread. A real passive strategy would not, but a
    /// replay then fills only when the market trades through the resting
    /// price, which makes the demo silent. Crossing is stated plainly here
    /// rather than hidden in the fill model.
    pub cross_spread: bool,
}

impl Default for AlphaConfig {
    fn default() -> Self {
        Self {
            min_top_size: 200,
            imbalance_threshold: 0.6,
            exit_imbalance: 0.0,
            order_shares: 100,
            max_position: 100,
            cooldown_ns: 1_000_000_000,
            cross_spread: true,
        }
    }
}

/// Why the alpha wants to trade.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SignalReason {
    /// The imbalance crossed the entry threshold.
    Imbalance,
    /// The imbalance stopped supporting an existing position, so the position
    /// is released.
    Exit,
}

/// A trade the alpha would like to do.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Signal {
    pub stock_locate: u16,
    pub side: BuySell,
    pub shares: u32,
    pub limit_price: Price,
    /// The observed imbalance, in `[-1, 1]`.
    pub imbalance: f64,
    /// Microprice minus mid, in canonical price units. Positive means the
    /// book says the next trade is likely to be higher.
    pub microprice_edge: Price,
    /// The midpoint the edge was measured against.
    pub reference_price: Price,
    pub reason: SignalReason,
}

impl Signal {
    /// The microprice edge in basis points of the midpoint.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn edge_bps(self) -> f64 {
        let mid = self.reference_price.as_x10_8();
        if mid == 0 {
            return 0.0;
        }
        self.microprice_edge.as_x10_8() as f64 / mid as f64 * 10_000.0
    }
}

/// Counters describing what the alpha considered.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct AlphaStats {
    pub evaluated: u64,
    pub signals: u64,
    pub entries: u64,
    pub exits: u64,
    /// The book had fewer than two sides.
    pub one_sided: u64,
    /// The book was locked or crossed; no sensible trade exists.
    pub crossed: u64,
    /// The touch was thinner than `min_top_size`.
    pub thin: u64,
    /// Imbalance did not clear the threshold and no position needed releasing.
    pub below_threshold: u64,
    /// `max_position` was already reached.
    pub at_position_limit: u64,
    /// A signal was suppressed by the per-symbol cooldown.
    pub cooling_down: u64,
    /// A signal would have had zero size.
    pub zero_size: u64,
}

/// Inventory-aware top-of-book imbalance strategy.
#[derive(Debug, Clone)]
pub struct ImbalanceAlpha {
    config: AlphaConfig,
    last_signal_ns: HashMap<u16, u64>,
    stats: AlphaStats,
}

impl ImbalanceAlpha {
    #[must_use]
    pub fn new(config: AlphaConfig) -> Self {
        Self {
            config,
            last_signal_ns: HashMap::new(),
            stats: AlphaStats::default(),
        }
    }

    #[must_use]
    pub const fn config(&self) -> &AlphaConfig {
        &self.config
    }

    #[must_use]
    pub const fn stats(&self) -> AlphaStats {
        self.stats
    }

    /// Evaluates the latest quote and returns a signal if one is warranted.
    ///
    /// `timestamp` is ITCH's nanoseconds-since-midnight clock and `position`
    /// is the current signed position in shares, so the alpha can size entries
    /// and release inventory without owning either.
    pub fn on_quote(
        &mut self,
        stock_locate: u16,
        top: &TopOfBook,
        timestamp: u64,
        position: i64,
    ) -> Option<Signal> {
        self.stats.evaluated += 1;

        let (Some(bid), Some(ask)) = (top.bid, top.ask) else {
            self.stats.one_sided += 1;
            return None;
        };
        if bid.price.as_x10_8() >= ask.price.as_x10_8() {
            self.stats.crossed += 1;
            return None;
        }
        if bid.shares + ask.shares < self.config.min_top_size {
            self.stats.thin += 1;
            return None;
        }
        let imbalance = top.imbalance()?;
        let reference_price = bid.price.midpoint(ask.price);
        let microprice_edge = top.microprice().map_or_else(Price::default, |microprice| {
            Price::from_x10_8_canonical(
                microprice
                    .as_x10_8()
                    .saturating_sub(reference_price.as_x10_8()),
            )
        });

        let (side, shares, reason) = self.choose(imbalance, position)?;

        if self
            .last_signal_ns
            .get(&stock_locate)
            .is_some_and(|&last| timestamp.saturating_sub(last) < self.config.cooldown_ns)
        {
            self.stats.cooling_down += 1;
            return None;
        }
        let limit_price = match side {
            BuySell::Buy => {
                if self.config.cross_spread {
                    ask.price
                } else {
                    bid.price
                }
            }
            BuySell::Sell => {
                if self.config.cross_spread {
                    bid.price
                } else {
                    ask.price
                }
            }
            BuySell::Other(_) => return None,
        };

        self.last_signal_ns.insert(stock_locate, timestamp);
        self.stats.signals += 1;
        match reason {
            SignalReason::Imbalance => self.stats.entries += 1,
            SignalReason::Exit => self.stats.exits += 1,
        }

        Some(Signal {
            stock_locate,
            side,
            shares,
            limit_price,
            imbalance,
            microprice_edge,
            reference_price,
            reason,
        })
    }

    /// Picks a side and size, or records why no trade is possible.
    fn choose(&mut self, imbalance: f64, position: i64) -> Option<(BuySell, u32, SignalReason)> {
        let config = self.config;

        let release_long = position > 0 && imbalance <= config.exit_imbalance;
        let release_short = position < 0 && imbalance >= -config.exit_imbalance;
        if release_long {
            return self
                .sized(BuySell::Sell, position.unsigned_abs())
                .map(|(side, shares)| (side, shares, SignalReason::Exit));
        }
        if release_short {
            return self
                .sized(BuySell::Buy, position.unsigned_abs())
                .map(|(side, shares)| (side, shares, SignalReason::Exit));
        }

        if imbalance >= config.imbalance_threshold {
            if position >= config.max_position {
                self.stats.at_position_limit += 1;
                return None;
            }
            let room = config.max_position.saturating_sub(position).unsigned_abs();
            return self
                .sized(BuySell::Buy, room)
                .map(|(side, shares)| (side, shares, SignalReason::Imbalance));
        }

        if imbalance <= -config.imbalance_threshold {
            if position <= -config.max_position {
                self.stats.at_position_limit += 1;
                return None;
            }
            let room = config.max_position.saturating_add(position).unsigned_abs();
            return self
                .sized(BuySell::Sell, room)
                .map(|(side, shares)| (side, shares, SignalReason::Imbalance));
        }

        self.stats.below_threshold += 1;
        None
    }

    fn sized(&mut self, side: BuySell, room: u64) -> Option<(BuySell, u32)> {
        let shares = room.min(u64::from(self.config.order_shares));
        let shares = u32::try_from(shares).unwrap_or(u32::MAX);
        if shares == 0 {
            self.stats.zero_size += 1;
            return None;
        }
        Some((side, shares))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::Level;

    const SECOND: u64 = 1_000_000_000;

    fn level(price_cents: i64, shares: u64) -> Level {
        Level {
            price: Price::from_x10_8_canonical(price_cents * 1_000_000),
            shares,
            orders: 1,
        }
    }

    fn top(bid_cents: i64, bid_shares: u64, ask_cents: i64, ask_shares: u64) -> TopOfBook {
        TopOfBook {
            bid: Some(level(bid_cents, bid_shares)),
            ask: Some(level(ask_cents, ask_shares)),
        }
    }

    fn alpha() -> ImbalanceAlpha {
        ImbalanceAlpha::new(AlphaConfig::default())
    }

    #[test]
    fn a_heavy_bid_opens_a_long_at_the_offer() {
        let mut alpha = alpha();
        // imbalance = (900 - 100) / 1000 = 0.8
        let signal = alpha
            .on_quote(1, &top(10_000, 900, 10_005, 100), 0, 0)
            .expect("heavy bid should signal");

        assert_eq!(signal.side, BuySell::Buy);
        assert_eq!(signal.shares, 100);
        assert_eq!(signal.limit_price, level(10_005, 1).price);
        assert_eq!(signal.reason, SignalReason::Imbalance);
        assert!((signal.imbalance - 0.8).abs() < 1e-12);
        assert!(signal.microprice_edge.as_x10_8() > 0);
        assert!(signal.edge_bps() > 0.0);
        assert_eq!(alpha.stats().entries, 1);
    }

    #[test]
    fn a_heavy_ask_opens_a_short_at_the_bid() {
        let mut alpha = alpha();
        // imbalance = (100 - 900) / 1000 = -0.8
        let signal = alpha
            .on_quote(1, &top(10_000, 100, 10_005, 900), 0, 0)
            .expect("heavy ask should signal");

        assert_eq!(signal.side, BuySell::Sell);
        assert_eq!(signal.limit_price, level(10_000, 1).price);
        assert!(signal.microprice_edge.as_x10_8() < 0);
        assert!(signal.edge_bps() < 0.0);
    }

    #[test]
    fn a_balanced_book_does_nothing() {
        let mut alpha = alpha();
        assert!(
            alpha
                .on_quote(1, &top(10_000, 500, 10_005, 500), 0, 0)
                .is_none()
        );
        assert_eq!(alpha.stats().below_threshold, 1);
        assert_eq!(alpha.stats().signals, 0);
    }

    #[test]
    fn thin_and_one_sided_books_are_ignored() {
        let mut alpha = alpha();
        // Combined size 100 is below the 200-share minimum.
        assert!(
            alpha
                .on_quote(1, &top(10_000, 90, 10_005, 10), 0, 0)
                .is_none()
        );
        assert_eq!(alpha.stats().thin, 1);

        let one_sided = TopOfBook {
            bid: Some(level(10_000, 1_000)),
            ask: None,
        };
        assert!(alpha.on_quote(1, &one_sided, 0, 0).is_none());
        assert_eq!(alpha.stats().one_sided, 1);
    }

    #[test]
    fn a_crossed_book_is_never_traded() {
        let mut alpha = alpha();
        assert!(
            alpha
                .on_quote(1, &top(10_010, 900, 10_005, 100), 0, 0)
                .is_none()
        );
        assert_eq!(alpha.stats().crossed, 1);
    }

    #[test]
    fn a_position_is_released_once_the_imbalance_stops_supporting_it() {
        let mut alpha = alpha();
        // Long 100, and the book is now mildly ask-heavy: get flat at the bid.
        let signal = alpha
            .on_quote(1, &top(10_000, 450, 10_005, 550), SECOND, 100)
            .expect("a stale long should be released");

        assert_eq!(signal.side, BuySell::Sell);
        assert_eq!(signal.shares, 100);
        assert_eq!(signal.reason, SignalReason::Exit);
        assert_eq!(signal.limit_price, level(10_000, 1).price);
    }

    #[test]
    fn a_short_is_covered_when_the_book_turns_bid_heavy() {
        let mut alpha = alpha();
        let signal = alpha
            .on_quote(1, &top(10_000, 550, 10_005, 450), SECOND, -100)
            .expect("a stale short should be covered");

        assert_eq!(signal.side, BuySell::Buy);
        assert_eq!(signal.shares, 100);
        assert_eq!(signal.reason, SignalReason::Exit);
    }

    #[test]
    fn the_position_limit_caps_size_and_blocks_further_entries() {
        let mut alpha = ImbalanceAlpha::new(AlphaConfig {
            max_position: 150,
            order_shares: 100,
            ..AlphaConfig::default()
        });

        // Long 100 of a 150 limit leaves room for only 50.
        let signal = alpha
            .on_quote(1, &top(10_000, 900, 10_005, 100), SECOND, 100)
            .expect("there is still room");
        assert_eq!(signal.shares, 50);

        // At the limit, a still-heavy bid produces nothing.
        assert!(
            alpha
                .on_quote(1, &top(10_000, 900, 10_005, 100), 2 * SECOND, 150)
                .is_none()
        );
        assert_eq!(alpha.stats().at_position_limit, 1);
    }

    #[test]
    fn the_cooldown_suppresses_a_second_signal_for_the_same_symbol() {
        let mut alpha = alpha();
        assert!(
            alpha
                .on_quote(1, &top(10_000, 900, 10_005, 100), 0, 0)
                .is_some()
        );
        assert!(
            alpha
                .on_quote(1, &top(10_000, 900, 10_005, 100), SECOND / 2, 0)
                .is_none()
        );
        assert_eq!(alpha.stats().cooling_down, 1);

        // Once the cooldown has elapsed a fresh signal is allowed.
        assert!(
            alpha
                .on_quote(1, &top(10_000, 900, 10_005, 100), SECOND, 0)
                .is_some()
        );
        assert_eq!(alpha.stats().signals, 2);
    }

    #[test]
    fn cooldown_is_per_symbol() {
        let mut alpha = alpha();
        assert!(
            alpha
                .on_quote(1, &top(10_000, 900, 10_005, 100), 0, 0)
                .is_some()
        );
        assert!(
            alpha
                .on_quote(2, &top(10_000, 900, 10_005, 100), 0, 0)
                .is_some()
        );
        assert_eq!(alpha.stats().signals, 2);
    }

    #[test]
    fn passive_mode_quotes_on_its_own_side_of_the_spread() {
        let mut alpha = ImbalanceAlpha::new(AlphaConfig {
            cross_spread: false,
            ..AlphaConfig::default()
        });
        let signal = alpha
            .on_quote(1, &top(10_000, 900, 10_005, 100), 0, 0)
            .unwrap();
        assert_eq!(signal.limit_price, level(10_000, 1).price);
    }
}
