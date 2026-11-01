//! Nasdaq TotalView-ITCH 5.0 message decoding.
//!
//! This module is a pure byte-to-struct transform: it never allocates, never
//! performs I/O, and never panics on untrusted input. Every parser validates
//! the message type byte and the exact message length **before** touching any
//! field offset, so all indexing below is in-bounds by construction. The
//! `every_type_rejects_every_wrong_length` test enforces that invariant for
//! every known type across every length from 0 to 60 bytes.
//!
//! Two numeric conventions apply across the whole protocol:
//!
//! * **Timestamps** are 6-byte big-endian counts of nanoseconds since
//!   midnight, widened into a `u64`. The single exception is
//!   [`DLCRPriceDiscoveryMessage::near_execution_time`], which is a full
//!   8-byte field.
//! * **Prices** are fixed-point. Most are `Price (4)` (units of 1e-4); the
//!   market-wide circuit breaker levels in [`MWCBDeclineLevelMessage`] are
//!   `Price (8)` (units of 1e-8). Both are normalised into [`Price`], which
//!   stores 1e-8 units, so mixed-scale comparisons are impossible.
#![allow(clippy::missing_errors_doc)]

// ---------------------------------------------------------------------------
// Primitive value types
// ---------------------------------------------------------------------------

/// A price normalised to 1e-8 units.
///
/// ITCH mixes two wire scales: `Price (4)` for essentially everything, and
/// `Price (8)` for the MWCB decline levels. Storing both in one canonical
/// scale means downstream code cannot accidentally add a 1e-4 value to a
/// 1e-8 value and be silently wrong by four orders of magnitude.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default)]
pub struct Price(i64);

impl Price {
    /// Number of canonical units in one whole currency unit.
    pub const SCALE: i64 = 100_000_000;

    /// A `Price (4)` wire value: infallible, since `u32::MAX` scaled by 1e4
    /// is far below `i64::MAX`.
    #[must_use]
    pub fn from_x10_4(raw: u32) -> Self {
        Self(i64::from(raw) * 10_000)
    }

    /// A `Price (8)` wire value. Returns `None` when the value does not fit
    /// in an `i64`, which is reachable on the wire but not in practice.
    #[must_use]
    pub fn from_x10_8(raw: u64) -> Option<Self> {
        i64::try_from(raw).ok().map(Self)
    }

    /// The canonical (1e-8) representation.
    #[must_use]
    pub const fn as_x10_8(self) -> i64 {
        self.0
    }

    /// Lossy convenience accessor for display and demo strategies.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(self) -> f64 {
        self.0 as f64 / Self::SCALE as f64
    }
}

/// An 8-byte, space-padded, ASCII security symbol.
///
/// Constructed only through [`Symbol::from_bytes`], so the bytes are always
/// printable ASCII. [`Symbol::as_str`] trims the right padding.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default)]
pub struct Symbol([u8; 8]);

impl Symbol {
    /// Validates that every byte is printable ASCII or a space.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 8]) -> Option<Self> {
        bytes
            .iter()
            .all(|&b| b.is_ascii_graphic() || b == b' ')
            .then_some(Self(bytes))
    }

    /// The raw, space-padded bytes exactly as they appeared on the wire.
    #[must_use]
    pub const fn raw(self) -> [u8; 8] {
        self.0
    }

    /// The symbol with right padding removed. Always valid UTF-8 because the
    /// constructor restricted the bytes to 7-bit ASCII.
    #[must_use]
    pub fn as_str(&self) -> &str {
        let end = self.0.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
        // SAFETY-equivalent argument: `from_bytes` proves ASCII, so this
        // cannot fail; falling back to "" keeps the function panic-free.
        std::str::from_utf8(&self.0[..end]).unwrap_or("")
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Indicator enums
// ---------------------------------------------------------------------------

/// Declares a single-byte indicator enum with an `Other(u8)` catch-all.
///
/// The catch-all is deliberate: Nasdaq has added values to these fields over
/// time (for example `M` = NYSE Texas in market category), and an unrecognised
/// indicator must never cost the caller an otherwise-valid message.
macro_rules! byte_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $byte:literal ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, PartialEq, Eq, Clone, Copy)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )*
            /// A value not defined by ITCH 5.0, preserved verbatim.
            Other(u8),
        }

        impl $name {
            #[must_use]
            pub const fn from_byte(b: u8) -> Self {
                match b {
                    $( $byte => Self::$variant, )*
                    other => Self::Other(other),
                }
            }

            #[must_use]
            pub const fn as_byte(self) -> u8 {
                match self {
                    $( Self::$variant => $byte, )*
                    Self::Other(b) => b,
                }
            }
        }
    };
}

byte_enum! {
    /// `S`ystem Event codes.
    pub enum EventCode {
        /// `O` — start of messages, before any other message of the day.
        StartOfMessages = b'O',
        /// `S` — start of system hours.
        StartOfSystemHours = b'S',
        /// `Q` — start of market hours.
        StartOfMarketHours = b'Q',
        /// `M` — end of market hours.
        EndOfMarketHours = b'M',
        /// `E` — end of system hours.
        EndOfSystemHours = b'E',
        /// `C` — end of messages, the last message of the day.
        EndOfMessages = b'C',
    }
}

byte_enum! {
    /// Listing market or tier of an instrument (`R` messages).
    pub enum MarketCategory {
        /// `Q` — Nasdaq Global Select Market.
        GlobalSelect = b'Q',
        /// `G` — Nasdaq Global Market.
        GlobalMarket = b'G',
        /// `S` — Nasdaq Capital Market.
        CapitalMarket = b'S',
        /// `N` — New York Stock Exchange.
        Nyse = b'N',
        /// `A` — NYSE American.
        NyseAmerican = b'A',
        /// `P` — NYSE Arca.
        NyseArca = b'P',
        /// `M` — NYSE Texas.
        NyseTexas = b'M',
        /// `Z` — BATS Z Exchange.
        BatsZ = b'Z',
        /// `V` — Investors' Exchange.
        Iex = b'V',
        /// A space — not available.
        NotAvailable = b' ',
    }
}

byte_enum! {
    /// Current trading state of a security (`H` messages).
    pub enum TradingState {
        /// `H` — halted across all U.S. equity markets.
        Halted = b'H',
        /// `P` — paused across all U.S. equity markets (Nasdaq-listed only).
        Paused = b'P',
        /// `Q` — quotation-only period during a cross-SRO halt or pause.
        QuotationOnly = b'Q',
        /// `T` — trading on Nasdaq.
        Trading = b'T',
    }
}

byte_enum! {
    /// Operational halt action (`h` messages).
    pub enum HaltAction {
        /// `H` — operationally halted on the identified market.
        Halted = b'H',
        /// `T` — halt lifted, trading resumed.
        Resumed = b'T',
    }
}

byte_enum! {
    /// Market center a `h` message refers to.
    pub enum MarketCode {
        /// `Q` — Nasdaq.
        Nasdaq = b'Q',
        /// `B` — Nasdaq BX.
        NasdaqBx = b'B',
        /// `X` — Nasdaq PSX.
        NasdaqPsx = b'X',
    }
}

byte_enum! {
    /// Which cross an execution belongs to (`Q`, `I` messages).
    pub enum CrossType {
        /// `O` — opening cross.
        Opening = b'O',
        /// `C` — closing cross.
        Closing = b'C',
        /// `H` — cross for IPO and halted/paused securities.
        IpoOrHalt = b'H',
        /// `I` — intraday cross.
        Intraday = b'I',
    }
}

byte_enum! {
    /// Side of an imbalance (`I` messages).
    pub enum ImbalanceDirection {
        /// `B` — buy imbalance.
        Buy = b'B',
        /// `S` — sell imbalance.
        Sell = b'S',
        /// `N` — no imbalance.
        None = b'N',
        /// `O` — insufficient orders to calculate.
        InsufficientOrders = b'O',
    }
}

byte_enum! {
    /// Deviation of the near price from the current reference price.
    pub enum PriceVariationIndicator {
        /// `L` — less than 1%.
        LessThanOnePercent = b'L',
        /// `N` — between 1% and 2%.
        OneToTwoPercent = b'N',
        /// `O` — 2% or more.
        TwoPercentOrMore = b'O',
        /// A space — not applicable.
        NotApplicable = b' ',
    }
}

