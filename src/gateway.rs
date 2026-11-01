//! A mock order gateway with real risk checks and deterministic paper fills.
//!
//! This is the execution boundary of the demo stack. It is honest about what
//! it is: there is no exchange, no acknowledgement latency, no partial-fill
//! queue, and no reject from a venue. What it *does* model is the part that
//! matters for wiring a system together correctly:
//!
//! * pre-trade risk checks that can reject an order,
//! * an order lifecycle (accepted → resting → filled or cancelled),
//! * a fill model that only fills when the market is actually at or through
//!   the limit price, and
//! * position, average cost, and realised P&L accounting.
//!
//! # Fill model
//!
//! A marketable order fills in full at the touching price:
//!
//! * a buy fills at the best ask when `limit >= ask`,
//! * a sell fills at the best bid when `limit <= bid`.
//!
//! It does not walk the book for size, and it does not consume the displayed
//! liquidity it fills against. Those are the two simplifications a paper
//! gateway usually makes; both are called out here so nobody mistakes the
//! resulting P&L for a backtest.

use std::collections::BTreeMap;

use crate::book::TopOfBook;
use crate::itch::{BuySell, Price};

impl From<crate::alpha::Signal> for OrderIntent {
    fn from(signal: crate::alpha::Signal) -> Self {
        Self {
            stock_locate: signal.stock_locate,
            side: signal.side,
            shares: signal.shares,
            limit_price: signal.limit_price,
        }
    }
}

/// A request to place an order.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OrderIntent {
    pub stock_locate: u16,
    pub side: BuySell,
    pub shares: u32,
    pub limit_price: Price,
}

/// Risk limits enforced by [`MockGateway::submit`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct GatewayConfig {
    /// Largest single order, in shares.
    pub max_order_shares: u32,
    /// Largest absolute position the account may reach, in shares.
    pub max_position: i64,
    /// Largest number of simultaneously resting orders.
    pub max_open_orders: usize,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            max_order_shares: 1_000,
            max_position: 500,
            max_open_orders: 16,
        }
    }
}

/// A resting (or just-accepted) order.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct WorkingOrder {
    pub order_id: u64,
    pub stock_locate: u16,
    pub side: BuySell,
    pub shares: u32,
    pub remaining_shares: u32,
    pub limit_price: Price,
    pub submitted_ns: u64,
}

/// Why an order was refused before it ever rested.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RejectReason {
    ZeroShares,
    QuantityTooLarge { requested: u32, maximum: u32 },
    PositionLimit { resulting: i64, maximum: i64 },
    TooManyOpenOrders { maximum: usize },
    UnsupportedSide(u8),
    NonPositivePrice,
}

/// What happened to a submission.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum GatewayEvent {
    Accepted {
        order_id: u64,
        intent: OrderIntent,
    },
    Rejected {
        intent: OrderIntent,
        reason: RejectReason,
    },
}

/// A completed execution.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct FillEvent {
    pub order_id: u64,
    pub stock_locate: u16,
    pub side: BuySell,
    pub shares: u32,
    pub price: Price,
    pub timestamp_ns: u64,
}

/// Signed position with average-cost accounting.
///
/// `realized_pnl` is in canonical price units (1e-8 currency), so it can be
/// rendered with [`Price`]'s `Display` and never loses precision to a float.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct Position {
    shares: i64,
    average_price: Price,
    realized_pnl: i64,
}

impl Position {
    #[must_use]
    pub const fn shares(&self) -> i64 {
        self.shares
    }

    #[must_use]
    pub const fn average_price(&self) -> Price {
        self.average_price
    }

    #[must_use]
    pub const fn realized_pnl(&self) -> i64 {
        self.realized_pnl
    }

    #[must_use]
    pub const fn is_flat(&self) -> bool {
        self.shares == 0
    }

    /// Unrealised P&L at `mark`, in canonical price units.
    #[must_use]
    pub fn unrealized_pnl(&self, mark: Price) -> i64 {
        let delta = (i128::from(mark.as_x10_8()) - i128::from(self.average_price.as_x10_8()))
            * i128::from(self.shares);
        i64::try_from(delta).unwrap_or(if delta < 0 { i64::MIN } else { i64::MAX })
    }

