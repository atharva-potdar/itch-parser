//! A limit order book rebuilt from ITCH order events.
//!
//! ITCH is market-by-order: every resting order arrives as an `A` (or `F`)
//! message keyed by an order reference number, and is then reduced by `E`/`C`
//! (executions), `X` (partial cancels), `D` (deletes), or replaced by `U`.
//! Maintaining a book is therefore a matter of applying those five operations
//! consistently in both directions: an order map for the per-order view and
//! price-level aggregates for the top-of-book view.
//!
//! # What this book is, and is not
//!
//! It is an exact reconstruction of the *displayed* Nasdaq book as ITCH
//! describes it. It is not a matching engine, and it deliberately ignores
//! `P` (trade), `Q` (cross trade), and `B` (broken trade) messages: those
//! report executions against non-displayed interest and do not change the
//! displayed book.
//!
//! # Integrity
//!
//! Every operation validates before mutating. An execution against an order
//! that does not exist, or for more shares than it holds, returns a
//! [`BookError`] and leaves the book untouched. That matters because the
//! caller is expected to run the feed under
//! [`GapPolicy::Halt`](crate::source::GapPolicy::Halt); if a book operation
//! fails, the honest response is to stop trading, not to guess.

use std::collections::{BTreeMap, HashMap};

use crate::itch::{BuySell, ParsedMessage, Price};

/// A single resting order.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct RestingOrder {
    pub order_reference_number: u64,
    pub side: BuySell,
    pub shares: u32,
    pub price: Price,
}

/// Aggregated resting interest at one price.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Level {
    pub price: Price,
    pub shares: u64,
    pub orders: u32,
}

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
struct Aggregate {
    shares: u64,
    orders: u32,
}

/// The two-sided book for one `stock_locate`.
#[derive(Debug, Clone)]
pub struct Book {
    locate: u16,
    orders: HashMap<u64, RestingOrder>,
    bids: BTreeMap<Price, Aggregate>,
    asks: BTreeMap<Price, Aggregate>,
}