byte_enum! {
    /// Market-wide circuit breaker level breached (`W` messages).
    pub enum BreachedLevel {
        /// `1` — level 1.
        Level1 = b'1',
        /// `2` — level 2.
        Level2 = b'2',
        /// `3` — level 3.
        Level3 = b'3',
    }
}

byte_enum! {
    /// Whether a DLCR security may be released for trading (`O` messages).
    pub enum OpenEligibilityStatus {
        /// `N` — not eligible.
        NotEligible = b'N',
        /// `Y` — eligible.
        Eligible = b'Y',
    }
}

byte_enum! {
    /// Retail interest side (`N` messages).
    pub enum InterestFlag {
        /// `B` — retail interest on the bid.
        Bid = b'B',
        /// `A` — retail interest on the ask.
        Ask = b'A',
        /// `N` — no retail interest.
        None = b'N',
    }
}

byte_enum! {
    /// Reg SHO short sale price test action (`Y` messages).
    pub enum RegSHOAction {
        /// `0` — no price test in place.
        None = b'0',
        /// `1` — restriction in effect due to an intraday price drop.
        IntradayRestriction = b'1',
        /// `2` — restriction remains in effect.
        RestrictionRemains = b'2',
    }
}

byte_enum! {
    /// Whether a `R` message came from the production or test feed.
    pub enum Authenticity {
        /// `P` — live/production.
        Production = b'P',
        /// `T` — test.
        Test = b'T',
    }
}

/// Side of a resting order or trade.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum BuySell {
    /// `B` — buy.
    Buy,
    /// `S` — sell.
    Sell,
    /// A value not defined by ITCH 5.0, preserved verbatim.
    Other(u8),
}

impl BuySell {
    #[must_use]
    pub const fn from_byte(b: u8) -> Self {
        match b {
            b'B' => Self::Buy,
            b'S' => Self::Sell,
            other => Self::Other(other),
        }
    }

    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::Buy => b'B',
            Self::Sell => b'S',
            Self::Other(b) => b,
        }
    }
}

// ---------------------------------------------------------------------------
// Message type tags
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MessageType {
    SystemEvent,               // 'S'
    StockDirectory,            // 'R'
    StockTradingAction,        // 'H'
    RegSHORestriction,         // 'Y'
    MarketParticipantPosition, // 'L'
    MWCBDeclineLevel,          // 'V'
    MWCBStatus,                // 'W'
    IPOQuotingPeriodUpdate,    // 'K'
    LULDAuctionCollar,         // 'J'
    OperationalHalt,           // 'h'
    AddOrder,                  // 'A'
    AddOrderMPIDAttribution,   // 'F'
    OrderExecuted,             // 'E'
    OrderExecutedPrice,        // 'C'
    OrderCancel,               // 'X'
    OrderDelete,               // 'D'
    OrderReplace,              // 'U'
    Trade,                     // 'P'
    CrossTrade,                // 'Q'
    BrokenTrade,               // 'B'
    NOII,                      // 'I'
    RPII,                      // 'N'
    DLCRPriceDiscovery,        // 'O'
}

impl MessageType {
    /// Every type byte defined by ITCH 5.0.
    pub const ALL: [Self; 23] = [
        Self::SystemEvent,
        Self::StockDirectory,
        Self::StockTradingAction,
        Self::RegSHORestriction,
        Self::MarketParticipantPosition,
        Self::MWCBDeclineLevel,
        Self::MWCBStatus,
        Self::IPOQuotingPeriodUpdate,
        Self::LULDAuctionCollar,
        Self::OperationalHalt,
        Self::AddOrder,
        Self::AddOrderMPIDAttribution,
        Self::OrderExecuted,
        Self::OrderExecutedPrice,
        Self::OrderCancel,
        Self::OrderDelete,
        Self::OrderReplace,
        Self::Trade,
        Self::CrossTrade,
        Self::BrokenTrade,
        Self::NOII,
        Self::RPII,
        Self::DLCRPriceDiscovery,
    ];

    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        match b {
            b'S' => Some(Self::SystemEvent),
            b'R' => Some(Self::StockDirectory),
            b'H' => Some(Self::StockTradingAction),
            b'Y' => Some(Self::RegSHORestriction),
            b'L' => Some(Self::MarketParticipantPosition),
            b'V' => Some(Self::MWCBDeclineLevel),
            b'W' => Some(Self::MWCBStatus),
            b'K' => Some(Self::IPOQuotingPeriodUpdate),
            b'J' => Some(Self::LULDAuctionCollar),
            b'h' => Some(Self::OperationalHalt),
            b'A' => Some(Self::AddOrder),
            b'F' => Some(Self::AddOrderMPIDAttribution),
            b'E' => Some(Self::OrderExecuted),
            b'C' => Some(Self::OrderExecutedPrice),
            b'X' => Some(Self::OrderCancel),
            b'D' => Some(Self::OrderDelete),
            b'U' => Some(Self::OrderReplace),
            b'P' => Some(Self::Trade),
            b'Q' => Some(Self::CrossTrade),
            b'B' => Some(Self::BrokenTrade),
            b'I' => Some(Self::NOII),
            b'N' => Some(Self::RPII),
            b'O' => Some(Self::DLCRPriceDiscovery),
            _ => None,
        }
    }

    /// The exact on-wire length of this message type. ITCH 5.0 messages are
    /// all fixed-length.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        match self {
            Self::SystemEvent => b'S',
            Self::StockDirectory => b'R',
            Self::StockTradingAction => b'H',
            Self::RegSHORestriction => b'Y',
            Self::MarketParticipantPosition => b'L',
            Self::MWCBDeclineLevel => b'V',
            Self::MWCBStatus => b'W',
            Self::IPOQuotingPeriodUpdate => b'K',
            Self::LULDAuctionCollar => b'J',
            Self::OperationalHalt => b'h',
            Self::AddOrder => b'A',
            Self::AddOrderMPIDAttribution => b'F',
            Self::OrderExecuted => b'E',
            Self::OrderExecutedPrice => b'C',
            Self::OrderCancel => b'X',
            Self::OrderDelete => b'D',
            Self::OrderReplace => b'U',
            Self::Trade => b'P',
            Self::CrossTrade => b'Q',
            Self::BrokenTrade => b'B',
            Self::NOII => b'I',
            Self::RPII => b'N',
            Self::DLCRPriceDiscovery => b'O',
        }
    }

    #[must_use]
    pub const fn message_len(self) -> usize {
        match self {
            Self::SystemEvent => SYSTEM_EVENT_MESSAGE_LEN,
            Self::StockDirectory => STOCK_DIRECTORY_MESSAGE_LEN,
            Self::StockTradingAction => STOCK_TRADING_ACTION_MESSAGE_LEN,
            Self::RegSHORestriction => REG_SHO_RESTRICTION_MESSAGE_LEN,
            Self::MarketParticipantPosition => MARKET_PARTICIPANT_MESSAGE_LEN,
            Self::MWCBDeclineLevel => MWCB_DECLINE_LEVEL_MESSAGE_LEN,
            Self::MWCBStatus => MWCB_STATUS_MESSAGE_LEN,
            Self::IPOQuotingPeriodUpdate => IPO_QUOTATION_REQUEST_MESSAGE_LEN,
            Self::LULDAuctionCollar => LULD_ACTION_MESSAGE_LEN,
            Self::OperationalHalt => OPERATIONAL_HALT_MESSAGE_LEN,
            Self::AddOrder => ADD_ORDER_MESSAGE_LEN,
            Self::AddOrderMPIDAttribution => ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN,
            Self::OrderExecuted => ORDER_EXECUTED_MESSAGE_LEN,
            Self::OrderExecutedPrice => ORDER_EXECUTED_PRICE_MESSAGE_LEN,
            Self::OrderCancel => ORDER_CANCEL_MESSAGE_LEN,
            Self::OrderDelete => ORDER_DELETE_MESSAGE_LEN,
            Self::OrderReplace => ORDER_REPLACE_MESSAGE_LEN,
            Self::Trade => TRADE_MESSAGE_LEN,
            Self::CrossTrade => CROSS_TRADE_MESSAGE_LEN,
            Self::BrokenTrade => BROKEN_TRADE_MESSAGE_LEN,
            Self::NOII => NOII_MESSAGE_LEN,
            Self::RPII => RPII_MESSAGE_LEN,
            Self::DLCRPriceDiscovery => DLCR_PRICE_DISCOVERY_MESSAGE_LEN,
        }
    }
}