    /// Applies an execution, updating average cost or realising P&L.
    ///
    /// Crossing through zero resets the average price to the fill price for
    /// whatever position remains on the other side.
    pub fn apply_fill(&mut self, side: BuySell, shares: u32, price: Price) {
        if shares == 0 {
            return;
        }
        let quantity = i64::from(shares);
        let delta = match side {
            BuySell::Buy => quantity,
            BuySell::Sell => -quantity,
            BuySell::Other(_) => return,
        };
        let fill_price = price.as_x10_8();

        let increasing = self.shares == 0 || (self.shares > 0) == (delta > 0);
        if increasing {
            let prior = self.shares.unsigned_abs();
            let total = prior + u64::from(shares);
            let weighted = i128::from(self.average_price.as_x10_8()) * i128::from(prior)
                + i128::from(fill_price) * i128::from(shares);
            let average = weighted / i128::from(total);
            self.average_price =
                Price::from_x10_8_canonical(i64::try_from(average).unwrap_or(fill_price));
            self.shares += delta;
            return;
        }

        let closing = u64::from(shares).min(self.shares.unsigned_abs());
        let realized = if self.shares > 0 {
            (i128::from(fill_price) - i128::from(self.average_price.as_x10_8()))
                * i128::from(closing)
        } else {
            (i128::from(self.average_price.as_x10_8()) - i128::from(fill_price))
                * i128::from(closing)
        };
        self.realized_pnl =
            self.realized_pnl
                .saturating_add(i64::try_from(realized).unwrap_or(if realized < 0 {
                    i64::MIN
                } else {
                    i64::MAX
                }));
        self.shares += delta;
        if self.shares == 0 {
            // Average cost is meaningless when flat, and leaving the last
            // value behind makes a report read as if a position still exists.
            self.average_price = Price::default();
        } else if u64::from(shares) > closing {
            // Flipped through flat; the residual position opened at this fill.
            self.average_price = price;
        }
    }
}

/// Counters describing gateway activity.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct GatewayStats {
    pub submitted: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub cancelled: u64,
    pub fills: u64,
    pub filled_shares: u64,
    /// Signed cash notional of fills, in canonical price units × shares.
    pub filled_notional: i128,
}

/// A paper-trading gateway.
#[derive(Debug, Clone)]
pub struct MockGateway {
    config: GatewayConfig,
    next_order_id: u64,
    open: BTreeMap<u64, WorkingOrder>,
    position: Position,
    stats: GatewayStats,
}