impl Book {
    #[must_use]
    pub fn new(locate: u16) -> Self {
        Self {
            locate,
            orders: HashMap::new(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn locate(&self) -> u16 {
        self.locate
    }

    #[must_use]
    pub fn order(&self, order_reference_number: u64) -> Option<&RestingOrder> {
        self.orders.get(&order_reference_number)
    }

    #[must_use]
    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    #[must_use]
    pub fn level_count(&self) -> usize {
        self.bids.len() + self.asks.len()
    }

    #[must_use]
    pub fn best_bid(&self) -> Option<Level> {
        self.bids
            .last_key_value()
            .map(|(&price, aggregate)| level(price, *aggregate))
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<Level> {
        self.asks
            .first_key_value()
            .map(|(&price, aggregate)| level(price, *aggregate))
    }

    /// The midpoint of the best bid and ask, or `None` when either side is
    /// empty. Rounds towards zero on an odd spread.
    #[must_use]
    pub fn mid(&self) -> Option<Price> {
        Some(self.best_bid()?.price.midpoint(self.best_ask()?.price))
    }

    /// Best ask minus best bid, or `None` when either side is empty.
    #[must_use]
    pub fn spread(&self) -> Option<Price> {
        let bid = self.best_bid()?.price;
        let ask = self.best_ask()?.price;
        Some(Price::from_x10_8_canonical(
            ask.as_x10_8().saturating_sub(bid.as_x10_8()),
        ))
    }

    /// Up to `depth` levels on `side`, best price first.
    ///
    /// A side that is not [`BuySell::Buy`] or [`BuySell::Sell`] has no book
    /// representation, so the result is empty.
    #[must_use]
    pub fn depth(&self, side: BuySell, depth: usize) -> Vec<Level> {
        match side {
            BuySell::Buy => self
                .bids
                .iter()
                .rev()
                .take(depth)
                .map(|(&price, aggregate)| level(price, *aggregate))
                .collect(),
            BuySell::Sell => self
                .asks
                .iter()
                .take(depth)
                .map(|(&price, aggregate)| level(price, *aggregate))
                .collect(),
            BuySell::Other(_) => Vec::new(),
        }
    }

    /// Whether the order map and the level aggregates agree.
    ///
    /// Exposed because it is the cheapest way to assert the book's central
    /// invariant in tests and in a long-running demo.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        let expected = |side: BuySell| {
            let mut levels: BTreeMap<Price, Aggregate> = BTreeMap::new();
            for order in self.orders.values().filter(|order| order.side == side) {
                let aggregate = levels.entry(order.price).or_default();
                aggregate.shares += u64::from(order.shares);
                aggregate.orders += 1;
            }
            levels
        };

        expected(BuySell::Buy) == self.bids && expected(BuySell::Sell) == self.asks
    }

    /// Adds a resting order.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::ZeroShares`] for a zero-size order,
    /// [`BookError::UnsupportedSide`] for a side byte ITCH does not define,
    /// and [`BookError::DuplicateOrder`] if the reference number is live.
    pub fn add(
        &mut self,
        order_reference_number: u64,
        side: BuySell,
        shares: u32,
        price: Price,
    ) -> Result<RestingOrder, BookError> {
        if shares == 0 {
            return Err(BookError::ZeroShares {
                order_reference_number,
            });
        }
        if !matches!(side, BuySell::Buy | BuySell::Sell) {
            return Err(BookError::UnsupportedSide(side.as_byte()));
        }
        if self.orders.contains_key(&order_reference_number) {
            return Err(BookError::DuplicateOrder {
                order_reference_number,
            });
        }

        let order = RestingOrder {
            order_reference_number,
            side,
            shares,
            price,
        };
        self.orders.insert(order_reference_number, order);
        let aggregate = self.levels_mut(side).entry(price).or_default();
        aggregate.shares += u64::from(shares);
        aggregate.orders += 1;
        Ok(order)
    }

    /// Applies an execution to a resting order.
    ///
    /// The level price comes from the resting order, not from the message: an
    /// `E` message carries no price at all, and a `C` message's execution
    /// price is a property of the trade rather than of the resting order.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownOrder`] if the order is not live, and
    /// [`BookError::OverReduction`] if the execution exceeds its size.
    pub fn execute(&mut self, order_reference_number: u64, shares: u32) -> Result<Fill, BookError> {
        let order = *self
            .orders
            .get(&order_reference_number)
            .ok_or(BookError::UnknownOrder {
                order_reference_number,
            })?;
        if shares > order.shares {
            return Err(BookError::OverReduction {
                order_reference_number,
                resting_shares: order.shares,
                attempted_shares: shares,
            });
        }

        let remaining = order.shares - shares;
        if remaining == 0 {
            self.orders.remove(&order_reference_number);
            self.reduce_level(order.side, order.price, u64::from(shares), 1);
        } else {
            if let Some(resting) = self.orders.get_mut(&order_reference_number) {
                resting.shares = remaining;
            }
            self.reduce_level(order.side, order.price, u64::from(shares), 0);
        }

        Ok(Fill {
            price: order.price,
            shares,
            remaining_shares: remaining,
        })
    }

    /// Partially or fully cancels a resting order.
    ///
    /// A cancel that removes the whole order is treated as a delete; ITCH
    /// sends `X` for partial cancels and `D` for full ones, but real feeds do
    /// occasionally send `X` for the remaining size.
    ///
    /// # Errors
    ///
    /// Same conditions as [`Book::execute`].
    pub fn cancel(
        &mut self,
        order_reference_number: u64,
        shares: u32,
    ) -> Result<Cancel, BookError> {
        let order = *self
            .orders
            .get(&order_reference_number)
            .ok_or(BookError::UnknownOrder {
                order_reference_number,
            })?;
        if shares > order.shares {
            return Err(BookError::OverReduction {
                order_reference_number,
                resting_shares: order.shares,
                attempted_shares: shares,
            });
        }

        let remaining = order.shares - shares;
        if remaining == 0 {
            self.orders.remove(&order_reference_number);
            self.reduce_level(order.side, order.price, u64::from(shares), 1);
        } else {
            if let Some(resting) = self.orders.get_mut(&order_reference_number) {
                resting.shares = remaining;
            }
            self.reduce_level(order.side, order.price, u64::from(shares), 0);
        }

        Ok(Cancel {
            cancelled_shares: shares,
            remaining_shares: remaining,
        })
    }

    /// Removes a resting order and returns its remaining size.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownOrder`] if the order is not live.
    pub fn delete(&mut self, order_reference_number: u64) -> Result<u32, BookError> {
        let order = self
            .orders
            .remove(&order_reference_number)
            .ok_or(BookError::UnknownOrder {
                order_reference_number,
            })?;
        self.reduce_level(order.side, order.price, u64::from(order.shares), 1);
        Ok(order.shares)
    }

    /// Replaces an order, preserving its side.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::UnknownOrder`] if the original is not live,
    /// [`BookError::DuplicateOrder`] if `new_order_reference_number` already
    /// exists, and [`BookError::ZeroShares`] for a zero-size replacement.
    pub fn replace(
        &mut self,
        original_order_reference_number: u64,
        new_order_reference_number: u64,
        shares: u32,
        price: Price,
    ) -> Result<RestingOrder, BookError> {
        if shares == 0 {
            return Err(BookError::ZeroShares {
                order_reference_number: new_order_reference_number,
            });
        }
        if self.orders.contains_key(&new_order_reference_number) {
            return Err(BookError::DuplicateOrder {
                order_reference_number: new_order_reference_number,
            });
        }
        let original =
            *self
                .orders
                .get(&original_order_reference_number)
                .ok_or(BookError::UnknownOrder {
                    order_reference_number: original_order_reference_number,
                })?;

        self.orders.remove(&original_order_reference_number);
        self.reduce_level(original.side, original.price, u64::from(original.shares), 1);

        let replacement = RestingOrder {
            order_reference_number: new_order_reference_number,
            side: original.side,
            shares,
            price,
        };
        self.orders.insert(new_order_reference_number, replacement);
        let aggregate = self.levels_mut(replacement.side).entry(price).or_default();
        aggregate.shares += u64::from(shares);
        aggregate.orders += 1;
        Ok(replacement)
    }

    const fn levels_mut(&mut self, side: BuySell) -> &mut BTreeMap<Price, Aggregate> {
        match side {
            BuySell::Buy => &mut self.bids,
            _ => &mut self.asks,
        }
    }

    /// Removes `shares` (and optionally whole orders) from a level, dropping
    /// the level once it is empty.
    fn reduce_level(&mut self, side: BuySell, price: Price, shares: u64, orders: u32) {
        let map = self.levels_mut(side);
        let Some(aggregate) = map.get_mut(&price) else {
            return;
        };
        aggregate.shares = aggregate.shares.saturating_sub(shares);
        aggregate.orders = aggregate.orders.saturating_sub(orders);
        let empty = aggregate.orders == 0;
        if empty {
            map.remove(&price);
        }
    }
}

const fn level(price: Price, aggregate: Aggregate) -> Level {
    Level {
        price,
        shares: aggregate.shares,
        orders: aggregate.orders,
    }
}

/// A book operation that failed validation.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum BookError {
    /// A reduction referenced an order that is not resting.
    UnknownOrder { order_reference_number: u64 },
    /// An add or replace reused a live order reference number.
    DuplicateOrder { order_reference_number: u64 },
    /// An order was announced with zero shares.
    ZeroShares { order_reference_number: u64 },
    /// The side byte is not `B` or `S`.
    UnsupportedSide(u8),
    /// An execution or cancel exceeded the order's remaining size.
    OverReduction {
        order_reference_number: u64,
        resting_shares: u32,
        attempted_shares: u32,
    },
}

/// The result of applying an execution.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Fill {
    pub price: Price,
    pub shares: u32,
    pub remaining_shares: u32,
}