// ---------------------------------------------------------------------------
// Parse errors
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParseError {
    /// The type byte does not name any ITCH 5.0 message.
    UnknownType(u8),
    /// A type-specific parser was handed a message of a different type.
    WrongType { expected: u8, actual: u8 },
    /// The message length does not match the fixed length for its type.
    WrongLength { expected: usize, actual: usize },
    /// An 8-byte symbol field contained bytes that are not printable ASCII.
    InvalidSymbol,
    /// A wire value did not fit the canonical representation.
    ValueOutOfRange,
}

// ---------------------------------------------------------------------------
// Message payloads
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct SystemEventMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub event_code: EventCode,
}

impl SystemEventMessage {
    /// True for the final message of the trading day (`C`).
    #[must_use]
    pub const fn is_end_of_messages(&self) -> bool {
        matches!(self.event_code, EventCode::EndOfMessages)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)] // mirrors the wire format 1:1
pub struct StockDirectoryMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub market_category: MarketCategory,
    /// Reference data; decoded at the ingest layer (ITCH Appendix D).
    pub financial_status_indicator: u8,
    pub round_lot_size: u32,
    pub round_lots_only: bool,
    /// Reference data; decoded at the ingest layer (ITCH Appendix D).
    pub issue_classification: u8,
    pub issue_subtype: [u8; 2],
    pub authenticity: Authenticity,
    pub short_sale_threshold_indicator: bool,
    pub ipo_flag: bool,
    /// Reference data; decoded at the ingest layer.
    pub luld_reference_price_tier: u8,
    pub etp_flag: bool,
    pub etp_leverage_factor: u32,
    pub inverse_indicator: bool,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct StockTradingActionMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub trading_state: TradingState,
    pub reserved: u8,
    pub reason: [u8; 4],
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct RegSHORestrictionMessage {
    pub locate_code: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub reg_sho_action: RegSHOAction,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MarketParticipantPositionMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub mpid: [u8; 4],
    pub stock: Symbol,
    pub primary_market_maker: bool,
    /// Reference data; decoded at the ingest layer.
    pub market_maker_mode: u8,
    /// Reference data; decoded at the ingest layer.
    pub market_participant_state: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MWCBDeclineLevelMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    // These three are the only `Price (8)` fields in the protocol.
    pub level1: Price,
    pub level2: Price,
    pub level3: Price,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct MWCBStatusMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub breached_level: BreachedLevel,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct IPOQuotingPeriodUpdateMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    /// Seconds since midnight.
    pub ipo_quotation_release_time: u32,
    /// Reference data; decoded at the ingest layer.
    pub ipo_quotation_release_qualifier: u8,
    pub ipo_price: Price,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct LULDAuctionCollarMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub auction_collar_reference_price: Price,
    pub upper_auction_collar_price: Price,
    pub lower_auction_collar_price: Price,
    pub auction_collar_extension: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OperationalHaltMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub market_code: MarketCode,
    pub operational_halt_action: HaltAction,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct AddOrderMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub buy_sell_indicator: BuySell,
    pub shares: u32,
    pub stock: Symbol,
    pub price: Price,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct AddOrderMPIDAttributionMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub buy_sell_indicator: BuySell,
    pub shares: u32,
    pub stock: Symbol,
    pub price: Price,
    pub attribution: [u8; 4],
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OrderExecutedMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub executed_shares: u32,
    pub match_number: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OrderExecutedPriceMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub executed_shares: u32,
    pub match_number: u64,
    pub printable: bool,
    pub execution_price: Price,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OrderCancelMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub cancelled_shares: u32,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OrderDeleteMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct OrderReplaceMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub original_order_reference_number: u64,
    pub new_order_reference_number: u64,
    pub shares: u32,
    pub price: Price,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct TradeMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub order_reference_number: u64,
    pub buy_sell_indicator: BuySell,
    pub shares: u32,
    pub stock: Symbol,
    pub price: Price,
    pub match_number: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct CrossTradeMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub shares: u64,
    pub stock: Symbol,
    pub cross_price: Price,
    pub match_number: u64,
    pub cross_type: CrossType,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct BrokenTradeMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub match_number: u64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct NOIIMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub paired_shares: u64,
    pub imbalance_shares: u64,
    pub imbalance_direction: ImbalanceDirection,
    pub stock: Symbol,
    pub far_price: Price,
    pub near_price: Price,
    pub current_reference_price: Price,
    pub cross_type: CrossType,
    pub price_variation_indicator: PriceVariationIndicator,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct RPIIMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub interest_flag: InterestFlag,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct DLCRPriceDiscoveryMessage {
    pub stock_locate: u16,
    pub tracking_number: u16,
    pub timestamp: u64,
    pub stock: Symbol,
    pub open_eligibility_status: OpenEligibilityStatus,
    pub minimum_allowable_price: Price,
    pub maximum_allowable_price: Price,
    pub near_execution_price: Price,
    /// Nanoseconds since midnight. Unusually, this field is a full 8 bytes
    /// rather than the 6-byte timestamp used everywhere else.
    pub near_execution_time: u64,
    pub lower_price_range_collar: Price,
    pub upper_price_range_collar: Price,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ParsedMessage {
    SystemEvent(SystemEventMessage),
    StockDirectory(StockDirectoryMessage),
    StockTradingAction(StockTradingActionMessage),
    RegSHORestriction(RegSHORestrictionMessage),
    MarketParticipantPosition(MarketParticipantPositionMessage),
    MWCBDeclineLevel(MWCBDeclineLevelMessage),
    MWCBStatus(MWCBStatusMessage),
    IPOQuotingPeriodUpdate(IPOQuotingPeriodUpdateMessage),
    LULDAuctionCollar(LULDAuctionCollarMessage),
    OperationalHalt(OperationalHaltMessage),
    AddOrder(AddOrderMessage),
    AddOrderMPIDAttribution(AddOrderMPIDAttributionMessage),
    OrderExecuted(OrderExecutedMessage),
    OrderExecutedPrice(OrderExecutedPriceMessage),
    OrderCancel(OrderCancelMessage),
    OrderDelete(OrderDeleteMessage),
    OrderReplace(OrderReplaceMessage),
    Trade(TradeMessage),
    CrossTrade(CrossTradeMessage),
    BrokenTrade(BrokenTradeMessage),
    NOII(NOIIMessage),
    RPII(RPIIMessage),
    DLCRPriceDiscovery(DLCRPriceDiscoveryMessage),
}

// ---------------------------------------------------------------------------
// Lengths
// ---------------------------------------------------------------------------

pub const SYSTEM_EVENT_MESSAGE_LEN: usize = 12;
pub const STOCK_DIRECTORY_MESSAGE_LEN: usize = 39;
pub const STOCK_TRADING_ACTION_MESSAGE_LEN: usize = 25;
pub const REG_SHO_RESTRICTION_MESSAGE_LEN: usize = 20;
pub const MARKET_PARTICIPANT_MESSAGE_LEN: usize = 26;
pub const MWCB_DECLINE_LEVEL_MESSAGE_LEN: usize = 35;
pub const MWCB_STATUS_MESSAGE_LEN: usize = 12;
pub const IPO_QUOTATION_REQUEST_MESSAGE_LEN: usize = 28;
pub const LULD_ACTION_MESSAGE_LEN: usize = 35;
pub const OPERATIONAL_HALT_MESSAGE_LEN: usize = 21;
pub const ADD_ORDER_MESSAGE_LEN: usize = 36;
pub const ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN: usize = 40;
pub const ORDER_EXECUTED_MESSAGE_LEN: usize = 31;
pub const ORDER_EXECUTED_PRICE_MESSAGE_LEN: usize = 36;
pub const ORDER_CANCEL_MESSAGE_LEN: usize = 23;
pub const ORDER_DELETE_MESSAGE_LEN: usize = 19;
pub const ORDER_REPLACE_MESSAGE_LEN: usize = 35;
pub const TRADE_MESSAGE_LEN: usize = 44;
pub const CROSS_TRADE_MESSAGE_LEN: usize = 40;
pub const BROKEN_TRADE_MESSAGE_LEN: usize = 19;
pub const NOII_MESSAGE_LEN: usize = 50;
pub const RPII_MESSAGE_LEN: usize = 20;
pub const DLCR_PRICE_DISCOVERY_MESSAGE_LEN: usize = 48;

// ---------------------------------------------------------------------------
// Field readers
//
// Every reader assumes the enclosing parser has already validated `message.len()`
// against the exact wire length. That is enforced by the `length_sweep` test.
// ---------------------------------------------------------------------------

/// Reads a 6-byte big-endian value (ITCH's on-wire timestamp width) and
/// widens it into a `u64`.
fn read_u48_be(bytes: &[u8]) -> u64 {
    let mut widened = [0u8; 8];
    widened[2..8].copy_from_slice(bytes);
    u64::from_be_bytes(widened)
}

const fn read_u16(message: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([message[off], message[off + 1]])
}

const fn read_u32(message: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([
        message[off],
        message[off + 1],
        message[off + 2],
        message[off + 3],
    ])
}

const fn read_u64(message: &[u8], off: usize) -> u64 {
    u64::from_be_bytes([
        message[off],
        message[off + 1],
        message[off + 2],
        message[off + 3],
        message[off + 4],
        message[off + 5],
        message[off + 6],
        message[off + 7],
    ])
}

fn read_array<const N: usize>(message: &[u8], off: usize) -> Result<[u8; N], ParseError> {
    message
        .get(off..off + N)
        .and_then(|slice| <[u8; N]>::try_from(slice).ok())
        .ok_or(ParseError::WrongLength {
            expected: off + N,
            actual: message.len(),
        })
}

fn read_symbol(message: &[u8], off: usize) -> Result<Symbol, ParseError> {
    Symbol::from_bytes(read_array::<8>(message, off)?).ok_or(ParseError::InvalidSymbol)
}

/// A `Price (4)` field.
fn read_price4(message: &[u8], off: usize) -> Price {
    Price::from_x10_4(read_u32(message, off))
}

/// A `Price (8)` field, of which the protocol defines exactly three.
fn read_price8(message: &[u8], off: usize) -> Result<Price, ParseError> {
    Price::from_x10_8(read_u64(message, off)).ok_or(ParseError::ValueOutOfRange)
}

/// Reads a `Y`/`N` field, mapping anything else to `false`.
const fn read_flag(message: &[u8], off: usize) -> bool {
    message[off] == b'Y'
}

/// Validates type byte and exact length. Must be the first statement of every
/// type-specific parser.
macro_rules! check {
    ($message:expr, $type_byte:literal, $len:expr) => {
        if $message.len() != $len {
            return Err(ParseError::WrongLength {
                expected: $len,
                actual: $message.len(),
            });
        }
        if $message[0] != $type_byte {
            return Err(ParseError::WrongType {
                expected: $type_byte,
                actual: $message[0],
            });
        }
    };
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Parses a single ITCH 5.0 message.
///
/// Unlike the previous `Option`-returning API, this distinguishes "I do not
/// know this type" from "I know this type and the message is malformed", which
/// is what a feed handler needs in order to report loss accurately.
pub fn parse_message(message: &[u8]) -> Result<ParsedMessage, ParseError> {
    let Some(&type_byte) = message.first() else {
        return Err(ParseError::WrongLength {
            expected: 1,
            actual: 0,
        });
    };
    let Some(message_type) = MessageType::from_byte(type_byte) else {
        return Err(ParseError::UnknownType(type_byte));
    };
    match message_type {
        MessageType::SystemEvent => parse_system_event(message).map(ParsedMessage::SystemEvent),
        MessageType::StockDirectory => {
            parse_stock_directory(message).map(ParsedMessage::StockDirectory)
        }
        MessageType::StockTradingAction => {
            parse_stock_trading_action(message).map(ParsedMessage::StockTradingAction)
        }
        MessageType::RegSHORestriction => {
            parse_reg_sho_restriction(message).map(ParsedMessage::RegSHORestriction)
        }
        MessageType::MarketParticipantPosition => {
            parse_market_participant_position(message).map(ParsedMessage::MarketParticipantPosition)
        }
        MessageType::MWCBDeclineLevel => {
            parse_mwcb_decline_level(message).map(ParsedMessage::MWCBDeclineLevel)
        }
        MessageType::MWCBStatus => parse_mwcb_status(message).map(ParsedMessage::MWCBStatus),
        MessageType::IPOQuotingPeriodUpdate => {
            parse_ipo_quoting_period_update(message).map(ParsedMessage::IPOQuotingPeriodUpdate)
        }
        MessageType::LULDAuctionCollar => {
            parse_luld_action_collar(message).map(ParsedMessage::LULDAuctionCollar)
        }
        MessageType::OperationalHalt => {
            parse_operational_halt(message).map(ParsedMessage::OperationalHalt)
        }
        MessageType::AddOrder => parse_add_order(message).map(ParsedMessage::AddOrder),
        MessageType::AddOrderMPIDAttribution => {
            parse_add_order_mpid_attribution(message).map(ParsedMessage::AddOrderMPIDAttribution)
        }
        MessageType::OrderExecuted => {
            parse_order_executed(message).map(ParsedMessage::OrderExecuted)
        }
        MessageType::OrderExecutedPrice => {
            parse_order_executed_price(message).map(ParsedMessage::OrderExecutedPrice)
        }
        MessageType::OrderCancel => parse_order_cancel(message).map(ParsedMessage::OrderCancel),
        MessageType::OrderDelete => parse_order_delete(message).map(ParsedMessage::OrderDelete),
        MessageType::OrderReplace => parse_order_replace(message).map(ParsedMessage::OrderReplace),
        MessageType::Trade => parse_trade(message).map(ParsedMessage::Trade),
        MessageType::CrossTrade => parse_cross_trade(message).map(ParsedMessage::CrossTrade),
        MessageType::BrokenTrade => parse_broken_trade(message).map(ParsedMessage::BrokenTrade),
        MessageType::NOII => parse_noii(message).map(ParsedMessage::NOII),
        MessageType::RPII => parse_rpii(message).map(ParsedMessage::RPII),
        MessageType::DLCRPriceDiscovery => {
            parse_dlcr_price_discovery(message).map(ParsedMessage::DLCRPriceDiscovery)
        }
    }
}

// ---------------------------------------------------------------------------
// Individual parsers
// ---------------------------------------------------------------------------

pub fn parse_system_event(message: &[u8]) -> Result<SystemEventMessage, ParseError> {
    check!(message, b'S', SYSTEM_EVENT_MESSAGE_LEN);
    Ok(SystemEventMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        event_code: EventCode::from_byte(message[11]),
    })
}

pub fn parse_stock_directory(message: &[u8]) -> Result<StockDirectoryMessage, ParseError> {
    check!(message, b'R', STOCK_DIRECTORY_MESSAGE_LEN);
    Ok(StockDirectoryMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        market_category: MarketCategory::from_byte(message[19]),
        financial_status_indicator: message[20],
        round_lot_size: read_u32(message, 21),
        round_lots_only: read_flag(message, 25),
        issue_classification: message[26],
        issue_subtype: read_array::<2>(message, 27)?,
        authenticity: Authenticity::from_byte(message[29]),
        short_sale_threshold_indicator: read_flag(message, 30),
        ipo_flag: read_flag(message, 31),
        luld_reference_price_tier: message[32],
        etp_flag: read_flag(message, 33),
        etp_leverage_factor: read_u32(message, 34),
        inverse_indicator: read_flag(message, 38),
    })
}

pub fn parse_stock_trading_action(message: &[u8]) -> Result<StockTradingActionMessage, ParseError> {
    check!(message, b'H', STOCK_TRADING_ACTION_MESSAGE_LEN);
    Ok(StockTradingActionMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        trading_state: TradingState::from_byte(message[19]),
        reserved: message[20],
        reason: read_array::<4>(message, 21)?,
    })
}