impl MockGateway {
    #[must_use]
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            next_order_id: 1,
            open: BTreeMap::new(),
            position: Position::default(),
            stats: GatewayStats::default(),
        }
    }

    #[must_use]
    pub const fn config(&self) -> &GatewayConfig {
        &self.config
    }

    #[must_use]
    pub const fn position(&self) -> &Position {
        &self.position
    }

    #[must_use]
    pub const fn stats(&self) -> GatewayStats {
        self.stats
    }

    #[must_use]
    pub fn open_order(&self, order_id: u64) -> Option<&WorkingOrder> {
        self.open.get(&order_id)
    }

    /// Resting orders, oldest id first.
    pub fn open_orders(&self) -> impl Iterator<Item = &WorkingOrder> {
        self.open.values()
    }

    #[must_use]
    pub fn open_order_count(&self) -> usize {
        self.open.len()
    }

    /// Risk-checks and rests an order.
    ///
    /// The position projection counts only filled shares, not working orders.
    /// That is a deliberate simplification: a full risk system would reserve
    /// worst-case exposure for resting orders too.
    pub fn submit(&mut self, intent: OrderIntent, timestamp_ns: u64) -> GatewayEvent {
        self.stats.submitted += 1;

        let rejection = if intent.shares == 0 {
            Some(RejectReason::ZeroShares)
        } else if !matches!(intent.side, BuySell::Buy | BuySell::Sell) {
            Some(RejectReason::UnsupportedSide(intent.side.as_byte()))
        } else if intent.limit_price.as_x10_8() <= 0 {
            Some(RejectReason::NonPositivePrice)
        } else if intent.shares > self.config.max_order_shares {
            Some(RejectReason::QuantityTooLarge {
                requested: intent.shares,
                maximum: self.config.max_order_shares,
            })
        } else if self.open.len() >= self.config.max_open_orders {
            Some(RejectReason::TooManyOpenOrders {
                maximum: self.config.max_open_orders,
            })
        } else {
            let signed = i64::from(intent.shares);
            let projected = match intent.side {
                BuySell::Buy => self.position.shares().saturating_add(signed),
                _ => self.position.shares().saturating_sub(signed),
            };
            (projected.unsigned_abs() > self.config.max_position.unsigned_abs()).then_some(
                RejectReason::PositionLimit {
                    resulting: projected,
                    maximum: self.config.max_position,
                },
            )
        };

        if let Some(reason) = rejection {
            self.stats.rejected += 1;
            return GatewayEvent::Rejected { intent, reason };
        }

        let order_id = self.next_order_id;
        self.next_order_id += 1;
        self.stats.accepted += 1;
        self.open.insert(
            order_id,
            WorkingOrder {
                order_id,
                stock_locate: intent.stock_locate,
                side: intent.side,
                shares: intent.shares,
                remaining_shares: intent.shares,
                limit_price: intent.limit_price,
                submitted_ns: timestamp_ns,
            },
        );
        GatewayEvent::Accepted { order_id, intent }
    }

    /// Fills every resting order at `stock_locate` that the quote now crosses.
    ///
    /// Returns the fills in order-id order, which makes the demo output
    /// deterministic.
    pub fn on_quote(
        &mut self,
        stock_locate: u16,
        top: &TopOfBook,
        timestamp_ns: u64,
    ) -> Vec<FillEvent> {
        let candidates: Vec<u64> = self
            .open
            .values()
            .filter(|order| order.stock_locate == stock_locate)
            .map(|order| order.order_id)
            .collect();

        let mut fills = Vec::new();
        for order_id in candidates {
            let Some(order) = self.open.get(&order_id).copied() else {
                continue;
            };
            let execution_price = match order.side {
                BuySell::Buy => top
                    .ask
                    .filter(|ask| order.limit_price >= ask.price)
                    .map(|ask| ask.price),
                BuySell::Sell => top
                    .bid
                    .filter(|bid| order.limit_price <= bid.price)
                    .map(|bid| bid.price),
                BuySell::Other(_) => None,
            };
            let Some(price) = execution_price else {
                continue;
            };

            self.open.remove(&order_id);
            self.position
                .apply_fill(order.side, order.remaining_shares, price);

            let notional = i128::from(price.as_x10_8())
                * i128::from(order.remaining_shares)
                * if order.side == BuySell::Buy { -1 } else { 1 };
            self.stats.fills += 1;
            self.stats.filled_shares += u64::from(order.remaining_shares);
            self.stats.filled_notional += notional;
            fills.push(FillEvent {
                order_id,
                stock_locate,
                side: order.side,
                shares: order.remaining_shares,
                price,
                timestamp_ns,
            });
        }
        fills
    }

    /// Cancels a resting order, returning it if it was still open.
    pub fn cancel(&mut self, order_id: u64) -> Option<WorkingOrder> {
        let order = self.open.remove(&order_id);
        if order.is_some() {
            self.stats.cancelled += 1;
        }
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::Level;

    fn price(cents: i64) -> Price {
        Price::from_x10_8_canonical(cents * 1_000_000)
    }

    fn level(cents: i64, shares: u64) -> Level {
        Level {
            price: price(cents),
            shares,
            orders: 1,
        }
    }

    fn top(bid: Option<(i64, u64)>, ask: Option<(i64, u64)>) -> TopOfBook {
        TopOfBook {
            bid: bid.map(|(cents, shares)| level(cents, shares)),
            ask: ask.map(|(cents, shares)| level(cents, shares)),
        }
    }

    fn intent(side: BuySell, shares: u32, cents: i64) -> OrderIntent {
        OrderIntent {
            stock_locate: 1,
            side,
            shares,
            limit_price: price(cents),
        }
    }

    fn accepted_event(gateway: &mut MockGateway, order: OrderIntent) -> u64 {
        match gateway.submit(order, 0) {
            GatewayEvent::Accepted { order_id, .. } => order_id,
            other @ GatewayEvent::Rejected { .. } => panic!("expected acceptance, got {other:?}"),
        }
    }

    #[test]
    fn submit_accepts_a_valid_order_and_assigns_increasing_ids() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        let first = accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_000));
        let second = accepted_event(&mut gateway, intent(BuySell::Sell, 50, 10_010));

        assert_eq!((first, second), (1, 2));
        assert_eq!(gateway.open_order_count(), 2);
        assert_eq!(gateway.open_order(first).unwrap().remaining_shares, 100);
        assert_eq!(gateway.stats().accepted, 2);
    }

    #[test]
    fn zero_shares_are_rejected() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        assert_eq!(
            gateway.submit(intent(BuySell::Buy, 0, 10_000), 0),
            GatewayEvent::Rejected {
                intent: intent(BuySell::Buy, 0, 10_000),
                reason: RejectReason::ZeroShares
            }
        );
        assert_eq!(gateway.stats().rejected, 1);
    }

    #[test]
    fn oversized_orders_are_rejected() {
        let mut gateway = MockGateway::new(GatewayConfig {
            max_order_shares: 100,
            ..GatewayConfig::default()
        });
        assert_eq!(
            gateway.submit(intent(BuySell::Buy, 101, 10_000), 0),
            GatewayEvent::Rejected {
                intent: intent(BuySell::Buy, 101, 10_000),
                reason: RejectReason::QuantityTooLarge {
                    requested: 101,
                    maximum: 100
                }
            }
        );
    }

    #[test]
    fn the_position_limit_rejects_the_order_that_would_breach_it() {
        let mut gateway = MockGateway::new(GatewayConfig {
            max_position: 100,
            ..GatewayConfig::default()
        });
        assert!(matches!(
            gateway.submit(intent(BuySell::Buy, 150, 10_000), 0),
            GatewayEvent::Rejected {
                reason: RejectReason::PositionLimit {
                    resulting: 150,
                    maximum: 100
                },
                ..
            }
        ));
        assert_eq!(gateway.open_order_count(), 0);
    }

    #[test]
    fn too_many_open_orders_are_refused() {
        let mut gateway = MockGateway::new(GatewayConfig {
            max_open_orders: 1,
            ..GatewayConfig::default()
        });
        accepted_event(&mut gateway, intent(BuySell::Buy, 10, 9_900));
        assert_eq!(
            gateway.submit(intent(BuySell::Buy, 10, 9_900), 0),
            GatewayEvent::Rejected {
                intent: intent(BuySell::Buy, 10, 9_900),
                reason: RejectReason::TooManyOpenOrders { maximum: 1 }
            }
        );
    }

    #[test]
    fn bad_sides_and_non_positive_prices_are_rejected() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        assert_eq!(
            gateway.submit(intent(BuySell::Other(b'?'), 10, 10_000), 0),
            GatewayEvent::Rejected {
                intent: intent(BuySell::Other(b'?'), 10, 10_000),
                reason: RejectReason::UnsupportedSide(b'?')
            }
        );
        assert_eq!(
            gateway.submit(intent(BuySell::Buy, 10, 0), 0),
            GatewayEvent::Rejected {
                intent: intent(BuySell::Buy, 10, 0),
                reason: RejectReason::NonPositivePrice
            }
        );
        assert_eq!(
            gateway.submit(intent(BuySell::Buy, 10, -5), 0),
            GatewayEvent::Rejected {
                intent: intent(BuySell::Buy, 10, -5),
                reason: RejectReason::NonPositivePrice
            }
        );
    }

    #[test]
    fn a_marketable_buy_fills_at_the_ask() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        let id = accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_005));

        let fills = gateway.on_quote(1, &top(Some((10_000, 500)), Some((10_005, 300))), 7);

        assert_eq!(
            fills,
            vec![FillEvent {
                order_id: id,
                stock_locate: 1,
                side: BuySell::Buy,
                shares: 100,
                price: price(10_005),
                timestamp_ns: 7
            }]
        );
        assert_eq!(gateway.position().shares(), 100);
        assert_eq!(gateway.stats().filled_shares, 100);
        assert_eq!(gateway.open_order_count(), 0);
    }

    #[test]
    fn a_buy_below_the_ask_stays_resting() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 9_999));

        let fills = gateway.on_quote(1, &top(Some((10_000, 500)), Some((10_005, 300))), 0);

        assert!(fills.is_empty());
        assert_eq!(gateway.open_order_count(), 1);
        assert!(gateway.position().is_flat());
    }

    #[test]
    fn a_marketable_sell_fills_at_the_bid() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        // Open long first.
        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_005));
        gateway.on_quote(1, &top(Some((10_000, 500)), Some((10_005, 300))), 0);

        let id = accepted_event(&mut gateway, intent(BuySell::Sell, 100, 10_000));
        let fills = gateway.on_quote(1, &top(Some((10_000, 500)), Some((10_005, 300))), 1);

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].order_id, id);
        assert_eq!(fills[0].price, price(10_000));
        assert!(gateway.position().is_flat());
    }

    #[test]
    fn a_round_trip_realises_the_price_difference() {
        let mut gateway = MockGateway::new(GatewayConfig::default());

        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_005));
        gateway.on_quote(1, &top(Some((10_000, 500)), Some((10_005, 300))), 0);
        accepted_event(&mut gateway, intent(BuySell::Sell, 100, 10_000));
        gateway.on_quote(1, &top(Some((10_050, 500)), Some((10_055, 300))), 1);

        assert!(gateway.position().is_flat());
        // Bought at 100.05, sold at 100.50: +0.45 * 100 shares = +45.
        assert_eq!(gateway.position().realized_pnl(), 45 * Price::SCALE);
        assert_eq!(
            Price::from_x10_8_canonical(gateway.position().realized_pnl()).to_string(),
            "45"
        );
    }

    #[test]
    fn average_price_is_size_weighted_across_two_buys() {
        let mut gateway = MockGateway::new(GatewayConfig::default());

        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_000));
        gateway.on_quote(1, &top(None, Some((10_000, 1_000))), 0);
        accepted_event(&mut gateway, intent(BuySell::Buy, 300, 10_010));
        gateway.on_quote(1, &top(None, Some((10_010, 1_000))), 1);

        assert_eq!(gateway.position().shares(), 400);
        // (100 * $100.00 + 300 * $100.10) / 400 = $100.075
        assert_eq!(
            gateway.position().average_price(),
            Price::from_x10_8_canonical(10_007_500_000)
        );
    }

    #[test]
    fn a_short_position_realises_pnl_when_covered_lower() {
        let mut gateway = MockGateway::new(GatewayConfig::default());

        accepted_event(&mut gateway, intent(BuySell::Sell, 100, 10_000));
        gateway.on_quote(1, &top(Some((10_000, 500)), None), 0);
        assert_eq!(gateway.position().shares(), -100);

        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 9_900));
        gateway.on_quote(1, &top(None, Some((9_900, 500))), 1);

        assert!(gateway.position().is_flat());
        // Sold at 100.00, bought at 99.00: +1.00 * 100 = +100.
        assert_eq!(gateway.position().realized_pnl(), 100 * Price::SCALE);
    }

    #[test]
    fn crossing_through_zero_resets_the_average_price() {
        let mut gateway = MockGateway::new(GatewayConfig::default());

        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_000));
        gateway.on_quote(1, &top(None, Some((10_000, 500))), 0);
        // Sell 300: close the 100 long and open a 200 short at 100.50.
        accepted_event(&mut gateway, intent(BuySell::Sell, 300, 10_050));
        gateway.on_quote(1, &top(Some((10_050, 500)), None), 1);

        assert_eq!(gateway.position().shares(), -200);
        assert_eq!(gateway.position().average_price(), price(10_050));
        assert_eq!(gateway.position().realized_pnl(), 50 * Price::SCALE);
    }

    #[test]
    fn unrealized_pnl_tracks_the_mark() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_000));
        gateway.on_quote(1, &top(None, Some((10_000, 500))), 0);

        assert_eq!(
            gateway.position().unrealized_pnl(price(10_050)),
            50 * Price::SCALE
        );
        assert_eq!(gateway.position().unrealized_pnl(price(10_000)), 0);
    }

    #[test]
    fn cancelling_removes_a_resting_order() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        let id = accepted_event(&mut gateway, intent(BuySell::Buy, 100, 9_000));

        assert_eq!(gateway.cancel(id).unwrap().order_id, id);
        assert!(gateway.cancel(id).is_none());
        assert_eq!(gateway.open_order_count(), 0);
        assert_eq!(gateway.stats().cancelled, 1);
    }

    #[test]
    fn fills_only_apply_to_the_requested_locate() {
        let mut gateway = MockGateway::new(GatewayConfig::default());
        accepted_event(&mut gateway, intent(BuySell::Buy, 100, 10_005));
        let mut other = intent(BuySell::Buy, 100, 10_005);
        other.stock_locate = 2;
        accepted_event(&mut gateway, other);

        let fills = gateway.on_quote(2, &top(Some((10_000, 1)), Some((10_005, 1))), 0);

        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].stock_locate, 2);
        assert_eq!(gateway.open_order_count(), 1);
    }
}
