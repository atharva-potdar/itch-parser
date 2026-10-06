use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use crate::itch::{AddOrderMessage, OrderCancelMessage, OrderDeleteMessage};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    #[must_use]
    pub const fn from_indicator(b: u8) -> Option<Self> {
        match b {
            b'B' => Some(Self::Buy),
            b'S' => Some(Self::Sell),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BookError {
    InvalidSide(u8),
    ZeroShares(u64),
    DuplicateOrder(u64),
    UnknownOrder(u64),
    CancelExceedsRemaining {
        order_reference_number: u64,
        remaining: u32,
        requested: u32,
    },
}

impl std::fmt::Display for BookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSide(b) => write!(f, "invalid buy/sell indicator: {b:#04x}"),
            Self::ZeroShares(r) => write!(f, "order {r} has zero shares"),
            Self::DuplicateOrder(r) => write!(f, "order {r} is already in the book"),
            Self::UnknownOrder(r) => write!(f, "order {r} is not in the book"),
            Self::CancelExceedsRemaining {
                order_reference_number,
                remaining,
                requested,
            } => write!(
                f,
                "cancel of {requested} shares exceeds the {remaining} remaining on order {order_reference_number}"
            ),
        }
    }
}

impl std::error::Error for BookError {}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Level {
    pub price: u32,
    pub shares: u64,
}

#[derive(Debug, Clone, Copy)]
struct RestingOrder {
    side: Side,
    price: u32,
    shares: u32,
}

#[derive(Default)]
pub struct OrderBook {
    orders: HashMap<u64, RestingOrder>,
    bids: BTreeMap<u32, u64>,
    asks: BTreeMap<u32, u64>,
}

impl OrderBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn levels_mut(&mut self, side: Side) -> &mut BTreeMap<u32, u64> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    pub fn add_order(&mut self, msg: &AddOrderMessage) -> Result<(), BookError> {
        if msg.shares == 0 {
            return Err(BookError::ZeroShares(msg.order_reference_number));
        }
        let side = Side::from_indicator(msg.buy_sell_indicator)
            .ok_or(BookError::InvalidSide(msg.buy_sell_indicator))?;

        match self.orders.entry(msg.order_reference_number) {
            Entry::Occupied(_) => {
                return Err(BookError::DuplicateOrder(msg.order_reference_number));
            }
            Entry::Vacant(slot) => {
                slot.insert(RestingOrder {
                    side,
                    price: msg.price,
                    shares: msg.shares,
                });
            }
        }

        *self.levels_mut(side).entry(msg.price).or_insert(0) += u64::from(msg.shares);
        Ok(())
    }

    fn reduce_level(&mut self, side: Side, price: u32, shares: u32) {
        let levels = self.levels_mut(side);
        let Some(total) = levels.get_mut(&price) else {
            debug_assert!(false, "order existed but its price level did not");
            return;
        };

        let shares = u64::from(shares);
        debug_assert!(*total >= shares, "level total would underflow");
        *total = total.saturating_sub(shares);
        if *total == 0 {
            levels.remove(&price);
        }
    }

    pub fn cancel_order(&mut self, msg: &OrderCancelMessage) -> Result<(), BookError> {
        let reference = msg.order_reference_number;
        let order = self
            .orders
            .get_mut(&reference)
            .ok_or(BookError::UnknownOrder(reference))?;

        if msg.cancelled_shares > order.shares {
            return Err(BookError::CancelExceedsRemaining {
                order_reference_number: reference,
                remaining: order.shares,
                requested: msg.cancelled_shares,
            });
        }

        order.shares -= msg.cancelled_shares;
        let (side, price, now_empty) = (order.side, order.price, order.shares == 0);

        if now_empty {
            self.orders.remove(&reference);
        }
        self.reduce_level(side, price, msg.cancelled_shares);
        Ok(())
    }

    pub fn delete_order(&mut self, msg: &OrderDeleteMessage) -> Result<(), BookError> {
        let reference = msg.order_reference_number;
        let order = self
            .orders
            .remove(&reference)
            .ok_or(BookError::UnknownOrder(reference))?;

        self.reduce_level(order.side, order.price, order.shares);
        Ok(())
    }

    #[must_use]
    pub fn best_bid(&self) -> Option<Level> {
        self.bids
            .last_key_value()
            .map(|(&price, &shares)| Level { price, shares })
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<Level> {
        self.asks
            .first_key_value()
            .map(|(&price, &shares)| Level { price, shares })
    }
}