pub fn parse_reg_sho_restriction(message: &[u8]) -> Result<RegSHORestrictionMessage, ParseError> {
    check!(message, b'Y', REG_SHO_RESTRICTION_MESSAGE_LEN);
    Ok(RegSHORestrictionMessage {
        locate_code: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        reg_sho_action: RegSHOAction::from_byte(message[19]),
    })
}

pub fn parse_market_participant_position(
    message: &[u8],
) -> Result<MarketParticipantPositionMessage, ParseError> {
    check!(message, b'L', MARKET_PARTICIPANT_MESSAGE_LEN);
    Ok(MarketParticipantPositionMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        mpid: read_array::<4>(message, 11)?,
        stock: read_symbol(message, 15)?,
        primary_market_maker: read_flag(message, 23),
        market_maker_mode: message[24],
        market_participant_state: message[25],
    })
}

pub fn parse_mwcb_decline_level(message: &[u8]) -> Result<MWCBDeclineLevelMessage, ParseError> {
    check!(message, b'V', MWCB_DECLINE_LEVEL_MESSAGE_LEN);
    Ok(MWCBDeclineLevelMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        level1: read_price8(message, 11)?,
        level2: read_price8(message, 19)?,
        level3: read_price8(message, 27)?,
    })
}

pub fn parse_mwcb_status(message: &[u8]) -> Result<MWCBStatusMessage, ParseError> {
    check!(message, b'W', MWCB_STATUS_MESSAGE_LEN);
    Ok(MWCBStatusMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        breached_level: BreachedLevel::from_byte(message[11]),
    })
}