/// The result of applying a cancel.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Cancel {
    pub cancelled_shares: u32,
    pub remaining_shares: u32,
}

/// What a message did to the book.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum BookChange {
    Added {
        locate: u16,
        order_reference_number: u64,
    },
    Executed {
        locate: u16,
        order_reference_number: u64,
        shares: u32,
        price: Price,
        remaining_shares: u32,
    },
    Cancelled {
        locate: u16,
        order_reference_number: u64,
        shares: u32,
        remaining_shares: u32,
    },
    Deleted {
        locate: u16,
        order_reference_number: u64,
    },
    Replaced {
        locate: u16,
        original_order_reference_number: u64,
        new_order_reference_number: u64,
    },
    /// An administrative, trade, or non-displayed message that leaves the
    /// displayed book unchanged.
    Ignored,
}

/// Counters describing book activity.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct BookStats {
    pub added: u64,
    pub executed: u64,
    pub cancelled: u64,
    pub deleted: u64,
    pub replaced: u64,
    /// Operations rejected by validation, and therefore not applied.
    pub rejected: u64,
}

/// A collection of books, one per `stock_locate`.
#[derive(Debug, Default, Clone)]
pub struct BookSet {
    books: HashMap<u16, Book>,
    stats: BookStats,
}

impl BookSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one decoded ITCH message.
    ///
    /// # Errors
    ///
    /// Returns the [`BookError`] from the underlying book if the message is
    /// inconsistent with the current state. The book is left untouched in
    /// that case, and [`BookStats::rejected`] is incremented.
    pub fn apply(&mut self, message: &ParsedMessage) -> Result<BookChange, BookError> {
        let result = self.apply_inner(message);
        if result.is_err() {
            self.stats.rejected += 1;
        }
        result
    }

    fn apply_inner(&mut self, message: &ParsedMessage) -> Result<BookChange, BookError> {
        match message {
            ParsedMessage::AddOrder(add) => self.add(
                add.stock_locate,
                add.order_reference_number,
                add.buy_sell_indicator,
                add.shares,
                add.price,
            ),
            ParsedMessage::AddOrderMPIDAttribution(add) => self.add(
                add.stock_locate,
                add.order_reference_number,
                add.buy_sell_indicator,
                add.shares,
                add.price,
            ),
            ParsedMessage::OrderExecuted(execution) => {
                let book = self.book_mut(execution.stock_locate);
                let fill =
                    book.execute(execution.order_reference_number, execution.executed_shares)?;
                self.stats.executed += 1;
                Ok(BookChange::Executed {
                    locate: execution.stock_locate,
                    order_reference_number: execution.order_reference_number,
                    shares: fill.shares,
                    price: fill.price,
                    remaining_shares: fill.remaining_shares,
                })
            }
            ParsedMessage::OrderExecutedPrice(execution) => {
                let book = self.book_mut(execution.stock_locate);
                let fill =
                    book.execute(execution.order_reference_number, execution.executed_shares)?;
                self.stats.executed += 1;
                Ok(BookChange::Executed {
                    locate: execution.stock_locate,
                    order_reference_number: execution.order_reference_number,
                    shares: fill.shares,
                    price: fill.price,
                    remaining_shares: fill.remaining_shares,
                })
            }
            ParsedMessage::OrderCancel(cancel) => {
                let book = self.book_mut(cancel.stock_locate);
                let outcome =
                    book.cancel(cancel.order_reference_number, cancel.cancelled_shares)?;
                self.stats.cancelled += 1;
                Ok(BookChange::Cancelled {
                    locate: cancel.stock_locate,
                    order_reference_number: cancel.order_reference_number,
                    shares: outcome.cancelled_shares,
                    remaining_shares: outcome.remaining_shares,
                })
            }
            ParsedMessage::OrderDelete(delete) => {
                let book = self.book_mut(delete.stock_locate);
                book.delete(delete.order_reference_number)?;
                self.stats.deleted += 1;
                Ok(BookChange::Deleted {
                    locate: delete.stock_locate,
                    order_reference_number: delete.order_reference_number,
                })
            }
            ParsedMessage::OrderReplace(replace) => {
                let book = self.book_mut(replace.stock_locate);
                book.replace(
                    replace.original_order_reference_number,
                    replace.new_order_reference_number,
                    replace.shares,
                    replace.price,
                )?;
                self.stats.replaced += 1;
                Ok(BookChange::Replaced {
                    locate: replace.stock_locate,
                    original_order_reference_number: replace.original_order_reference_number,
                    new_order_reference_number: replace.new_order_reference_number,
                })
            }
            _ => Ok(BookChange::Ignored),
        }
    }

    fn add(
        &mut self,
        locate: u16,
        order_reference_number: u64,
        side: BuySell,
        shares: u32,
        price: Price,
    ) -> Result<BookChange, BookError> {
        self.book_mut(locate)
            .add(order_reference_number, side, shares, price)?;
        self.stats.added += 1;
        Ok(BookChange::Added {
            locate,
            order_reference_number,
        })
    }

    #[must_use]
    pub fn book(&self, locate: u16) -> Option<&Book> {
        self.books.get(&locate)
    }

    pub fn book_mut(&mut self, locate: u16) -> &mut Book {
        self.books
            .entry(locate)
            .or_insert_with(|| Book::new(locate))
    }

    /// The current two-sided quote for `locate`, if both sides are populated.
    #[must_use]
    pub fn top_of_book(&self, locate: u16) -> Option<TopOfBook> {
        let book = self.book(locate)?;
        Some(TopOfBook {
            bid: book.best_bid(),
            ask: book.best_ask(),
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.books.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
    }

    #[must_use]
    pub const fn stats(&self) -> BookStats {
        self.stats
    }

    /// Whether every book satisfies its internal order/level invariant.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.books.values().all(Book::is_consistent)
    }
}