pub fn parse_ipo_quoting_period_update(
    message: &[u8],
) -> Result<IPOQuotingPeriodUpdateMessage, ParseError> {
    check!(message, b'K', IPO_QUOTATION_REQUEST_MESSAGE_LEN);
    Ok(IPOQuotingPeriodUpdateMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        ipo_quotation_release_time: read_u32(message, 19),
        ipo_quotation_release_qualifier: message[23],
        ipo_price: read_price4(message, 24),
    })
}

pub fn parse_luld_action_collar(message: &[u8]) -> Result<LULDAuctionCollarMessage, ParseError> {
    check!(message, b'J', LULD_ACTION_MESSAGE_LEN);
    Ok(LULDAuctionCollarMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        auction_collar_reference_price: read_price4(message, 19),
        upper_auction_collar_price: read_price4(message, 23),
        lower_auction_collar_price: read_price4(message, 27),
        auction_collar_extension: read_u32(message, 31),
    })
}

pub fn parse_operational_halt(message: &[u8]) -> Result<OperationalHaltMessage, ParseError> {
    check!(message, b'h', OPERATIONAL_HALT_MESSAGE_LEN);
    Ok(OperationalHaltMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        market_code: MarketCode::from_byte(message[19]),
        operational_halt_action: HaltAction::from_byte(message[20]),
    })
}

pub fn parse_add_order(message: &[u8]) -> Result<AddOrderMessage, ParseError> {
    check!(message, b'A', ADD_ORDER_MESSAGE_LEN);
    Ok(AddOrderMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
        buy_sell_indicator: BuySell::from_byte(message[19]),
        shares: read_u32(message, 20),
        stock: read_symbol(message, 24)?,
        price: read_price4(message, 32),
    })
}

pub fn parse_add_order_mpid_attribution(
    message: &[u8],
) -> Result<AddOrderMPIDAttributionMessage, ParseError> {
    check!(message, b'F', ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN);
    Ok(AddOrderMPIDAttributionMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
        buy_sell_indicator: BuySell::from_byte(message[19]),
        shares: read_u32(message, 20),
        stock: read_symbol(message, 24)?,
        price: read_price4(message, 32),
        attribution: read_array::<4>(message, 36)?,
    })
}

pub fn parse_order_executed(message: &[u8]) -> Result<OrderExecutedMessage, ParseError> {
    check!(message, b'E', ORDER_EXECUTED_MESSAGE_LEN);
    Ok(OrderExecutedMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
        executed_shares: read_u32(message, 19),
        match_number: read_u64(message, 23),
    })
}

pub fn parse_order_executed_price(message: &[u8]) -> Result<OrderExecutedPriceMessage, ParseError> {
    check!(message, b'C', ORDER_EXECUTED_PRICE_MESSAGE_LEN);
    Ok(OrderExecutedPriceMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
        executed_shares: read_u32(message, 19),
        match_number: read_u64(message, 23),
        printable: read_flag(message, 31),
        execution_price: read_price4(message, 32),
    })
}

pub fn parse_order_cancel(message: &[u8]) -> Result<OrderCancelMessage, ParseError> {
    check!(message, b'X', ORDER_CANCEL_MESSAGE_LEN);
    Ok(OrderCancelMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
        cancelled_shares: read_u32(message, 19),
    })
}

pub fn parse_order_delete(message: &[u8]) -> Result<OrderDeleteMessage, ParseError> {
    check!(message, b'D', ORDER_DELETE_MESSAGE_LEN);
    Ok(OrderDeleteMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
    })
}

pub fn parse_order_replace(message: &[u8]) -> Result<OrderReplaceMessage, ParseError> {
    check!(message, b'U', ORDER_REPLACE_MESSAGE_LEN);
    Ok(OrderReplaceMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        original_order_reference_number: read_u64(message, 11),
        new_order_reference_number: read_u64(message, 19),
        shares: read_u32(message, 27),
        price: read_price4(message, 31),
    })
}

pub fn parse_trade(message: &[u8]) -> Result<TradeMessage, ParseError> {
    check!(message, b'P', TRADE_MESSAGE_LEN);
    Ok(TradeMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        order_reference_number: read_u64(message, 11),
        buy_sell_indicator: BuySell::from_byte(message[19]),
        shares: read_u32(message, 20),
        stock: read_symbol(message, 24)?,
        price: read_price4(message, 32),
        match_number: read_u64(message, 36),
    })
}

pub fn parse_cross_trade(message: &[u8]) -> Result<CrossTradeMessage, ParseError> {
    check!(message, b'Q', CROSS_TRADE_MESSAGE_LEN);
    Ok(CrossTradeMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        shares: read_u64(message, 11),
        stock: read_symbol(message, 19)?,
        cross_price: read_price4(message, 27),
        match_number: read_u64(message, 31),
        cross_type: CrossType::from_byte(message[39]),
    })
}

pub fn parse_broken_trade(message: &[u8]) -> Result<BrokenTradeMessage, ParseError> {
    check!(message, b'B', BROKEN_TRADE_MESSAGE_LEN);
    Ok(BrokenTradeMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        match_number: read_u64(message, 11),
    })
}

pub fn parse_noii(message: &[u8]) -> Result<NOIIMessage, ParseError> {
    check!(message, b'I', NOII_MESSAGE_LEN);
    Ok(NOIIMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        paired_shares: read_u64(message, 11),
        imbalance_shares: read_u64(message, 19),
        imbalance_direction: ImbalanceDirection::from_byte(message[27]),
        stock: read_symbol(message, 28)?,
        far_price: read_price4(message, 36),
        near_price: read_price4(message, 40),
        current_reference_price: read_price4(message, 44),
        cross_type: CrossType::from_byte(message[48]),
        price_variation_indicator: PriceVariationIndicator::from_byte(message[49]),
    })
}

pub fn parse_rpii(message: &[u8]) -> Result<RPIIMessage, ParseError> {
    check!(message, b'N', RPII_MESSAGE_LEN);
    Ok(RPIIMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        interest_flag: InterestFlag::from_byte(message[19]),
    })
}

pub fn parse_dlcr_price_discovery(message: &[u8]) -> Result<DLCRPriceDiscoveryMessage, ParseError> {
    check!(message, b'O', DLCR_PRICE_DISCOVERY_MESSAGE_LEN);
    Ok(DLCRPriceDiscoveryMessage {
        stock_locate: read_u16(message, 1),
        tracking_number: read_u16(message, 3),
        timestamp: read_u48_be(&message[5..11]),
        stock: read_symbol(message, 11)?,
        open_eligibility_status: OpenEligibilityStatus::from_byte(message[19]),
        minimum_allowable_price: read_price4(message, 20),
        maximum_allowable_price: read_price4(message, 24),
        near_execution_price: read_price4(message, 28),
        near_execution_time: read_u64(message, 32),
        lower_price_range_collar: read_price4(message, 40),
        upper_price_range_collar: read_price4(message, 44),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// <Generated by Claude.ai>
#[cfg(test)]
mod tests {
    use super::*;

    const TS: u64 = 0x0102_0304_0506;

    #[allow(clippy::trivially_copy_pass_by_ref)] // keeps call sites terse
    fn sym(s: &[u8; 8]) -> Symbol {
        Symbol::from_bytes(*s).expect("test symbol should be valid ASCII")
    }

    fn p4(v: u32) -> Price {
        Price::from_x10_4(v)
    }

    fn p8(v: u64) -> Price {
        Price::from_x10_8(v).expect("test price should fit i64")
    }

    /// Builds a message, asserting the builder emitted the exact wire length.
    fn build(type_byte: u8, len: usize, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut buf = Vec::with_capacity(len);
        buf.push(type_byte);
        body(&mut buf);
        assert_eq!(buf.len(), len, "test builder produced the wrong length");
        buf
    }

    fn push_header(buf: &mut Vec<u8>, locate: u16, tracking: u16, timestamp: u64) {
        buf.extend_from_slice(&locate.to_be_bytes());
        buf.extend_from_slice(&tracking.to_be_bytes());
        buf.extend_from_slice(&timestamp.to_be_bytes()[2..8]);
    }

    /// Asserts the three failure modes every parser shares: wrong type byte,
    /// one byte short, one byte long.
    fn assert_rejects<T: std::fmt::Debug + PartialEq>(
        parse: impl Fn(&[u8]) -> Result<T, ParseError>,
        bytes: &[u8],
    ) {
        let type_byte = bytes[0];
        assert_eq!(
            parse(&bytes[..bytes.len() - 1]),
            Err(ParseError::WrongLength {
                expected: bytes.len(),
                actual: bytes.len() - 1,
            }),
            "one byte short must be rejected"
        );
        let mut longer = bytes.to_vec();
        longer.push(0);
        assert_eq!(
            parse(&longer),
            Err(ParseError::WrongLength {
                expected: bytes.len(),
                actual: bytes.len() + 1,
            }),
            "one byte long must be rejected"
        );
        let mut wrong_type = bytes.to_vec();
        wrong_type[0] = b'Z';
        assert_eq!(
            parse(&wrong_type),
            Err(ParseError::WrongType {
                expected: type_byte,
                actual: b'Z',
            }),
            "wrong type byte must be rejected"
        );
    }

    // ---- Price / Symbol ----

    #[test]
    fn price_scales_are_normalised_to_1e_8() {
        assert_eq!(p4(1_425_000).as_x10_8(), 14_250_000_000);
        assert_eq!(p8(14_250_000_000).as_x10_8(), 14_250_000_000);
        assert_eq!(p4(1_425_000), p8(14_250_000_000));
        assert_eq!(p4(0), Price::default());
        assert!((p4(12_345).as_f64() - 1.2345).abs() < f64::EPSILON);
    }

    #[test]
    fn price_from_x10_8_rejects_values_above_i64_max() {
        assert_eq!(Price::from_x10_8(u64::MAX), None);
        assert_eq!(Price::from_x10_8(i64::MAX as u64), Some(Price(i64::MAX)));
    }

    #[test]
    fn symbol_trims_padding_and_rejects_non_ascii() {
        assert_eq!(sym(b"AAPL    ").as_str(), "AAPL");
        assert_eq!(sym(b"BRK.B   ").as_str(), "BRK.B");
        assert_eq!(sym(b"        ").as_str(), "");
        assert_eq!(sym(b"ABCDEFGH").as_str(), "ABCDEFGH");
        assert_eq!(Symbol::from_bytes([0xFF; 8]), None);
        assert_eq!(Symbol::from_bytes(*b"\0\0\0\0\0\0\0\0"), None);
        assert_eq!(sym(b"AAPL    ").raw(), *b"AAPL    ");
    }

    // ---- Dispatch ----

    #[test]
    fn parse_message_unknown_type_is_distinguishable_from_bad_length() {
        assert_eq!(
            parse_message(b"Z\x00\x00\x00\x00"),
            Err(ParseError::UnknownType(b'Z'))
        );
        assert_eq!(
            parse_message(&[]),
            Err(ParseError::WrongLength {
                expected: 1,
                actual: 0
            })
        );
        // Known type, wrong length: a different error entirely.
        assert_eq!(
            parse_message(b"S\x00\x00\x00\x00"),
            Err(ParseError::WrongLength {
                expected: 12,
                actual: 5
            })
        );
    }

    #[test]
    fn message_type_byte_round_trips() {
        for message_type in MessageType::ALL {
            let byte = message_type.as_byte();
            assert_eq!(MessageType::from_byte(byte), Some(message_type));
        }
    }

    #[test]
    fn every_type_rejects_every_wrong_length() {
        for message_type in MessageType::ALL {
            let type_byte = message_type.as_byte();
            let expected_len = message_type.message_len();
            for len in 0..=60usize {
                let mut bytes = vec![0u8; len];
                if len > 0 {
                    bytes[0] = type_byte;
                }
                let result = parse_message(&bytes);
                if len != expected_len {
                    assert!(
                        result.is_err(),
                        "{message_type:?} with length {len} should be rejected, got {result:?}"
                    );
                }
                // The important assertion is that nothing above panicked.
            }
        }
    }

    #[test]
    fn every_enum_round_trips_and_has_a_catch_all() {
        macro_rules! check_enum {
            ($name:ident) => {
                for byte in 0u8..=255 {
                    let value = $name::from_byte(byte);
                    assert_eq!(
                        value.as_byte(),
                        byte,
                        concat!(stringify!($name), " must round-trip")
                    );
                }
            };
        }
        check_enum!(EventCode);
        check_enum!(MarketCategory);
        check_enum!(TradingState);
        check_enum!(HaltAction);
        check_enum!(MarketCode);
        check_enum!(CrossType);
        check_enum!(ImbalanceDirection);
        check_enum!(PriceVariationIndicator);
        check_enum!(BreachedLevel);
        check_enum!(OpenEligibilityStatus);
        check_enum!(InterestFlag);
        check_enum!(RegSHOAction);
        check_enum!(Authenticity);
        check_enum!(BuySell);

        assert_eq!(EventCode::from_byte(b'C'), EventCode::EndOfMessages);
        assert_eq!(EventCode::from_byte(b'x'), EventCode::Other(b'x'));
        assert_eq!(BuySell::from_byte(b'B'), BuySell::Buy);
        assert_eq!(BuySell::from_byte(b'?'), BuySell::Other(b'?'));
    }

    // ---- SystemEvent ----

    fn system_event_bytes(locate: u16, tracking: u16, timestamp: u64, event_code: u8) -> Vec<u8> {
        build(b'S', SYSTEM_EVENT_MESSAGE_LEN, |buf| {
            push_header(buf, locate, tracking, timestamp);
            buf.push(event_code);
        })
    }

    #[test]
    fn system_event_parses_and_detects_end_of_messages() {
        let bytes = system_event_bytes(0x1234, 0x5678, TS, b'C');
        let expected = SystemEventMessage {
            stock_locate: 0x1234,
            tracking_number: 0x5678,
            timestamp: TS,
            event_code: EventCode::EndOfMessages,
        };
        assert_eq!(parse_system_event(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::SystemEvent(expected))
        );
        assert!(expected.is_end_of_messages());
        assert_rejects(parse_system_event, &bytes);

        let start = system_event_bytes(1, 2, TS, b'O');
        assert!(!parse_system_event(&start).unwrap().is_end_of_messages());
    }

    // ---- StockDirectory ----

    fn stock_directory_bytes() -> Vec<u8> {
        build(b'R', STOCK_DIRECTORY_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"AAPL    ");
            buf.push(b'Q'); // market category
            buf.push(b'N'); // financial status
            buf.extend_from_slice(&100u32.to_be_bytes());
            buf.push(b'Y'); // round lots only
            buf.push(b'C'); // issue classification
            buf.extend_from_slice(b"XY"); // issue subtype
            buf.push(b'P'); // authenticity
            buf.push(b'N'); // short sale threshold
            buf.push(b'Y'); // ipo flag
            buf.push(b'1'); // luld tier
            buf.push(b'N'); // etp flag
            buf.extend_from_slice(&9999u32.to_be_bytes());
            buf.push(b'N'); // inverse
        })
    }

    #[test]
    fn stock_directory_parses_every_field() {
        let bytes = stock_directory_bytes();
        let expected = StockDirectoryMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"AAPL    "),
            market_category: MarketCategory::GlobalSelect,
            financial_status_indicator: b'N',
            round_lot_size: 100,
            round_lots_only: true,
            issue_classification: b'C',
            issue_subtype: *b"XY",
            authenticity: Authenticity::Production,
            short_sale_threshold_indicator: false,
            ipo_flag: true,
            luld_reference_price_tier: b'1',
            etp_flag: false,
            etp_leverage_factor: 9999,
            inverse_indicator: false,
        };
        assert_eq!(parse_stock_directory(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::StockDirectory(expected))
        );
        assert_rejects(parse_stock_directory, &bytes);
    }

    #[test]
    fn stock_directory_rejects_a_non_ascii_symbol() {
        let mut bytes = stock_directory_bytes();
        bytes[11] = 0xFF;
        assert_eq!(
            parse_stock_directory(&bytes),
            Err(ParseError::InvalidSymbol)
        );
    }

    // ---- StockTradingAction ----

    fn stock_trading_action_bytes() -> Vec<u8> {
        build(b'H', STOCK_TRADING_ACTION_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"MSFT    ");
            buf.push(b'T'); // trading state
            buf.push(0x00); // reserved
            buf.extend_from_slice(b"REG1");
        })
    }

    #[test]
    fn stock_trading_action_parses() {
        let bytes = stock_trading_action_bytes();
        let expected = StockTradingActionMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"MSFT    "),
            trading_state: TradingState::Trading,
            reserved: 0,
            reason: *b"REG1",
        };
        assert_eq!(parse_stock_trading_action(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::StockTradingAction(expected))
        );
        assert_rejects(parse_stock_trading_action, &bytes);
    }

    // ---- RegSHO ----

    fn reg_sho_bytes() -> Vec<u8> {
        build(b'Y', REG_SHO_RESTRICTION_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"TSLA    ");
            buf.push(b'1');
        })
    }

    #[test]
    fn reg_sho_parses() {
        let bytes = reg_sho_bytes();
        let expected = RegSHORestrictionMessage {
            locate_code: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"TSLA    "),
            reg_sho_action: RegSHOAction::IntradayRestriction,
        };
        assert_eq!(parse_reg_sho_restriction(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::RegSHORestriction(expected))
        );
        assert_rejects(parse_reg_sho_restriction, &bytes);
    }

    // ---- MarketParticipantPosition ----

    fn market_participant_matrix_bytes() -> Vec<u8> {
        build(b'L', MARKET_PARTICIPANT_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"ABCD");
            buf.extend_from_slice(b"GOOG    ");
            buf.push(b'Y');
            buf.push(b'1');
            buf.push(b'A');
        })
    }

    #[test]
    fn market_participant_position_parses() {
        let bytes = market_participant_matrix_bytes();
        let expected = MarketParticipantPositionMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            mpid: *b"ABCD",
            stock: sym(b"GOOG    "),
            primary_market_maker: true,
            market_maker_mode: b'1',
            market_participant_state: b'A',
        };
        assert_eq!(parse_market_participant_position(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::MarketParticipantPosition(expected))
        );
        assert_rejects(parse_market_participant_position, &bytes);
    }

    // ---- MWCB ----

    #[test]
    fn mwcb_decline_level_uses_the_1e_8_scale() {
        let bytes = build(b'V', MWCB_DECLINE_LEVEL_MESSAGE_LEN, |buf| {
            push_header(buf, 0, 0x2222, TS);
            buf.extend_from_slice(&1_000_000_000_000u64.to_be_bytes());
            buf.extend_from_slice(&2_000_000_000_000u64.to_be_bytes());
            buf.extend_from_slice(&3_000_000_000_000u64.to_be_bytes());
        });
        let expected = MWCBDeclineLevelMessage {
            stock_locate: 0,
            tracking_number: 0x2222,
            timestamp: TS,
            level1: p8(1_000_000_000_000),
            level2: p8(2_000_000_000_000),
            level3: p8(3_000_000_000_000),
        };
        assert_eq!(parse_mwcb_decline_level(&bytes), Ok(expected));
        // 1e12 canonical units == 10,000.00000000 whole units, which is 1e8
        // in Price (4) terms -- a scale mix-up would be four orders out.
        assert_eq!(expected.level1.as_x10_8(), 1_000_000_000_000);
        assert_rejects(parse_mwcb_decline_level, &bytes);
    }

    #[test]
    fn mwcb_status_parses() {
        let bytes = build(b'W', MWCB_STATUS_MESSAGE_LEN, |buf| {
            push_header(buf, 0, 0x2222, TS);
            buf.push(b'2');
        });
        let expected = MWCBStatusMessage {
            stock_locate: 0,
            tracking_number: 0x2222,
            timestamp: TS,
            breached_level: BreachedLevel::Level2,
        };
        assert_eq!(parse_mwcb_status(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::MWCBStatus(expected))
        );
        assert_rejects(parse_mwcb_status, &bytes);
    }

    // ---- IPO ----

    #[test]
    fn ipo_quoting_period_update_parses() {
        let bytes = build(b'K', IPO_QUOTATION_REQUEST_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"ABCD    ");
            buf.extend_from_slice(&34_200u32.to_be_bytes());
            buf.push(b'A');
            buf.extend_from_slice(&200_000u32.to_be_bytes());
        });
        let expected = IPOQuotingPeriodUpdateMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"ABCD    "),
            ipo_quotation_release_time: 34_200,
            ipo_quotation_release_qualifier: b'A',
            ipo_price: p4(200_000),
        };
        assert_eq!(parse_ipo_quoting_period_update(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::IPOQuotingPeriodUpdate(expected))
        );
        assert_rejects(parse_ipo_quoting_period_update, &bytes);
    }

    // ---- LULD ----

    #[test]
    fn luld_auction_collar_parses() {
        let bytes = build(b'J', LULD_ACTION_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"EFGH    ");
            buf.extend_from_slice(&100_000u32.to_be_bytes());
            buf.extend_from_slice(&110_000u32.to_be_bytes());
            buf.extend_from_slice(&90_000u32.to_be_bytes());
            buf.extend_from_slice(&2u32.to_be_bytes());
        });
        let expected = LULDAuctionCollarMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"EFGH    "),
            auction_collar_reference_price: p4(100_000),
            upper_auction_collar_price: p4(110_000),
            lower_auction_collar_price: p4(90_000),
            auction_collar_extension: 2,
        };
        assert_eq!(parse_luld_action_collar(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::LULDAuctionCollar(expected))
        );
        assert_rejects(parse_luld_action_collar, &bytes);
    }

    // ---- OperationalHalt ----

    #[test]
    fn operational_halt_parses() {
        let bytes = build(b'h', OPERATIONAL_HALT_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"IJKL    ");
            buf.push(b'Q');
            buf.push(b'H');
        });
        let expected = OperationalHaltMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"IJKL    "),
            market_code: MarketCode::Nasdaq,
            operational_halt_action: HaltAction::Halted,
        };
        assert_eq!(parse_operational_halt(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::OperationalHalt(expected))
        );
        assert_rejects(parse_operational_halt, &bytes);
    }

    // ---- AddOrder ----

    fn add_order_bytes() -> Vec<u8> {
        build(b'A', ADD_ORDER_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.push(b'B');
            buf.extend_from_slice(&500u32.to_be_bytes());
            buf.extend_from_slice(b"NVDA    ");
            buf.extend_from_slice(&1_425_000u32.to_be_bytes());
        })
    }

    #[test]
    fn add_order_parses() {
        let bytes = add_order_bytes();
        let expected = AddOrderMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
            buy_sell_indicator: BuySell::Buy,
            shares: 500,
            stock: sym(b"NVDA    "),
            price: p4(1_425_000),
        };
        assert_eq!(parse_add_order(&bytes), Ok(expected));
        assert_eq!(parse_message(&bytes), Ok(ParsedMessage::AddOrder(expected)));
        assert_rejects(parse_add_order, &bytes);
    }

    // ---- AddOrderMPIDAttribution ----

    #[test]
    fn add_order_mpid_attribution_parses() {
        let bytes = build(b'F', ADD_ORDER_MPID_ATTRIBUTION_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.push(b'S');
            buf.extend_from_slice(&300u32.to_be_bytes());
            buf.extend_from_slice(b"AMD     ");
            buf.extend_from_slice(&850_000u32.to_be_bytes());
            buf.extend_from_slice(b"EDGX");
        });
        let expected = AddOrderMPIDAttributionMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
            buy_sell_indicator: BuySell::Sell,
            shares: 300,
            stock: sym(b"AMD     "),
            price: p4(850_000),
            attribution: *b"EDGX",
        };
        assert_eq!(parse_add_order_mpid_attribution(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::AddOrderMPIDAttribution(expected))
        );
        assert_rejects(parse_add_order_mpid_attribution, &bytes);
    }

    // ---- OrderExecuted ----

    #[test]
    fn order_executed_parses() {
        let bytes = build(b'E', ORDER_EXECUTED_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.extend_from_slice(&200u32.to_be_bytes());
            buf.extend_from_slice(&0x99AA_BBCC_DDEE_FF00u64.to_be_bytes());
        });
        let expected = OrderExecutedMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
            executed_shares: 200,
            match_number: 0x99AA_BBCC_DDEE_FF00,
        };
        assert_eq!(parse_order_executed(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::OrderExecuted(expected))
        );
        assert_rejects(parse_order_executed, &bytes);
    }

    // ---- OrderExecutedPrice ----

    #[test]
    fn order_executed_price_parses() {
        let bytes = build(b'C', ORDER_EXECUTED_PRICE_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.extend_from_slice(&150u32.to_be_bytes());
            buf.extend_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
            buf.push(b'Y');
            buf.extend_from_slice(&1_430_000u32.to_be_bytes());
        });
        let expected = OrderExecutedPriceMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
            executed_shares: 150,
            match_number: 0x0102_0304_0506_0708,
            printable: true,
            execution_price: p4(1_430_000),
        };
        assert_eq!(parse_order_executed_price(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::OrderExecutedPrice(expected))
        );
        assert_rejects(parse_order_executed_price, &bytes);
    }

    // ---- OrderCancel ----

    #[test]
    fn order_cancel_parses() {
        let bytes = build(b'X', ORDER_CANCEL_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.extend_from_slice(&75u32.to_be_bytes());
        });
        let expected = OrderCancelMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
            cancelled_shares: 75,
        };
        assert_eq!(parse_order_cancel(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::OrderCancel(expected))
        );
        assert_rejects(parse_order_cancel, &bytes);
    }

    // ---- OrderDelete ----

    #[test]
    fn order_delete_parses() {
        let bytes = build(b'D', ORDER_DELETE_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
        });
        let expected = OrderDeleteMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
        };
        assert_eq!(parse_order_delete(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::OrderDelete(expected))
        );
        assert_rejects(parse_order_delete, &bytes);
    }

    // ---- OrderReplace ----

    #[test]
    fn order_replace_parses() {
        let bytes = build(b'U', ORDER_REPLACE_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.extend_from_slice(&0x8877_6655_4433_2211u64.to_be_bytes());
            buf.extend_from_slice(&900u32.to_be_bytes());
            buf.extend_from_slice(&1_500_000u32.to_be_bytes());
        });
        let expected = OrderReplaceMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            original_order_reference_number: 0x1122_3344_5566_7788,
            new_order_reference_number: 0x8877_6655_4433_2211,
            shares: 900,
            price: p4(1_500_000),
        };
        assert_eq!(parse_order_replace(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::OrderReplace(expected))
        );
        assert_rejects(parse_order_replace, &bytes);
    }

    // ---- Trade ----

    #[test]
    fn trade_parses() {
        let bytes = build(b'P', TRADE_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
            buf.push(b'S');
            buf.extend_from_slice(&100u32.to_be_bytes());
            buf.extend_from_slice(b"TSLA    ");
            buf.extend_from_slice(&2_500_000u32.to_be_bytes());
            buf.extend_from_slice(&0xDEAD_BEEF_CAFE_1234u64.to_be_bytes());
        });
        let expected = TradeMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            order_reference_number: 0x1122_3344_5566_7788,
            buy_sell_indicator: BuySell::Sell,
            shares: 100,
            stock: sym(b"TSLA    "),
            price: p4(2_500_000),
            match_number: 0xDEAD_BEEF_CAFE_1234,
        };
        assert_eq!(parse_trade(&bytes), Ok(expected));
        assert_eq!(parse_message(&bytes), Ok(ParsedMessage::Trade(expected)));
        assert_rejects(parse_trade, &bytes);
    }

    // ---- CrossTrade ----

    #[test]
    fn cross_trade_parses_with_an_8_byte_share_count() {
        let bytes = build(b'Q', CROSS_TRADE_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x0000_0001_0000_0000u64.to_be_bytes());
            buf.extend_from_slice(b"AAPL    ");
            buf.extend_from_slice(&1_000_000u32.to_be_bytes());
            buf.extend_from_slice(&0x0F1E_2D3C_4B5A_6978u64.to_be_bytes());
            buf.push(b'O');
        });
        let expected = CrossTradeMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            shares: 0x0000_0001_0000_0000,
            stock: sym(b"AAPL    "),
            cross_price: p4(1_000_000),
            match_number: 0x0F1E_2D3C_4B5A_6978,
            cross_type: CrossType::Opening,
        };
        assert_eq!(parse_cross_trade(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::CrossTrade(expected))
        );
        assert_rejects(parse_cross_trade, &bytes);
    }

    // ---- BrokenTrade ----

    #[test]
    fn broken_trade_parses() {
        let bytes = build(b'B', BROKEN_TRADE_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&0x0F1E_2D3C_4B5A_6978u64.to_be_bytes());
        });
        let expected = BrokenTradeMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            match_number: 0x0F1E_2D3C_4B5A_6978,
        };
        assert_eq!(parse_broken_trade(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::BrokenTrade(expected))
        );
        assert_rejects(parse_broken_trade, &bytes);
    }

    // ---- NOII ----

    #[test]
    fn noii_parses() {
        let bytes = build(b'I', NOII_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(&10_000u64.to_be_bytes());
            buf.extend_from_slice(&500u64.to_be_bytes());
            buf.push(b'B');
            buf.extend_from_slice(b"IBM     ");
            buf.extend_from_slice(&101_000u32.to_be_bytes());
            buf.extend_from_slice(&100_500u32.to_be_bytes());
            buf.extend_from_slice(&100_000u32.to_be_bytes());
            buf.push(b'O');
            buf.push(b'L');
        });
        let expected = NOIIMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            paired_shares: 10_000,
            imbalance_shares: 500,
            imbalance_direction: ImbalanceDirection::Buy,
            stock: sym(b"IBM     "),
            far_price: p4(101_000),
            near_price: p4(100_500),
            current_reference_price: p4(100_000),
            cross_type: CrossType::Opening,
            price_variation_indicator: PriceVariationIndicator::LessThanOnePercent,
        };
        assert_eq!(parse_noii(&bytes), Ok(expected));
        assert_eq!(parse_message(&bytes), Ok(ParsedMessage::NOII(expected)));
        assert_rejects(parse_noii, &bytes);
    }

    // ---- RPII ----

    #[test]
    fn rpii_parses() {
        let bytes = build(b'N', RPII_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"SPY     ");
            buf.push(b'B');
        });
        let expected = RPIIMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"SPY     "),
            interest_flag: InterestFlag::Bid,
        };
        assert_eq!(parse_rpii(&bytes), Ok(expected));
        assert_eq!(parse_message(&bytes), Ok(ParsedMessage::RPII(expected)));
        assert_rejects(parse_rpii, &bytes);
    }

    // ---- DLCR ----

    #[test]
    fn dlcr_uses_a_full_8_byte_near_execution_time() {
        let near_execution_time = 0x1122_3344_5566_7788u64;
        let bytes = build(b'O', DLCR_PRICE_DISCOVERY_MESSAGE_LEN, |buf| {
            push_header(buf, 0x1111, 0x2222, TS);
            buf.extend_from_slice(b"RIVN    ");
            buf.push(b'Y');
            buf.extend_from_slice(&90_000u32.to_be_bytes());
            buf.extend_from_slice(&110_000u32.to_be_bytes());
            buf.extend_from_slice(&100_000u32.to_be_bytes());
            buf.extend_from_slice(&near_execution_time.to_be_bytes());
            buf.extend_from_slice(&95_000u32.to_be_bytes());
            buf.extend_from_slice(&105_000u32.to_be_bytes());
        });
        let expected = DLCRPriceDiscoveryMessage {
            stock_locate: 0x1111,
            tracking_number: 0x2222,
            timestamp: TS,
            stock: sym(b"RIVN    "),
            open_eligibility_status: OpenEligibilityStatus::Eligible,
            minimum_allowable_price: p4(90_000),
            maximum_allowable_price: p4(110_000),
            near_execution_price: p4(100_000),
            near_execution_time,
            lower_price_range_collar: p4(95_000),
            upper_price_range_collar: p4(105_000),
        };
        assert_eq!(parse_dlcr_price_discovery(&bytes), Ok(expected));
        assert_eq!(
            parse_message(&bytes),
            Ok(ParsedMessage::DLCRPriceDiscovery(expected))
        );
        assert_rejects(parse_dlcr_price_discovery, &bytes);
    }
}
// </Generated by Claude.ai>