/// The best bid and ask of a book.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct TopOfBook {
    pub bid: Option<Level>,
    pub ask: Option<Level>,
}

impl TopOfBook {
    #[must_use]
    pub const fn is_two_sided(&self) -> bool {
        self.bid.is_some() && self.ask.is_some()
    }

    #[must_use]
    pub fn mid(&self) -> Option<Price> {
        Some(self.bid?.price.midpoint(self.ask?.price))
    }

    /// The size-weighted mid, often called the microprice.
    ///
    /// When the bid is large relative to the ask the microprice sits above the
    /// mid, because the next trade is more likely to lift the offer. Returns
    /// `None` for a one-sided or empty book.
    #[must_use]
    pub fn microprice(&self) -> Option<Price> {
        let bid = self.bid?;
        let ask = self.ask?;
        let total = i128::from(bid.shares) + i128::from(ask.shares);
        if total == 0 {
            return None;
        }
        let weighted = i128::from(bid.price.as_x10_8()) * i128::from(ask.shares)
            + i128::from(ask.price.as_x10_8()) * i128::from(bid.shares);
        i64::try_from(weighted / total)
            .ok()
            .map(Price::from_x10_8_canonical)
    }

    /// Order-flow imbalance in `[-1, 1]`.
    ///
    /// `+1` means all displayed size is on the bid, `-1` all on the ask.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn imbalance(&self) -> Option<f64> {
        let bid = self.bid?;
        let ask = self.ask?;
        let total = bid.shares + ask.shares;
        if total == 0 {
            return None;
        }
        Some((bid.shares as f64 - ask.shares as f64) / total as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itch::Price;

    fn p(value: u32) -> Price {
        Price::from_x10_4(value)
    }

    /// One cent, in canonical units.
    const TICK: i64 = 1_000_000;

    fn ticked(cents: i64) -> Price {
        Price::from_x10_8_canonical(cents * TICK)
    }

    fn book_with_two_sided_market() -> Book {
        let mut book = Book::new(1);
        book.add(1, BuySell::Buy, 100, p(1_000_000)).unwrap();
        book.add(2, BuySell::Buy, 200, p(999_900)).unwrap();
        book.add(3, BuySell::Sell, 150, p(1_000_100)).unwrap();
        book.add(4, BuySell::Sell, 250, p(1_000_200)).unwrap();
        book
    }

    #[test]
    fn best_bid_and_ask_pick_the_inside_market() {
        let book = book_with_two_sided_market();

        assert_eq!(
            book.best_bid(),
            Some(Level {
                price: p(1_000_000),
                shares: 100,
                orders: 1
            })
        );
        assert_eq!(
            book.best_ask(),
            Some(Level {
                price: p(1_000_100),
                shares: 150,
                orders: 1
            })
        );
        assert_eq!(book.spread(), Some(ticked(1)));
        assert_eq!(book.mid(), Some(p(1_000_050)));
        assert!(book.is_consistent());
    }

    #[test]
    fn depth_is_best_price_first_and_side_aware() {
        let book = book_with_two_sided_market();

        assert_eq!(
            book.depth(BuySell::Buy, 10)
                .iter()
                .map(|level| level.price)
                .collect::<Vec<_>>(),
            vec![p(1_000_000), p(999_900)]
        );
        assert_eq!(
            book.depth(BuySell::Sell, 10)
                .iter()
                .map(|level| level.price)
                .collect::<Vec<_>>(),
            vec![p(1_000_100), p(1_000_200)]
        );
        assert_eq!(book.depth(BuySell::Buy, 1).len(), 1);
        assert!(book.depth(BuySell::Other(b'?'), 10).is_empty());
    }

    #[test]
    fn orders_at_the_same_price_aggregate_into_one_level() {
        let mut book = Book::new(1);
        book.add(1, BuySell::Buy, 100, p(1_000_000)).unwrap();
        book.add(2, BuySell::Buy, 250, p(1_000_000)).unwrap();

        assert_eq!(
            book.best_bid(),
            Some(Level {
                price: p(1_000_000),
                shares: 350,
                orders: 2
            })
        );
        assert_eq!(book.level_count(), 1);
        assert!(book.is_consistent());
    }

    #[test]
    fn a_partial_execution_reduces_size_but_keeps_the_order() {
        let mut book = book_with_two_sided_market();

        let fill = book.execute(1, 40).unwrap();

        assert_eq!(
            fill,
            Fill {
                price: p(1_000_000),
                shares: 40,
                remaining_shares: 60
            }
        );
        assert_eq!(book.best_bid().unwrap().shares, 60);
        assert_eq!(book.best_bid().unwrap().orders, 1);
        assert_eq!(book.order(1).unwrap().shares, 60);
        assert!(book.is_consistent());
    }

    #[test]
    fn a_full_execution_removes_the_order_and_its_level() {
        let mut book = book_with_two_sided_market();

        let fill = book.execute(1, 100).unwrap();

        assert_eq!(fill.remaining_shares, 0);
        assert!(book.order(1).is_none());
        // The 999_900 bid is now the inside market.
        assert_eq!(book.best_bid().unwrap().price, p(999_900));
        assert!(book.is_consistent());
    }

    #[test]
    fn cancel_and_delete_both_shrink_the_book() {
        let mut book = book_with_two_sided_market();

        assert_eq!(
            book.cancel(1, 30).unwrap(),
            Cancel {
                cancelled_shares: 30,
                remaining_shares: 70
            }
        );
        assert_eq!(book.best_bid().unwrap().shares, 70);

        assert_eq!(book.delete(2).unwrap(), 200);
        assert!(book.order(2).is_none());
        assert!(book.is_consistent());
    }

    #[test]
    fn a_cancel_for_the_whole_remainder_is_treated_as_a_delete() {
        let mut book = book_with_two_sided_market();
        book.cancel(1, 100).unwrap();
        assert!(book.order(1).is_none());
        assert_eq!(book.best_bid().unwrap().price, p(999_900));
    }

    #[test]
    fn replace_moves_size_to_the_new_price_and_preserves_the_side() {
        let mut book = book_with_two_sided_market();

        let replacement = book.replace(2, 99, 500, p(1_000_050)).unwrap();

        assert_eq!(replacement.side, BuySell::Buy);
        assert!(book.order(2).is_none());
        assert_eq!(
            book.best_bid(),
            Some(Level {
                price: p(1_000_050),
                shares: 500,
                orders: 1
            })
        );
        assert_eq!(book.best_bid().unwrap().price, p(1_000_050));
        assert!(book.is_consistent());
    }

    #[test]
    fn replacing_onto_an_occupied_price_merges_into_the_level() {
        let mut book = book_with_two_sided_market();

        book.replace(2, 99, 500, p(1_000_000)).unwrap();

        assert_eq!(
            book.best_bid(),
            Some(Level {
                price: p(1_000_000),
                shares: 600,
                orders: 2
            })
        );
        assert!(book.is_consistent());
    }

    #[test]
    fn unknown_orders_are_rejected_without_touching_state() {
        let mut book = book_with_two_sided_market();
        let before = book.clone();

        assert_eq!(
            book.execute(404, 1),
            Err(BookError::UnknownOrder {
                order_reference_number: 404
            })
        );
        assert_eq!(
            book.cancel(404, 1),
            Err(BookError::UnknownOrder {
                order_reference_number: 404
            })
        );
        assert_eq!(
            book.delete(404),
            Err(BookError::UnknownOrder {
                order_reference_number: 404
            })
        );
        assert_eq!(
            book.replace(404, 405, 1, p(1)),
            Err(BookError::UnknownOrder {
                order_reference_number: 404
            })
        );
        assert_eq!(book.best_bid(), before.best_bid());
        assert_eq!(book.order_count(), before.order_count());
    }

    #[test]
    fn over_execution_is_rejected_rather_than_clamping() {
        let mut book = book_with_two_sided_market();

        assert_eq!(
            book.execute(1, 101),
            Err(BookError::OverReduction {
                order_reference_number: 1,
                resting_shares: 100,
                attempted_shares: 101
            })
        );
        assert_eq!(
            book.best_bid().unwrap().shares,
            100,
            "state must be untouched"
        );
        assert!(book.is_consistent());
    }

    #[test]
    fn duplicate_and_zero_share_orders_are_rejected() {
        let mut book = Book::new(1);
        book.add(1, BuySell::Buy, 100, p(1_000_000)).unwrap();

        assert_eq!(
            book.add(1, BuySell::Buy, 100, p(1_000_000)),
            Err(BookError::DuplicateOrder {
                order_reference_number: 1
            })
        );
        assert_eq!(
            book.add(2, BuySell::Buy, 0, p(1_000_000)),
            Err(BookError::ZeroShares {
                order_reference_number: 2
            })
        );
        assert_eq!(
            book.add(2, BuySell::Other(b'?'), 100, p(1_000_000)),
            Err(BookError::UnsupportedSide(b'?'))
        );
        assert_eq!(book.order_count(), 1);
    }

    #[test]
    fn one_sided_book_has_no_mid_or_spread() {
        let mut book = Book::new(1);
        book.add(1, BuySell::Buy, 100, p(1_000_000)).unwrap();

        assert_eq!(book.mid(), None);
        assert_eq!(book.spread(), None);
        assert!(book.best_ask().is_none());
        assert!(book.is_consistent());
    }

    #[test]
    fn microprice_moves_towards_the_thin_side() {
        let heavy_bid = TopOfBook {
            bid: Some(Level {
                price: p(1_000_000),
                shares: 900,
                orders: 1,
            }),
            ask: Some(Level {
                price: p(1_000_100),
                shares: 100,
                orders: 1,
            }),
        };
        let mid = heavy_bid.mid().unwrap();
        let micro = heavy_bid.microprice().unwrap();
        assert!(micro > mid, "heavy bid should pull the microprice up");
        assert!((heavy_bid.imbalance().unwrap() - 0.8).abs() < 1e-12);

        let balanced = TopOfBook {
            bid: Some(Level {
                price: p(1_000_000),
                shares: 500,
                orders: 1,
            }),
            ask: Some(Level {
                price: p(1_000_100),
                shares: 500,
                orders: 1,
            }),
        };
        assert!(balanced.imbalance().unwrap().abs() < 1e-12);
        assert_eq!(balanced.microprice(), balanced.mid());
    }

    #[test]
    fn one_sided_top_of_book_has_no_imbalance() {
        let one_sided = TopOfBook {
            bid: Some(Level {
                price: p(1),
                shares: 100,
                orders: 1,
            }),
            ask: None,
        };
        assert!(!one_sided.is_two_sided());
        assert_eq!(one_sided.imbalance(), None);
        assert_eq!(one_sided.microprice(), None);
    }
}
