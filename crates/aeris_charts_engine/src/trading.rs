//! First-class, broker-neutral trading objects.
//!
//! The host owns authoritative broker state. This module owns only chart-local semantic state,
//! validation, geometry inputs, and dedicated hit identities. Live trading state is deliberately
//! absent from drawing persistence.

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{
    ChartEngine, ChartError, DrawingId, DrawingKind, DrawingPriceScale, ErrorCode, HitProfile,
    PriceScaleTarget, PANELESS,
};
use aeris_charts_core::style::{
    DEFAULT_PRIMARY_RGB, MARKET_DOWN_RGB, MARKET_UP_RGB, MARKET_WARNING_RGB,
};
use aeris_charts_render::color::Color;

pub const MAX_TRADING_OBJECTS: usize = 4_096;
const MAX_TRADING_ID_BYTES: usize = 128;
pub const MAX_TRADING_ANNOTATIONS: usize = 8;
pub const MAX_TRADING_ROUND_TRIPS: usize = 4_096;
pub const MAX_HOST_EVENTS: usize = 2_048;
pub const MAX_HOST_WINDOWS: usize = 512;
const MAX_TRADING_ANNOTATION_TEXT_BYTES: usize = 96;
const MAX_TRADING_ANNOTATION_TOOLTIP_BYTES: usize = 256;

macro_rules! trading_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ChartError> {
                let value = value.into();
                validate_id(stringify!($name), &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            fn heap_bytes(&self) -> usize {
                self.0.capacity()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

trading_id!(PositionId);
trading_id!(OrderId);
trading_id!(ExecutionId);
trading_id!(TradingGroupId);
trading_id!(AccountId);

fn validate_id(kind: &str, value: &str) -> Result<(), ChartError> {
    if value.is_empty() || value.len() > MAX_TRADING_ID_BYTES {
        return Err(ChartError::new(
            ErrorCode::InvalidData,
            format!("{kind} must contain 1..={MAX_TRADING_ID_BYTES} UTF-8 bytes"),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
    Buy,
    Sell,
}

fn opposite_order_side(side: OrderSide) -> OrderSide {
    match side {
        OrderSide::Buy => OrderSide::Sell,
        OrderSide::Sell => OrderSide::Buy,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    Market,
    Limit,
    Stop,
    StopLimit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderRole {
    #[default]
    Working,
    StopLoss,
    TakeProfit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    PendingSubmit,
    Working,
    PendingModify,
    PartiallyFilled,
    Filled,
    PendingCancel,
    Cancelled,
    Rejected,
    Expired,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingAnnotationTone {
    #[default]
    Neutral,
    Info,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingAnnotationPlacement {
    #[default]
    Inline,
    Above,
    Below,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingAnnotation {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub tone: TradingAnnotationTone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(default)]
    pub placement: TradingAnnotationPlacement,
}

impl TradingAnnotation {
    fn validate(&self) -> Result<(), ChartError> {
        validate_id("TradingAnnotation.id", &self.id)?;
        if self.text.is_empty() || self.text.len() > MAX_TRADING_ANNOTATION_TEXT_BYTES {
            return Err(invalid(format!(
                "trading annotation text must contain 1..={MAX_TRADING_ANNOTATION_TEXT_BYTES} UTF-8 bytes"
            )));
        }
        if self
            .tooltip
            .as_ref()
            .is_some_and(|tooltip| tooltip.len() > MAX_TRADING_ANNOTATION_TOOLTIP_BYTES)
        {
            return Err(invalid(format!(
                "trading annotation tooltip must be at most {MAX_TRADING_ANNOTATION_TOOLTIP_BYTES} UTF-8 bytes"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingPriceScale {
    #[default]
    Right,
    Left,
    Overlay,
}

impl From<TradingPriceScale> for PriceScaleTarget {
    fn from(value: TradingPriceScale) -> Self {
        match value {
            TradingPriceScale::Right => Self::Right,
            TradingPriceScale::Left => Self::Left,
            TradingPriceScale::Overlay => Self::Overlay,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InstrumentMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_precision: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_precision: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_quantity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub point_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingPosition {
    pub id: PositionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
    #[serde(default)]
    pub pane_index: usize,
    #[serde(default)]
    pub price_scale: TradingPriceScale,
    pub side: PositionSide,
    pub average_price: f64,
    pub quantity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_pnl: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<TradingAnnotation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkingOrder {
    pub id: OrderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
    #[serde(default)]
    pub pane_index: usize,
    #[serde(default)]
    pub price_scale: TradingPriceScale,
    pub side: OrderSide,
    pub kind: OrderKind,
    #[serde(default)]
    pub role: OrderRole,
    pub status: OrderStatus,
    pub price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<f64>,
    /// Host-computed trailing trigger shown without moving the broker order line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trailing_trigger_price: Option<f64>,
    /// Host-computed break-even trigger shown without moving the broker order line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_even_trigger_price: Option<f64>,
    pub quantity: f64,
    #[serde(default)]
    pub filled_quantity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<PositionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_order_id: Option<OrderId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bracket_id: Option<TradingGroupId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oco_group_id: Option<TradingGroupId>,
    #[serde(default)]
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<TradingAnnotation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionKind {
    Entry,
    PartialFill,
    Exit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMarkerShape {
    Circle,
    #[default]
    Arrow,
    Triangle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingRoundTripOutcome {
    Profit,
    Loss,
    Flat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingRoundTrip {
    pub id: TradingGroupId,
    pub entry_execution_id: ExecutionId,
    pub exit_execution_id: ExecutionId,
    pub result_label: String,
    pub outcome: TradingRoundTripOutcome,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostEventMarker {
    pub id: String,
    pub time: i64,
    pub importance: u8,
    pub label: String,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostTimeWindow {
    pub id: String,
    pub start_time: i64,
    pub end_time: i64,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HostOverlaySnapshot {
    pub events: Vec<HostEventMarker>,
    pub windows: Vec<HostTimeWindow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEventHit {
    pub id: String,
    pub window: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingExecution {
    pub id: ExecutionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
    #[serde(default)]
    pub pane_index: usize,
    #[serde(default)]
    pub price_scale: TradingPriceScale,
    pub side: OrderSide,
    pub kind: ExecutionKind,
    pub time: i64,
    pub price: f64,
    pub quantity: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_id: Option<OrderId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_id: Option<PositionId>,
    #[serde(default)]
    pub marker_shape: ExecutionMarkerShape,
    #[serde(default)]
    pub size_by_quantity: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TradingSnapshot {
    pub instrument: InstrumentMetadata,
    pub positions: Vec<TradingPosition>,
    pub orders: Vec<WorkingOrder>,
    pub executions: Vec<TradingExecution>,
    #[serde(default)]
    pub round_trips: Vec<TradingRoundTrip>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradingStyle {
    pub position: Color,
    pub working_order: Color,
    pub buy: Color,
    pub sell: Color,
    pub profit: Color,
    pub risk: Color,
    pub take_profit: Color,
    pub stop_loss: Color,
    pub pending: Color,
    pub rejected: Color,
    pub control: Color,
    pub label: Color,
    /// Execution arrows use their own blue/red pair, distinct from the green/red order chrome,
    /// so a past fill never reads as a live order.
    pub execution_buy: Color,
    pub execution_sell: Color,
}

impl Default for TradingStyle {
    fn default() -> Self {
        let primary = Color::rgb(
            DEFAULT_PRIMARY_RGB.0,
            DEFAULT_PRIMARY_RGB.1,
            DEFAULT_PRIMARY_RGB.2,
        );
        let market_up = Color::rgb(MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2);
        let sell = Color::rgb(MARKET_DOWN_RGB.0, MARKET_DOWN_RGB.1, MARKET_DOWN_RGB.2);
        let warning = Color::rgb(
            MARKET_WARNING_RGB.0,
            MARKET_WARNING_RGB.1,
            MARKET_WARNING_RGB.2,
        );
        Self {
            position: primary,
            working_order: primary,
            // Order and position chrome reads by direction, the way a trading terminal does:
            // buy/long shares the up-bar green, sell/short the down-bar red. `primary` stays the
            // neutral accent for connectors and group chrome, which carry no direction.
            buy: market_up,
            sell,
            profit: market_up,
            risk: sell,
            take_profit: market_up,
            stop_loss: warning,
            pending: warning,
            rejected: Color::rgb(0x78, 0x7b, 0x86),
            control: primary,
            label: Color::rgb(0xff, 0xff, 0xff),
            execution_buy: Color::rgb(0x29, 0x62, 0xff),
            execution_sell: Color::rgb(0xf2, 0x36, 0x45),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TradingStyleOptions {
    pub position: Option<String>,
    pub working_order: Option<String>,
    pub buy: Option<String>,
    pub sell: Option<String>,
    pub profit: Option<String>,
    pub risk: Option<String>,
    pub take_profit: Option<String>,
    pub stop_loss: Option<String>,
    pub pending: Option<String>,
    pub rejected: Option<String>,
    pub control: Option<String>,
    pub label: Option<String>,
    pub execution_buy: Option<String>,
    pub execution_sell: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingIntentAction {
    PlaceBracketOrder,
    ModifyOrder,
    CancelOrder,
    CreateStopLoss,
    CreateTakeProfit,
    ClosePosition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TradingIntent {
    pub sequence: u32,
    pub action: TradingIntentAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<AccountId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drawing_id: Option<DrawingId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<OrderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_id: Option<PositionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_scale: Option<TradingPriceScale>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<OrderSide>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<OrderKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<OrderRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    /// Exact price index in the instrument tick lattice when tick metadata is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_tick_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bracket_id: Option<TradingGroupId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oco_group_id: Option<TradingGroupId>,
    pub base_revision: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum TradingPreviewSource {
    Order { order_id: OrderId },
    StopLoss { position_id: PositionId },
    TakeProfit { position_id: PositionId },
    OrderStopLoss { order_id: OrderId },
    OrderTakeProfit { order_id: OrderId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TradingPreview {
    #[serde(flatten)]
    pub source: TradingPreviewSource,
    pub pane_index: usize,
    pub price_scale: TradingPriceScale,
    pub price: f64,
    pub quantity: f64,
    pub side: OrderSide,
    pub role: OrderRole,
    pub base_revision: u32,
}

const MAX_PENDING_TRADING_INTENTS: usize = 256;

#[derive(Clone, Debug, Default)]
pub(crate) enum TradingInteractionState {
    #[default]
    Idle,
    Hovering {
        hit: TradingHit,
    },
    DraggingOrder {
        authoritative_price: f64,
        preview: TradingPreview,
    },
    /// The chart already applied the change and emitted the intent. It keeps only what it needs
    /// to put things back if the host rejects — there is no shadow copy of the object and no
    /// pending chrome, because closing means the object is gone and moving means it has moved.
    PendingHostAck {
        sequence: u32,
        rollback: TradingRollback,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum TradingRollback {
    CreatedBracketOrder,
    CreatedProtection,
    RemovedOrder {
        index: usize,
        order: Box<WorkingOrder>,
    },
    RemovedPosition {
        index: usize,
        position: Box<TradingPosition>,
    },
    MovedOrder {
        id: OrderId,
        price: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TradingGroupKey {
    Bracket(TradingGroupId),
    Position(PositionId),
    ParentOrder(OrderId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum TradingGroupVisualState {
    #[default]
    Inactive,
    Active(TradingGroupKey),
}

impl TradingInteractionState {
    pub(crate) fn preview(&self) -> Option<&TradingPreview> {
        match self {
            Self::DraggingOrder { preview, .. } => Some(preview),
            Self::Idle | Self::Hovering { .. } | Self::PendingHostAck { .. } => None,
        }
    }

    fn dragging_preview_mut(&mut self) -> Option<&mut TradingPreview> {
        match self {
            Self::DraggingOrder { preview, .. } => Some(preview),
            _ => None,
        }
    }

    pub(crate) fn hover(&self) -> Option<&TradingHit> {
        match self {
            Self::Hovering { hit } => Some(hit),
            _ => None,
        }
    }

    fn is_idle_or_hovering(&self) -> bool {
        matches!(self, Self::Idle | Self::Hovering { .. })
    }

    fn references_position(&self, id: &PositionId) -> bool {
        self.hover().is_some_and(
            |hit| matches!(&hit.object, TradingObjectId::Position(position_id) if position_id == id),
        ) || self.preview().is_some_and(|preview| {
            matches!(
                &preview.source,
                TradingPreviewSource::StopLoss { position_id }
                    | TradingPreviewSource::TakeProfit { position_id }
                    if position_id == id
            )
        })
    }

    fn references_order(&self, id: &OrderId) -> bool {
        self.hover().is_some_and(
            |hit| matches!(&hit.object, TradingObjectId::Order(order_id) if order_id == id),
        ) || self.preview().is_some_and(|preview| {
            matches!(
                &preview.source,
                TradingPreviewSource::Order { order_id }
                    | TradingPreviewSource::OrderStopLoss { order_id }
                    | TradingPreviewSource::OrderTakeProfit { order_id }
                    if order_id == id
            )
        })
    }

    fn references_execution(&self, id: &ExecutionId) -> bool {
        self.hover().is_some_and(
            |hit| matches!(&hit.object, TradingObjectId::Execution(execution_id) if execution_id == id),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TradingState {
    pub instrument: InstrumentMetadata,
    pub visible_account_id: Option<AccountId>,
    pub positions: Vec<TradingPosition>,
    pub orders: Vec<WorkingOrder>,
    pub executions: Vec<TradingExecution>,
    pub round_trips: Vec<TradingRoundTrip>,
    pub host_overlay: HostOverlaySnapshot,
    pub style: TradingStyle,
    pub interaction: TradingInteractionState,
    pub feedback_hover: Option<TradingHit>,
    pub feedback_pressed: Option<TradingHit>,
    /// Set by the host once its hover dwell elapses. Action tooltips stay hidden until then, so
    /// sweeping the pointer across a row of markers never flashes a tooltip per marker.
    pub tooltip_armed: bool,
    pub group_visual: TradingGroupVisualState,
    intents: VecDeque<TradingIntent>,
    next_intent_sequence: u32,
}

impl TradingState {
    pub(crate) fn snapshot(&self) -> TradingSnapshot {
        TradingSnapshot {
            instrument: self.instrument.clone(),
            positions: self.positions.clone(),
            orders: self.orders.clone(),
            executions: self.executions.clone(),
            round_trips: self.round_trips.clone(),
        }
    }

    pub(crate) fn account_visible(&self, account_id: Option<&AccountId>) -> bool {
        self.visible_account_id
            .as_ref()
            .is_none_or(|visible| account_id == Some(visible))
    }

    fn next_sequence(&mut self) -> u32 {
        self.next_intent_sequence = self.next_intent_sequence.wrapping_add(1).max(1);
        self.next_intent_sequence
    }

    fn push_intent(&mut self, intent: TradingIntent) {
        if self.intents.len() == MAX_PENDING_TRADING_INTENTS {
            self.intents.pop_front();
        }
        self.intents.push_back(intent);
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        let retained_strings = self
            .positions
            .iter()
            .map(|value| {
                value.id.heap_bytes() + value.currency.as_ref().map_or(0, |value| value.capacity())
            })
            .sum::<usize>()
            + self
                .orders
                .iter()
                .map(|value| {
                    value.id.heap_bytes()
                        + value.position_id.as_ref().map_or(0, PositionId::heap_bytes)
                        + value
                            .parent_order_id
                            .as_ref()
                            .map_or(0, OrderId::heap_bytes)
                        + value
                            .bracket_id
                            .as_ref()
                            .map_or(0, TradingGroupId::heap_bytes)
                        + value
                            .oco_group_id
                            .as_ref()
                            .map_or(0, TradingGroupId::heap_bytes)
                })
                .sum::<usize>()
            + self
                .executions
                .iter()
                .map(|value| {
                    value.id.heap_bytes()
                        + value.order_id.as_ref().map_or(0, OrderId::heap_bytes)
                        + value.position_id.as_ref().map_or(0, PositionId::heap_bytes)
                })
                .sum::<usize>()
            + self
                .instrument
                .currency
                .as_ref()
                .map_or(0, |value| value.capacity());
        let visible_account_bytes = self
            .visible_account_id
            .as_ref()
            .map_or(0, AccountId::heap_bytes);
        self.positions.capacity() * std::mem::size_of::<TradingPosition>()
            + self.orders.capacity() * std::mem::size_of::<WorkingOrder>()
            + self.executions.capacity() * std::mem::size_of::<TradingExecution>()
            + self.intents.capacity() * std::mem::size_of::<TradingIntent>()
            + self
                .feedback_hover
                .as_ref()
                .map_or(0, TradingHit::heap_bytes)
            + self
                .feedback_pressed
                .as_ref()
                .map_or(0, TradingHit::heap_bytes)
            + retained_strings
            + visible_account_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradingObjectId {
    Position(PositionId),
    Order(OrderId),
    Execution(ExecutionId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradingHitKind {
    PositionLine,
    OrderLine,
    TakeProfitButton,
    StopLossButton,
    CancelButton,
    ExecutionMarker,
    Annotation,
}

/// Pointer affordance for a trading object: `Grab` over a draggable line, `Pointer` over a
/// close control or TP/SL protection button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TradingCursor {
    Grab,
    Pointer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradingHit {
    pub object: TradingObjectId,
    pub kind: TradingHitKind,
    pub distance: f64,
    pub annotation_id: Option<String>,
}

impl TradingHit {
    fn heap_bytes(&self) -> usize {
        match &self.object {
            TradingObjectId::Position(id) => id.heap_bytes(),
            TradingObjectId::Order(id) => id.heap_bytes(),
            TradingObjectId::Execution(id) => id.heap_bytes(),
        }
    }
}

fn invalid(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidData, message)
}

fn validate_instrument(instrument: &InstrumentMetadata) -> Result<(), ChartError> {
    for (name, value) in [
        ("tick_size", instrument.tick_size),
        ("minimum_quantity", instrument.minimum_quantity),
        ("point_value", instrument.point_value),
    ] {
        if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
            return Err(invalid(format!(
                "instrument {name} must be finite and positive"
            )));
        }
    }
    if instrument.price_precision.is_some_and(|value| value > 15)
        || instrument
            .quantity_precision
            .is_some_and(|value| value > 15)
    {
        return Err(invalid("instrument precision must be in 0..=15"));
    }
    if instrument
        .currency
        .as_ref()
        .is_some_and(|value| value.len() > 16)
    {
        return Err(invalid("instrument currency is too long"));
    }
    Ok(())
}

fn parse_style_color(name: &str, value: Option<&str>) -> Result<Option<Color>, ChartError> {
    value
        .map(|value| {
            Color::parse_css(value).ok_or_else(|| {
                invalid(format!("trading style {name} is not a supported CSS color"))
            })
        })
        .transpose()
}

fn validate_position(position: &TradingPosition) -> Result<(), ChartError> {
    validate_id("PositionId", position.id.as_str())?;
    if let Some(account_id) = &position.account_id {
        validate_id("AccountId", account_id.as_str())?;
    }
    if !position.average_price.is_finite()
        || !position.quantity.is_finite()
        || position.quantity <= 0.0
    {
        return Err(invalid(
            "position price must be finite and quantity positive",
        ));
    }
    if position.display_pnl.is_some_and(|value| !value.is_finite()) {
        return Err(invalid("position display_pnl must be finite"));
    }
    validate_annotations(&position.annotations)?;
    Ok(())
}

fn validate_order(order: &WorkingOrder) -> Result<(), ChartError> {
    validate_id("OrderId", order.id.as_str())?;
    if let Some(account_id) = &order.account_id {
        validate_id("AccountId", account_id.as_str())?;
    }
    if !order.price.is_finite()
        || order.stop_price.is_some_and(|value| !value.is_finite())
        || order
            .trailing_trigger_price
            .is_some_and(|value| !value.is_finite())
        || order
            .break_even_trigger_price
            .is_some_and(|value| !value.is_finite())
        || !order.quantity.is_finite()
        || order.quantity <= 0.0
        || !order.filled_quantity.is_finite()
        || order.filled_quantity < 0.0
        || order.filled_quantity > order.quantity
    {
        return Err(invalid("order prices and quantities are invalid"));
    }
    validate_annotations(&order.annotations)?;
    Ok(())
}

fn validate_annotations(annotations: &[TradingAnnotation]) -> Result<(), ChartError> {
    if annotations.len() > MAX_TRADING_ANNOTATIONS {
        return Err(invalid(format!(
            "trading annotations are capped at {MAX_TRADING_ANNOTATIONS} per object"
        )));
    }
    validate_unique(
        annotations.iter().map(|annotation| annotation.id.as_str()),
        "annotation",
    )?;
    for annotation in annotations {
        annotation.validate()?;
    }
    Ok(())
}

fn validate_execution(execution: &TradingExecution) -> Result<(), ChartError> {
    validate_id("ExecutionId", execution.id.as_str())?;
    if let Some(account_id) = &execution.account_id {
        validate_id("AccountId", account_id.as_str())?;
    }
    if !execution.price.is_finite() || !execution.quantity.is_finite() || execution.quantity <= 0.0
    {
        return Err(invalid(
            "execution price must be finite and quantity positive",
        ));
    }
    aeris_charts_core::model::data_validation::validate_timestamp(execution.time as f64)
        .map_err(|error| invalid(format!("execution time is invalid: {error}")))?;
    Ok(())
}

fn validate_round_trip(round_trip: &TradingRoundTrip) -> Result<(), ChartError> {
    validate_id("TradingGroupId", round_trip.id.as_str())?;
    validate_id("ExecutionId", round_trip.entry_execution_id.as_str())?;
    validate_id("ExecutionId", round_trip.exit_execution_id.as_str())?;
    if round_trip.result_label.len() > 128 {
        return Err(invalid("round-trip result label is too long"));
    }
    Ok(())
}

fn validate_host_overlay(overlay: &HostOverlaySnapshot) -> Result<(), ChartError> {
    if overlay.events.len() > MAX_HOST_EVENTS || overlay.windows.len() > MAX_HOST_WINDOWS {
        return Err(ChartError::new(
            ErrorCode::ResourceLimit,
            format!("host overlay exceeds {MAX_HOST_EVENTS} events or {MAX_HOST_WINDOWS} windows"),
        ));
    }
    for event in &overlay.events {
        validate_id("HostEventMarker.id", &event.id)?;
        if event.label.len() > 96 || event.icon.as_ref().is_some_and(|icon| icon.len() > 64) {
            return Err(invalid("host event label or icon is too long"));
        }
    }
    for window in &overlay.windows {
        validate_id("HostTimeWindow.id", &window.id)?;
        if window.end_time <= window.start_time || window.label.len() > 96 {
            return Err(invalid("host time window range or label is invalid"));
        }
    }
    validate_unique(
        overlay.events.iter().map(|value| value.id.as_str()),
        "host event",
    )?;
    validate_unique(
        overlay.windows.iter().map(|value| value.id.as_str()),
        "host window",
    )?;
    Ok(())
}

fn validate_unique<'a>(
    values: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<(), ChartError> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(invalid(format!("duplicate {kind} id '{value}'")));
        }
    }
    Ok(())
}

impl ChartEngine {
    pub fn trading_hit_at(&self, x_css: f64, y_css: f64) -> Option<TradingHit> {
        self.trading_hit_at_with_profile(x_css, y_css, HitProfile::PRECISION)
    }

    pub fn trading_hit_at_with_profile(
        &self,
        x_css: f64,
        y_css: f64,
        profile: HitProfile,
    ) -> Option<TradingHit> {
        if !x_css.is_finite() || !y_css.is_finite() || x_css < 0.0 || x_css > self.pane_w {
            return None;
        }
        let pane_index = self.pane_at_y(y_css)?;
        let line_tolerance = profile.trading_line_tolerance;

        for order in self.trading_state.orders.iter().rev() {
            if !self
                .trading_state
                .account_visible(order.account_id.as_ref())
            {
                continue;
            }
            if order.pane_index != pane_index {
                continue;
            }
            let Some(y) = self.trading_price_coordinate(
                pane_index,
                order.price_scale,
                self.trading_effective_order_price(order),
            ) else {
                continue;
            };
            let distance = (y_css - y).abs();
            if let Some(annotation_id) =
                self.trading_annotation_hit(&order.annotations, y, x_css, y_css)
            {
                return Some(TradingHit {
                    object: TradingObjectId::Order(order.id.clone()),
                    kind: TradingHitKind::Annotation,
                    distance: (y_css - y).abs(),
                    annotation_id: Some(annotation_id),
                });
            }
            if distance > line_tolerance.max(self.trading_control_height() / 2.0) {
                continue;
            }
            let kind = self
                .trading_order_protection_hit(order, x_css)
                .unwrap_or_else(|| self.trading_order_chip_hit(order, x_css));
            // The control cluster is a chip, not a hairline: over it the marker answers across the
            // chip's full height (and the device's control box, for touch), not the line tolerance.
            let tolerance = if matches!(
                kind,
                TradingHitKind::CancelButton
                    | TradingHitKind::TakeProfitButton
                    | TradingHitKind::StopLossButton
            ) {
                line_tolerance.max(self.trading_control_height() / 2.0)
            } else {
                line_tolerance
            };
            if distance > tolerance {
                continue;
            }
            return Some(TradingHit {
                object: TradingObjectId::Order(order.id.clone()),
                kind,
                distance,
                annotation_id: None,
            });
        }

        for position in self.trading_state.positions.iter().rev() {
            if !self
                .trading_state
                .account_visible(position.account_id.as_ref())
            {
                continue;
            }
            if position.pane_index != pane_index {
                continue;
            }
            let Some(y) = self.trading_price_coordinate(
                pane_index,
                position.price_scale,
                position.average_price,
            ) else {
                continue;
            };
            let distance = (y_css - y).abs();
            if let Some(annotation_id) =
                self.trading_annotation_hit(&position.annotations, y, x_css, y_css)
            {
                return Some(TradingHit {
                    object: TradingObjectId::Position(position.id.clone()),
                    kind: TradingHitKind::Annotation,
                    distance: (y_css - y).abs(),
                    annotation_id: Some(annotation_id),
                });
            }
            if distance > line_tolerance.max(self.trading_control_height() / 2.0) {
                continue;
            }
            let kind = self
                .trading_position_protection_hit(position, x_css)
                .unwrap_or_else(|| self.trading_position_chip_hit(position, x_css));
            let tolerance = if matches!(
                kind,
                TradingHitKind::CancelButton
                    | TradingHitKind::TakeProfitButton
                    | TradingHitKind::StopLossButton
            ) {
                line_tolerance.max(self.trading_control_height() / 2.0)
            } else {
                line_tolerance
            };
            if distance > tolerance {
                continue;
            }
            return Some(TradingHit {
                object: TradingObjectId::Position(position.id.clone()),
                kind,
                distance,
                annotation_id: None,
            });
        }

        // Hits resolve against the exact marks the frame draws; the topmost (last drawn) wins and
        // answers with the bar's latest fill on that side.
        let layout = self.trading_execution_layout(pane_index);
        for mark in layout.marks.iter().rev() {
            let half_w = (mark.size / 2.0).max(profile.control_half_size);
            let half_h = (mark.height / 2.0).max(profile.control_half_size);
            if (x_css - mark.x).abs() > half_w || (y_css - mark.y).abs() > half_h {
                continue;
            }
            let execution = &self.trading_state.executions[layout.order[mark.fills.end - 1]];
            return Some(TradingHit {
                object: TradingObjectId::Execution(execution.id.clone()),
                kind: TradingHitKind::ExecutionMarker,
                distance: (x_css - mark.x).hypot(y_css - mark.y),
                annotation_id: None,
            });
        }
        None
    }

    /// Pointer affordance for the trading object under the cursor, shared by every host so a
    /// line reads as draggable exactly when a drag would start there.
    pub fn trading_cursor_at(&self, x_css: f64, y_css: f64) -> Option<TradingCursor> {
        let hit = self.trading_hit_at(x_css, y_css)?;
        match (&hit.object, hit.kind) {
            // An execution arrow is clickable detail: it reveals the exact fill on its bar.
            (_, TradingHitKind::CancelButton | TradingHitKind::ExecutionMarker) => {
                Some(TradingCursor::Pointer)
            }
            // A protection button reads as a button: it answers with the click affordance even
            // though pressing and dragging it also places the protection price.
            (TradingObjectId::Order(id), TradingHitKind::TakeProfitButton) => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == id)
                .and_then(|order| {
                    self.trading_order_protection_preview(order, OrderRole::TakeProfit)
                })
                .map(|_| TradingCursor::Pointer),
            (TradingObjectId::Order(id), TradingHitKind::StopLossButton) => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == id)
                .and_then(|order| self.trading_order_protection_preview(order, OrderRole::StopLoss))
                .map(|_| TradingCursor::Pointer),
            (TradingObjectId::Position(id), TradingHitKind::TakeProfitButton) => self
                .trading_state
                .positions
                .iter()
                .find(|position| &position.id == id)
                .and_then(|position| {
                    self.trading_position_protection_preview(position, OrderRole::TakeProfit)
                })
                .map(|_| TradingCursor::Pointer),
            (TradingObjectId::Position(id), TradingHitKind::StopLossButton) => self
                .trading_state
                .positions
                .iter()
                .find(|position| &position.id == id)
                .and_then(|position| {
                    self.trading_position_protection_preview(position, OrderRole::StopLoss)
                })
                .map(|_| TradingCursor::Pointer),
            (TradingObjectId::Order(id), TradingHitKind::OrderLine) => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == id)
                .and_then(|order| self.trading_order_drag_preview(order))
                .map(|_| TradingCursor::Grab),
            _ => None,
        }
    }

    pub fn set_trading_hover(&mut self, x_css: f64, y_css: f64) -> bool {
        let next = self.trading_hit_at(x_css, y_css);
        let feedback_changed = self.trading_state.feedback_hover.as_ref() != next.as_ref();
        let interaction_changed = self.trading_state.interaction.is_idle_or_hovering()
            && self.trading_state.interaction.hover() != next.as_ref();
        if !feedback_changed && !interaction_changed {
            return false;
        }
        self.trading_state.feedback_hover = next.clone();
        // A new hover target restarts the dwell; the host arms the tooltip again when it elapses.
        self.trading_state.tooltip_armed = false;
        if interaction_changed {
            self.trading_state.interaction = next.map_or(TradingInteractionState::Idle, |hit| {
                TradingInteractionState::Hovering { hit }
            });
        }
        self.invalidate_frame_trading();
        self.invalidate_frame_overlay();
        true
    }

    /// Reveal the hovered control's action tooltip. The host calls this once its hover dwell
    /// elapses — the engine is headless and owns no timer of its own. Returns whether the frame
    /// changed, so the host can skip a repaint when the hover already moved on.
    pub fn arm_trading_tooltip(&mut self) -> bool {
        let hovering_control = self
            .trading_state
            .feedback_hover
            .as_ref()
            .is_some_and(|hit| hit.kind == TradingHitKind::CancelButton);
        if !hovering_control || self.trading_state.tooltip_armed {
            return false;
        }
        self.trading_state.tooltip_armed = true;
        self.invalidate_frame_trading();
        true
    }

    pub fn clear_trading_hover(&mut self) -> bool {
        let had_feedback = self.trading_state.feedback_hover.take().is_some();
        let had_interaction = matches!(
            self.trading_state.interaction,
            TradingInteractionState::Hovering { .. }
        );
        self.trading_state.tooltip_armed = false;
        if !had_feedback && !had_interaction {
            return false;
        }
        if had_interaction {
            self.trading_state.interaction = TradingInteractionState::Idle;
        }
        self.invalidate_frame_trading();
        self.invalidate_frame_overlay();
        true
    }

    pub fn set_trading_pressed(&mut self, x_css: f64, y_css: f64) -> bool {
        self.set_trading_pressed_with_profile(x_css, y_css, HitProfile::PRECISION)
    }

    pub fn set_trading_pressed_with_profile(
        &mut self,
        x_css: f64,
        y_css: f64,
        profile: HitProfile,
    ) -> bool {
        let next = self.trading_hit_at_with_profile(x_css, y_css, profile);
        if self.trading_state.feedback_pressed.as_ref() == next.as_ref() {
            return false;
        }
        self.trading_state.feedback_pressed = next;
        self.invalidate_frame_trading();
        true
    }

    pub fn clear_trading_pressed(&mut self) -> bool {
        if self.trading_state.feedback_pressed.take().is_none() {
            return false;
        }
        self.invalidate_frame_trading();
        true
    }

    fn snap_trading_price(&self, price: f64) -> f64 {
        let Some(tick) = self.trading_state.instrument.tick_size else {
            return price;
        };
        (price / tick).round() * tick
    }

    /// The price-band ladder that governs trading prices on `(pane, scale)`: the tick ladder of
    /// that scale's formatter-source series. When configured it is the single tick source for
    /// both the axis labels and order snapping; the scalar instrument `tick_size` applies only
    /// where no ladder is configured.
    fn trading_tick_ladder(
        &self,
        pane_index: usize,
        price_scale: TradingPriceScale,
    ) -> Option<&aeris_charts_core::format::price_tick_ladder::PriceTickLadder> {
        self.scale_formatter_source(pane_index, price_scale.into())?
            .price_format
            .active_tick_ladder()
    }

    /// Snap to the ladder band tick on `(pane, scale)`, else the instrument tick.
    fn snap_trading_price_on(
        &self,
        pane_index: usize,
        price_scale: TradingPriceScale,
        price: f64,
    ) -> f64 {
        match self.trading_tick_ladder(pane_index, price_scale) {
            Some(ladder) => ladder.snap(price),
            None => self.snap_trading_price(price),
        }
    }

    /// Exact tick index on `(pane, scale)`: cumulative band ticks with a ladder, else
    /// `price / tick_size`.
    fn trading_price_tick_index_on(
        &self,
        pane_index: usize,
        price_scale: TradingPriceScale,
        price: f64,
    ) -> Option<i64> {
        match self.trading_tick_ladder(pane_index, price_scale) {
            Some(ladder) => ladder.tick_index(price),
            None => self.trading_price_tick_index(price),
        }
    }

    /// Half the tick at `price` on `(pane, scale)`: the tolerance for "the price did not move".
    fn trading_half_tick_on(
        &self,
        pane_index: usize,
        price_scale: TradingPriceScale,
        price: f64,
    ) -> f64 {
        match self.trading_tick_ladder(pane_index, price_scale) {
            Some(ladder) => ladder.min_move_at(price) * 0.5,
            None => {
                self.trading_state
                    .instrument
                    .tick_size
                    .unwrap_or(f64::EPSILON)
                    * 0.5
            }
        }
    }

    fn trading_price_tick_index(&self, price: f64) -> Option<i64> {
        let tick = self.trading_state.instrument.tick_size?;
        if !tick.is_finite() || tick <= 0.0 || !price.is_finite() {
            return None;
        }
        let index = (price / tick).round();
        if !index.is_finite() || (price - index * tick).abs() > tick * 1.0e-9 {
            return None;
        }
        Some(index as i64)
    }

    fn trading_protection_quantity(&self, order: &WorkingOrder) -> f64 {
        order
            .position_id
            .as_ref()
            .and_then(|id| {
                self.trading_state
                    .positions
                    .iter()
                    .find(|position| &position.id == id)
                    .map(|position| position.quantity)
            })
            .unwrap_or_else(|| {
                let remaining = (order.quantity - order.filled_quantity).max(0.0);
                if remaining > 0.0 {
                    remaining
                } else {
                    order.quantity
                }
            })
    }

    fn trading_order_drag_preview(&self, order: &WorkingOrder) -> Option<TradingPreview> {
        if order.role == OrderRole::Working
            || !matches!(
                order.status,
                OrderStatus::Working | OrderStatus::PartiallyFilled
            )
        {
            return None;
        }
        Some(TradingPreview {
            source: TradingPreviewSource::Order {
                order_id: order.id.clone(),
            },
            pane_index: order.pane_index,
            price_scale: order.price_scale,
            price: order.price,
            quantity: (order.quantity - order.filled_quantity).max(0.0),
            side: order.side,
            role: order.role,
            base_revision: order.revision,
        })
    }

    fn trading_order_has_protection(&self, order: &WorkingOrder, role: OrderRole) -> bool {
        self.trading_state.orders.iter().any(|candidate| {
            candidate.role == role
                && candidate.id != order.id
                && (candidate.parent_order_id.as_ref() == Some(&order.id)
                    || order.position_id.is_some() && candidate.position_id == order.position_id
                    || order.bracket_id.is_some() && candidate.bracket_id == order.bracket_id)
        })
    }

    pub(crate) fn trading_order_protection_preview(
        &self,
        order: &WorkingOrder,
        role: OrderRole,
    ) -> Option<TradingPreview> {
        if order.role != OrderRole::Working
            || !matches!(
                order.status,
                OrderStatus::Working | OrderStatus::PartiallyFilled | OrderStatus::Filled
            )
            || !matches!(role, OrderRole::TakeProfit | OrderRole::StopLoss)
            || self.trading_order_has_protection(order, role)
        {
            return None;
        }
        let source = match role {
            OrderRole::TakeProfit => TradingPreviewSource::OrderTakeProfit {
                order_id: order.id.clone(),
            },
            OrderRole::StopLoss => TradingPreviewSource::OrderStopLoss {
                order_id: order.id.clone(),
            },
            OrderRole::Working => return None,
        };
        Some(TradingPreview {
            source,
            pane_index: order.pane_index,
            price_scale: order.price_scale,
            price: order.price,
            quantity: self.trading_protection_quantity(order),
            side: opposite_order_side(order.side),
            role,
            base_revision: order.revision,
        })
    }

    pub(crate) fn trading_position_protection_preview(
        &self,
        position: &TradingPosition,
        role: OrderRole,
    ) -> Option<TradingPreview> {
        if !position.quantity.is_finite() || position.quantity <= 0.0 {
            return None;
        }
        let attached = |role| {
            self.trading_state
                .orders
                .iter()
                .any(|order| order.position_id.as_ref() == Some(&position.id) && order.role == role)
        };
        if !matches!(role, OrderRole::TakeProfit | OrderRole::StopLoss) || attached(role) {
            return None;
        }
        let source = match role {
            OrderRole::TakeProfit => TradingPreviewSource::TakeProfit {
                position_id: position.id.clone(),
            },
            OrderRole::StopLoss => TradingPreviewSource::StopLoss {
                position_id: position.id.clone(),
            },
            OrderRole::Working => return None,
        };
        Some(TradingPreview {
            source,
            pane_index: position.pane_index,
            price_scale: position.price_scale,
            price: position.average_price,
            quantity: position.quantity,
            side: match position.side {
                PositionSide::Long => OrderSide::Sell,
                PositionSide::Short => OrderSide::Buy,
            },
            role,
            base_revision: 0,
        })
    }

    pub(crate) fn trading_preview_relation(&self, preview: &TradingPreview) -> Option<(f64, bool)> {
        let position_relation = |order: &WorkingOrder| {
            let position = self
                .trading_state
                .positions
                .iter()
                .find(|position| order.position_id.as_ref() == Some(&position.id))?;
            Some((position.average_price, position.side == PositionSide::Long))
        };
        let order_relation =
            |order: &WorkingOrder| Some((order.price, order.side == OrderSide::Buy));
        match &preview.source {
            TradingPreviewSource::Order { order_id } => {
                let order = self
                    .trading_state
                    .orders
                    .iter()
                    .find(|order| &order.id == order_id)?;
                position_relation(order).or_else(|| {
                    order.parent_order_id.as_ref().and_then(|parent_id| {
                        self.trading_state
                            .orders
                            .iter()
                            .find(|parent| &parent.id == parent_id)
                            .and_then(order_relation)
                    })
                })
            }
            TradingPreviewSource::StopLoss { position_id }
            | TradingPreviewSource::TakeProfit { position_id } => self
                .trading_state
                .positions
                .iter()
                .find(|position| &position.id == position_id)
                .map(|position| (position.average_price, position.side == PositionSide::Long)),
            TradingPreviewSource::OrderStopLoss { order_id }
            | TradingPreviewSource::OrderTakeProfit { order_id } => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == order_id)
                .and_then(order_relation),
        }
    }

    fn trading_preview_price_valid(&self, preview: &TradingPreview) -> bool {
        // A confirmed broker order keeps its semantic role when it crosses the entry. This is
        // important for terminal-style editing: moving an SL above the entry does not silently
        // turn it into a TP (and vice versa).
        if matches!(preview.source, TradingPreviewSource::Order { .. }) {
            return true;
        }
        let Some((anchor, long)) = self.trading_preview_relation(preview) else {
            // Existing broker-owned protection orders returned above and remain freely movable.
            // A new protection request must still have the entry relationship that defines its
            // initial TP/SL classification.
            return false;
        };
        match (long, preview.role) {
            (true, OrderRole::TakeProfit) => preview.price > anchor,
            (true, OrderRole::StopLoss) => preview.price < anchor,
            (false, OrderRole::TakeProfit) => preview.price < anchor,
            (false, OrderRole::StopLoss) => preview.price > anchor,
            (_, OrderRole::Working) => true,
        }
    }

    /// Commit a dragged protection line as a modification, or a dragged entry as a new protection
    /// request. Existing lines move immediately with a rollback; creation remains host-owned
    /// because the broker supplies the new order identity.
    fn commit_trading_preview(&mut self) -> Option<TradingIntent> {
        let preview = self.trading_state.interaction.preview()?.clone();
        if !matches!(
            self.trading_state.interaction,
            TradingInteractionState::DraggingOrder { .. }
        ) {
            return None;
        }
        let sequence = self.trading_state.next_sequence();
        let (action, order_id, position_id, kind, rollback, bracket_id, oco_group_id, stop_price) =
            match &preview.source {
                TradingPreviewSource::Order { order_id } => {
                    let order = self
                        .trading_state
                        .orders
                        .iter_mut()
                        .find(|order| &order.id == order_id)?;
                    let rollback = TradingRollback::MovedOrder {
                        id: order.id.clone(),
                        price: order.price,
                    };
                    order.price = preview.price;
                    (
                        TradingIntentAction::ModifyOrder,
                        Some(order.id.clone()),
                        None,
                        Some(order.kind),
                        rollback,
                        order.bracket_id.clone(),
                        order.oco_group_id.clone(),
                        order.stop_price,
                    )
                }
                TradingPreviewSource::OrderStopLoss { order_id }
                | TradingPreviewSource::OrderTakeProfit { order_id } => {
                    let order = self
                        .trading_state
                        .orders
                        .iter()
                        .find(|order| &order.id == order_id)?;
                    if self.trading_state.orders.iter().any(|child| {
                        child.parent_order_id.as_ref() == Some(order_id)
                            && child.role == preview.role
                    }) {
                        return None;
                    }
                    (
                        if preview.role == OrderRole::StopLoss {
                            TradingIntentAction::CreateStopLoss
                        } else {
                            TradingIntentAction::CreateTakeProfit
                        },
                        Some(order.id.clone()),
                        None,
                        Some(if preview.role == OrderRole::StopLoss {
                            OrderKind::Stop
                        } else {
                            OrderKind::Limit
                        }),
                        TradingRollback::CreatedProtection,
                        order.bracket_id.clone(),
                        order.oco_group_id.clone(),
                        None,
                    )
                }
                TradingPreviewSource::StopLoss { position_id }
                | TradingPreviewSource::TakeProfit { position_id } => {
                    if self.trading_state.orders.iter().any(|order| {
                        order.position_id.as_ref() == Some(position_id)
                            && order.role == preview.role
                    }) {
                        return None;
                    }
                    (
                        if preview.role == OrderRole::StopLoss {
                            TradingIntentAction::CreateStopLoss
                        } else {
                            TradingIntentAction::CreateTakeProfit
                        },
                        None,
                        Some(position_id.clone()),
                        Some(if preview.role == OrderRole::StopLoss {
                            OrderKind::Stop
                        } else {
                            OrderKind::Limit
                        }),
                        TradingRollback::CreatedProtection,
                        None,
                        None,
                        None,
                    )
                }
            };
        let intent = TradingIntent {
            sequence,
            action,
            account_id: order_id
                .as_ref()
                .and_then(|id| {
                    self.trading_state
                        .orders
                        .iter()
                        .find(|order| &order.id == id)
                })
                .and_then(|order| order.account_id.clone())
                .or_else(|| {
                    position_id
                        .as_ref()
                        .and_then(|id| {
                            self.trading_state
                                .positions
                                .iter()
                                .find(|position| &position.id == id)
                        })
                        .and_then(|position| position.account_id.clone())
                }),
            drawing_id: None,
            order_id,
            position_id,
            pane_index: None,
            price_scale: None,
            side: Some(preview.side),
            kind,
            role: Some(preview.role),
            price: Some(preview.price),
            price_tick_index: self.trading_price_tick_index_on(
                preview.pane_index,
                preview.price_scale,
                preview.price,
            ),
            stop_price,
            take_profit_price: None,
            stop_loss_price: None,
            quantity: Some(preview.quantity),
            bracket_id,
            oco_group_id,
            base_revision: preview.base_revision,
        };
        // Moving a protection order asserts its bracket, so the connector chrome comes up with the
        // change rather than waiting on the host — the same moment the line itself moves.
        if preview.role != OrderRole::Working {
            if let Some(group) = self.trading_group_key_for_preview(&preview) {
                self.trading_state.group_visual = TradingGroupVisualState::Active(group);
            }
        }
        self.trading_state.interaction =
            TradingInteractionState::PendingHostAck { sequence, rollback };
        self.trading_state.push_intent(intent.clone());
        self.invalidate_frame_trading();
        Some(intent)
    }

    pub fn trading_drag_start_at(&mut self, x_css: f64, y_css: f64) -> bool {
        self.trading_drag_start_at_with_profile(x_css, y_css, HitProfile::PRECISION)
    }

    /// Begin keyboard adjustment of a working order without synthesizing screen coordinates.
    pub fn trading_keyboard_start_order(&mut self, id: &OrderId) -> bool {
        if !self.trading_state.interaction.is_idle_or_hovering() {
            return false;
        }
        let Some(order) = self
            .trading_state
            .orders
            .iter()
            .find(|order| &order.id == id)
        else {
            return false;
        };
        let preview = if order.role == OrderRole::Working {
            self.trading_order_protection_preview(order, OrderRole::StopLoss)
        } else {
            self.trading_order_drag_preview(order)
        };
        let Some(preview) = preview else {
            return false;
        };
        self.trading_state.interaction = TradingInteractionState::DraggingOrder {
            authoritative_price: preview.price,
            preview,
        };
        self.invalidate_frame_trading();
        self.invalidate_frame_overlay();
        true
    }

    /// Move the active keyboard preview by exact instrument ticks.
    pub fn trading_keyboard_adjust(&mut self, ticks: i32) -> bool {
        if ticks == 0 {
            return false;
        }
        let tick = self.trading_state.instrument.tick_size.unwrap_or(1.0);
        let Some(current) = self.trading_state.interaction.preview().cloned() else {
            return false;
        };
        // A ladder steps by exact band ticks across band boundaries.
        let price = match self.trading_tick_ladder(current.pane_index, current.price_scale) {
            Some(ladder) => match ladder.step(current.price, i64::from(ticks)) {
                Some(price) => price,
                None => return false,
            },
            None => current.price + f64::from(ticks) * tick,
        };
        let Some(preview) = self.trading_state.interaction.dragging_preview_mut() else {
            return false;
        };
        if !price.is_finite() || price <= 0.0 {
            return false;
        }
        preview.price = price;
        // Keyboard users have no separate TP/SL buttons to aim at, so a new protection created
        // from an entry takes its role from the side of the entry it moves to: toward profit is a
        // take profit, toward loss a stop loss. Pointer drags from the dedicated buttons never
        // come through here and keep their fixed role.
        let entry = match &preview.source {
            TradingPreviewSource::OrderStopLoss { order_id }
            | TradingPreviewSource::OrderTakeProfit { order_id } => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == order_id)
                .cloned(),
            _ => None,
        };
        if let Some(entry) = entry.filter(|entry| entry.price != price) {
            let toward_profit = (price > entry.price) == (entry.side == OrderSide::Buy);
            let role = if toward_profit {
                OrderRole::TakeProfit
            } else {
                OrderRole::StopLoss
            };
            if let Some(mut next) = self.trading_order_protection_preview(&entry, role) {
                next.price = price;
                if let Some(preview) = self.trading_state.interaction.dragging_preview_mut() {
                    *preview = next;
                }
            }
        }
        self.invalidate_frame_trading();
        true
    }

    /// Commit the keyboard preview. Like a pointer release, this emits the intent directly — a
    /// host that wants a confirmation step runs it around the intent, not inside the chart.
    pub fn trading_keyboard_commit(&mut self) -> Option<TradingIntent> {
        self.trading_drag_end()
    }

    pub fn trading_drag_start_at_with_profile(
        &mut self,
        x_css: f64,
        y_css: f64,
        profile: HitProfile,
    ) -> bool {
        if !self.trading_state.interaction.is_idle_or_hovering() {
            return false;
        }
        let Some(hit) = self.trading_hit_at_with_profile(x_css, y_css, profile) else {
            return false;
        };
        let preview = match (&hit.object, hit.kind) {
            (TradingObjectId::Order(id), TradingHitKind::OrderLine) => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == id)
                .and_then(|order| self.trading_order_drag_preview(order)),
            (TradingObjectId::Order(id), TradingHitKind::TakeProfitButton) => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == id)
                .and_then(|order| {
                    self.trading_order_protection_preview(order, OrderRole::TakeProfit)
                }),
            (TradingObjectId::Order(id), TradingHitKind::StopLossButton) => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == id)
                .and_then(|order| {
                    self.trading_order_protection_preview(order, OrderRole::StopLoss)
                }),
            (TradingObjectId::Position(id), TradingHitKind::TakeProfitButton) => self
                .trading_state
                .positions
                .iter()
                .find(|position| &position.id == id)
                .and_then(|position| {
                    self.trading_position_protection_preview(position, OrderRole::TakeProfit)
                }),
            (TradingObjectId::Position(id), TradingHitKind::StopLossButton) => self
                .trading_state
                .positions
                .iter()
                .find(|position| &position.id == id)
                .and_then(|position| {
                    self.trading_position_protection_preview(position, OrderRole::StopLoss)
                }),
            _ => None,
        };
        let Some(preview) = preview else {
            return false;
        };
        self.trading_state.interaction = TradingInteractionState::DraggingOrder {
            authoritative_price: preview.price,
            preview,
        };
        self.invalidate_frame_trading();
        self.invalidate_frame_overlay();
        true
    }

    pub fn trading_drag_to(&mut self, y_css: f64) -> bool {
        let Some(preview) = self.trading_state.interaction.preview().cloned() else {
            return false;
        };
        if self
            .trading_state
            .interaction
            .dragging_preview_mut()
            .is_none()
            || !y_css.is_finite()
        {
            return false;
        }
        let Some(price) =
            self.trading_coordinate_to_price(preview.pane_index, preview.price_scale, y_css)
        else {
            return false;
        };
        let price = self.snap_trading_price_on(preview.pane_index, preview.price_scale, price);
        if !price.is_finite() || price <= 0.0 || preview.price == price {
            return false;
        }
        self.trading_state
            .interaction
            .dragging_preview_mut()
            .expect("dragging interaction owns a preview")
            .price = price;
        self.invalidate_frame_trading();
        true
    }

    pub fn trading_drag_end(&mut self) -> Option<TradingIntent> {
        let preview = self.trading_state.interaction.preview()?.clone();
        let start_price = match &self.trading_state.interaction {
            TradingInteractionState::DraggingOrder {
                authoritative_price,
                ..
            } => *authoritative_price,
            _ => return None,
        };
        let tolerance =
            self.trading_half_tick_on(preview.pane_index, preview.price_scale, start_price);
        if (start_price - preview.price).abs() <= tolerance {
            self.trading_state.interaction = TradingInteractionState::Idle;
            self.invalidate_frame_trading();
            self.invalidate_frame_overlay();
            return None;
        }
        if !self.trading_preview_price_valid(&preview) {
            self.trading_state.interaction = TradingInteractionState::Idle;
            self.invalidate_frame_trading();
            self.invalidate_frame_overlay();
            return None;
        }
        let intent = self.commit_trading_preview();
        if intent.is_none() {
            self.trading_state.interaction = TradingInteractionState::Idle;
            self.invalidate_frame_trading();
        }
        self.invalidate_frame_overlay();
        intent
    }

    pub fn cancel_trading_drag(&mut self) -> bool {
        if !matches!(
            self.trading_state.interaction,
            TradingInteractionState::DraggingOrder { .. }
        ) {
            return false;
        }
        self.trading_state.interaction = TradingInteractionState::Idle;
        self.trading_state.group_visual = TradingGroupVisualState::Inactive;
        self.invalidate_frame_trading();
        self.invalidate_frame_overlay();
        true
    }

    /// Emit one host-authoritative bracket request from a complete Long/Short Position drawing.
    /// The drawing supplies semantic entry, target, stop, pane, and price-scale placement; the
    /// host supplies quantity and remains responsible for broker submission and live order IDs.
    /// No speculative trading objects are inserted into the chart.
    pub fn place_bracket_order_from_drawing(
        &mut self,
        drawing_id: DrawingId,
        quantity: f64,
    ) -> Result<(), ChartError> {
        if !quantity.is_finite() || quantity <= 0.0 {
            return Err(invalid(
                "bracket order quantity must be finite and positive",
            ));
        }
        if self
            .trading_state
            .instrument
            .minimum_quantity
            .is_some_and(|minimum| quantity < minimum)
        {
            return Err(invalid(
                "bracket order quantity is below the instrument minimum",
            ));
        }
        if !self.trading_state.interaction.is_idle_or_hovering() {
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "another trading request is still active",
            ));
        }

        let (kind, pane_index, price_scale, points) = {
            let drawing = self.drawing(drawing_id).ok_or_else(|| {
                ChartError::new(ErrorCode::InvalidHandle, "unknown position drawing")
            })?;
            if !matches!(
                drawing.kind,
                DrawingKind::LongPosition | DrawingKind::ShortPosition
            ) {
                return Err(ChartError::new(
                    ErrorCode::UnsupportedOperation,
                    "bracket orders require a Long Position or Short Position drawing",
                ));
            }
            if drawing.points.len() != 3 {
                return Err(invalid(
                    "position drawing must contain entry, target, and stop",
                ));
            }
            (
                drawing.kind,
                drawing.pane_index,
                drawing.price_scale,
                [
                    drawing.points[0].price,
                    drawing.points[1].price,
                    drawing.points[2].price,
                ],
            )
        };

        let snap_scale = match price_scale {
            DrawingPriceScale::Right => TradingPriceScale::Right,
            DrawingPriceScale::Left => TradingPriceScale::Left,
            DrawingPriceScale::Overlay => TradingPriceScale::Overlay,
        };
        let entry_price = self.snap_trading_price_on(pane_index, snap_scale, points[0]);
        let take_profit_price = self.snap_trading_price_on(pane_index, snap_scale, points[1]);
        let stop_loss_price = self.snap_trading_price_on(pane_index, snap_scale, points[2]);
        let side = match kind {
            DrawingKind::LongPosition
                if take_profit_price > entry_price && stop_loss_price < entry_price =>
            {
                OrderSide::Buy
            }
            DrawingKind::ShortPosition
                if take_profit_price < entry_price && stop_loss_price > entry_price =>
            {
                OrderSide::Sell
            }
            DrawingKind::LongPosition | DrawingKind::ShortPosition => {
                return Err(invalid(
                    "position levels collapse or cross after instrument tick snapping",
                ));
            }
            _ => unreachable!(),
        };
        let price_scale = match price_scale {
            DrawingPriceScale::Right => TradingPriceScale::Right,
            DrawingPriceScale::Left => TradingPriceScale::Left,
            DrawingPriceScale::Overlay => TradingPriceScale::Overlay,
        };
        let sequence = self.trading_state.next_sequence();
        self.trading_state.push_intent(TradingIntent {
            sequence,
            action: TradingIntentAction::PlaceBracketOrder,
            account_id: None,
            drawing_id: Some(drawing_id),
            order_id: None,
            position_id: None,
            pane_index: Some(pane_index),
            price_scale: Some(price_scale),
            side: Some(side),
            // An explicit entry level is a resting entry request. The host may translate or
            // reject it against its live quote and venue rules; the chart never owns that quote.
            kind: Some(OrderKind::Limit),
            role: Some(OrderRole::Working),
            price: Some(entry_price),
            price_tick_index: self.trading_price_tick_index_on(pane_index, snap_scale, entry_price),
            stop_price: None,
            take_profit_price: Some(take_profit_price),
            stop_loss_price: Some(stop_loss_price),
            quantity: Some(quantity),
            bracket_id: None,
            oco_group_id: None,
            base_revision: 0,
        });
        self.trading_state.interaction = TradingInteractionState::PendingHostAck {
            sequence,
            rollback: TradingRollback::CreatedBracketOrder,
        };
        self.invalidate_frame_trading();
        Ok(())
    }

    /// Activate the close control under the pointer. Closing REMOVES the object and emits the
    /// intent: there is no pending tint and no second gate, because "close" means the order or
    /// position is gone. A host that gates closes runs its confirmation around the intent and
    /// rejects it to put the object back.
    pub fn trading_activate_at(&mut self, x_css: f64, y_css: f64) -> bool {
        let Some(hit) = self.trading_hit_at(x_css, y_css) else {
            return false;
        };
        if !self.trading_state.interaction.is_idle_or_hovering() {
            return false;
        }
        if hit.kind != TradingHitKind::CancelButton {
            return false;
        }
        let sequence = self.trading_state.next_sequence();
        let (intent, rollback) = match hit.object {
            TradingObjectId::Order(order_id) => {
                let Some(index) = self
                    .trading_state
                    .orders
                    .iter()
                    .position(|order| order.id == order_id)
                else {
                    return false;
                };
                let order = &self.trading_state.orders[index];
                if !matches!(
                    order.status,
                    OrderStatus::Working | OrderStatus::PartiallyFilled
                ) {
                    return false;
                }
                let remaining = (order.quantity - order.filled_quantity).max(0.0);
                let intent = TradingIntent {
                    sequence,
                    action: TradingIntentAction::CancelOrder,
                    account_id: order.account_id.clone(),
                    drawing_id: None,
                    order_id: Some(order_id.clone()),
                    position_id: order.position_id.clone(),
                    pane_index: None,
                    price_scale: None,
                    side: Some(order.side),
                    kind: Some(order.kind),
                    role: Some(order.role),
                    price: Some(order.price),
                    price_tick_index: self.trading_price_tick_index_on(
                        order.pane_index,
                        order.price_scale,
                        order.price,
                    ),
                    stop_price: order.stop_price,
                    take_profit_price: None,
                    stop_loss_price: None,
                    quantity: Some(remaining),
                    bracket_id: order.bracket_id.clone(),
                    oco_group_id: order.oco_group_id.clone(),
                    base_revision: order.revision,
                };
                let order = self.trading_state.orders.remove(index);
                (
                    intent,
                    TradingRollback::RemovedOrder {
                        index,
                        order: Box::new(order),
                    },
                )
            }
            TradingObjectId::Position(position_id) => {
                let Some(index) = self
                    .trading_state
                    .positions
                    .iter()
                    .position(|position| position.id == position_id)
                else {
                    return false;
                };
                let intent = TradingIntent {
                    sequence,
                    action: TradingIntentAction::ClosePosition,
                    account_id: self.trading_state.positions[index].account_id.clone(),
                    drawing_id: None,
                    order_id: None,
                    position_id: Some(position_id),
                    pane_index: None,
                    price_scale: None,
                    side: None,
                    kind: None,
                    role: None,
                    price: None,
                    price_tick_index: None,
                    stop_price: None,
                    take_profit_price: None,
                    stop_loss_price: None,
                    quantity: None,
                    bracket_id: None,
                    oco_group_id: None,
                    base_revision: 0,
                };
                let position = self.trading_state.positions.remove(index);
                (
                    intent,
                    TradingRollback::RemovedPosition {
                        index,
                        position: Box::new(position),
                    },
                )
            }
            TradingObjectId::Execution(_) => return false,
        };
        // The object is gone, so nothing may still point at it.
        self.trading_state.feedback_hover = None;
        self.trading_state.feedback_pressed = None;
        self.trading_state.tooltip_armed = false;
        self.trading_state.interaction =
            TradingInteractionState::PendingHostAck { sequence, rollback };
        self.trading_state.push_intent(intent);
        self.reconcile_trading_group_visual();
        self.invalidate_frame_trading();
        self.invalidate_frame_scene();
        true
    }

    /// Answer an emitted intent. Acceptance simply releases the rollback — the chart already shows
    /// the change. Rejection puts the object back exactly where it was.
    pub fn resolve_trading_intent(&mut self, sequence: u32, accepted: bool) -> bool {
        let TradingInteractionState::PendingHostAck {
            sequence: pending,
            rollback,
        } = &self.trading_state.interaction
        else {
            return false;
        };
        if *pending != sequence {
            return false;
        }
        if !accepted {
            match rollback.clone() {
                TradingRollback::RemovedOrder { index, order } => {
                    let index = index.min(self.trading_state.orders.len());
                    self.trading_state.orders.insert(index, *order);
                }
                TradingRollback::RemovedPosition { index, position } => {
                    let index = index.min(self.trading_state.positions.len());
                    self.trading_state.positions.insert(index, *position);
                }
                TradingRollback::MovedOrder { id, price } => {
                    if let Some(order) = self
                        .trading_state
                        .orders
                        .iter_mut()
                        .find(|order| order.id == id)
                    {
                        order.price = price;
                    }
                }
                TradingRollback::CreatedBracketOrder | TradingRollback::CreatedProtection => {}
            }
            self.invalidate_frame_scene();
        }
        self.trading_state.interaction = TradingInteractionState::Idle;
        self.trading_state.group_visual = TradingGroupVisualState::Inactive;
        self.invalidate_frame_trading();
        true
    }

    pub fn trading_preview(&self) -> Option<&TradingPreview> {
        self.trading_state.interaction.preview()
    }

    pub fn discard_trading_interaction(&mut self) -> bool {
        if !matches!(
            self.trading_state.interaction,
            TradingInteractionState::DraggingOrder { .. }
        ) {
            return false;
        }
        self.trading_state.interaction = TradingInteractionState::Idle;
        self.trading_state.group_visual = TradingGroupVisualState::Inactive;
        self.invalidate_frame_trading();
        true
    }

    pub fn take_trading_intents(&mut self) -> Vec<TradingIntent> {
        self.trading_state.intents.drain(..).collect()
    }

    pub fn trading_snapshot(&self) -> TradingSnapshot {
        self.trading_state.snapshot()
    }

    /// Replace the bounded, host-owned context overlay atomically. It is not part of drawings,
    /// persistence, undo history, or the broker snapshot.
    pub fn set_host_overlay(&mut self, overlay: HostOverlaySnapshot) -> Result<(), ChartError> {
        validate_host_overlay(&overlay)?;
        self.trading_state.host_overlay = overlay;
        self.invalidate_frame_trading();
        Ok(())
    }

    #[must_use]
    pub fn host_overlay(&self) -> &HostOverlaySnapshot {
        &self.trading_state.host_overlay
    }

    #[must_use]
    pub fn host_event_hit_at(&self, x_css: f64, y_css: f64) -> Option<HostEventHit> {
        let pane_index = self.pane_at_y(y_css)?;
        let coordinate = |time: i64| -> Option<f64> {
            let logical = self.axis_index_for_time(time)? as i64;
            Some(self.time_scale.index_to_coordinate(logical))
        };
        for event in &self.trading_state.host_overlay.events {
            if self.replay_time_is_visible(event.time)
                && coordinate(event.time).is_some_and(|x| (x - x_css).abs() <= 8.0)
            {
                return Some(HostEventHit {
                    id: event.id.clone(),
                    window: false,
                });
            }
        }
        let pane = self.panes.get(pane_index)?;
        for window in &self.trading_state.host_overlay.windows {
            if !self.replay_time_is_visible(window.start_time) {
                continue;
            }
            let end_time = self
                .replay_cutoff_seconds()
                .map_or(window.end_time, |cutoff| window.end_time.min(cutoff));
            let Some(x0) = coordinate(window.start_time) else {
                continue;
            };
            let Some(x1) = coordinate(end_time) else {
                continue;
            };
            if x_css >= x0.min(x1)
                && x_css <= x0.max(x1)
                && y_css >= pane.top
                && y_css <= pane.top + pane.height
            {
                return Some(HostEventHit {
                    id: window.id.clone(),
                    window: true,
                });
            }
        }
        None
    }

    /// Selects the account whose trading objects are visible in chart geometry and hit-testing.
    /// `None` shows every account in the host projection.
    pub fn set_trading_visible_account(&mut self, account_id: Option<AccountId>) {
        self.trading_state.visible_account_id = account_id;
        self.trading_state.feedback_hover = None;
        self.trading_state.feedback_pressed = None;
        self.trading_state.tooltip_armed = false;
        self.trading_state.interaction = TradingInteractionState::Idle;
        self.trading_state.group_visual = TradingGroupVisualState::Inactive;
        self.invalidate_frame_trading();
    }

    /// Returns the host-selected visible account, if one is configured.
    #[must_use]
    pub fn trading_visible_account(&self) -> Option<&AccountId> {
        self.trading_state.visible_account_id.as_ref()
    }

    pub fn set_trading_snapshot(&mut self, snapshot: TradingSnapshot) -> Result<(), ChartError> {
        let total = snapshot.positions.len() + snapshot.orders.len() + snapshot.executions.len();
        if total > MAX_TRADING_OBJECTS {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                format!("trading snapshot exceeds {MAX_TRADING_OBJECTS} objects"),
            ));
        }
        validate_instrument(&snapshot.instrument)?;
        for position in &snapshot.positions {
            validate_position(position)?;
        }
        for order in &snapshot.orders {
            validate_order(order)?;
        }
        for execution in &snapshot.executions {
            validate_execution(execution)?;
        }
        if snapshot.round_trips.len() > MAX_TRADING_ROUND_TRIPS {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                format!("round trips exceed {MAX_TRADING_ROUND_TRIPS}"),
            ));
        }
        for round_trip in &snapshot.round_trips {
            validate_round_trip(round_trip)?;
        }
        validate_unique(
            snapshot.positions.iter().map(|value| value.id.as_str()),
            "position",
        )?;
        validate_unique(
            snapshot.orders.iter().map(|value| value.id.as_str()),
            "order",
        )?;
        validate_unique(
            snapshot.executions.iter().map(|value| value.id.as_str()),
            "execution",
        )?;
        validate_unique(
            snapshot.round_trips.iter().map(|value| value.id.as_str()),
            "round-trip",
        )?;
        let instrument_changed = self.trading_state.instrument != snapshot.instrument;
        let prior = std::mem::take(&mut self.trading_state);
        self.trading_state = TradingState {
            instrument: snapshot.instrument,
            visible_account_id: prior.visible_account_id,
            positions: snapshot.positions,
            orders: snapshot.orders,
            executions: snapshot.executions,
            round_trips: snapshot.round_trips,
            host_overlay: prior.host_overlay,
            style: prior.style,
            interaction: prior.interaction,
            feedback_hover: prior.feedback_hover,
            feedback_pressed: prior.feedback_pressed,
            tooltip_armed: false,
            group_visual: prior.group_visual,
            intents: prior.intents,
            next_intent_sequence: prior.next_intent_sequence,
        };
        self.reconcile_trading_interaction();
        self.reconcile_trading_group_visual();
        self.invalidate_frame_trading();
        if instrument_changed {
            self.invalidate_frame_drawings();
        }
        Ok(())
    }

    pub fn update_trading_position(&mut self, position: TradingPosition) -> Result<(), ChartError> {
        validate_position(&position)?;
        if let Some(index) = self
            .trading_state
            .positions
            .iter()
            .position(|value| value.id == position.id)
        {
            self.trading_state.positions[index] = position;
        } else {
            self.ensure_trading_capacity(1)?;
            self.trading_state.positions.push(position);
        }
        self.reconcile_trading_interaction();
        self.invalidate_frame_trading();
        Ok(())
    }

    pub fn remove_trading_position(&mut self, id: &PositionId) -> bool {
        let before = self.trading_state.positions.len();
        self.trading_state.positions.retain(|value| &value.id != id);
        let changed = before != self.trading_state.positions.len();
        if changed {
            if self.trading_state.interaction.references_position(id) {
                self.trading_state.interaction = TradingInteractionState::Idle;
            }
            self.reconcile_trading_group_visual();
            self.invalidate_frame_trading();
        }
        changed
    }

    pub fn update_working_order(&mut self, order: WorkingOrder) -> Result<(), ChartError> {
        validate_order(&order)?;
        if let Some(index) = self
            .trading_state
            .orders
            .iter()
            .position(|value| value.id == order.id)
        {
            self.trading_state.orders[index] = order;
        } else {
            self.ensure_trading_capacity(1)?;
            self.trading_state.orders.push(order);
        }
        self.reconcile_trading_interaction();
        self.invalidate_frame_trading();
        Ok(())
    }

    pub fn remove_working_order(&mut self, id: &OrderId) -> bool {
        let before = self.trading_state.orders.len();
        self.trading_state.orders.retain(|value| &value.id != id);
        let changed = before != self.trading_state.orders.len();
        if changed {
            if self.trading_state.interaction.references_order(id) {
                self.trading_state.interaction = TradingInteractionState::Idle;
            }
            self.reconcile_trading_group_visual();
            self.invalidate_frame_trading();
        }
        changed
    }

    pub fn apply_trading_execution(
        &mut self,
        execution: TradingExecution,
    ) -> Result<(), ChartError> {
        validate_execution(&execution)?;
        if let Some(index) = self
            .trading_state
            .executions
            .iter()
            .position(|value| value.id == execution.id)
        {
            self.trading_state.executions[index] = execution;
        } else {
            self.ensure_trading_capacity(1)?;
            self.trading_state.executions.push(execution);
        }
        self.invalidate_frame_trading();
        Ok(())
    }

    pub fn remove_trading_execution(&mut self, id: &ExecutionId) -> bool {
        let before = self.trading_state.executions.len();
        self.trading_state
            .executions
            .retain(|value| &value.id != id);
        let changed = before != self.trading_state.executions.len();
        if changed {
            if self.trading_state.interaction.references_execution(id) {
                self.trading_state.interaction = TradingInteractionState::Idle;
            }
            self.invalidate_frame_trading();
        }
        changed
    }

    pub fn set_instrument_metadata(
        &mut self,
        instrument: InstrumentMetadata,
    ) -> Result<(), ChartError> {
        validate_instrument(&instrument)?;
        self.trading_state.instrument = instrument;
        self.invalidate_frame_trading();
        // Position drawing statistics depend on tick size, point value and quantity precision.
        self.invalidate_frame_drawings();
        Ok(())
    }

    pub fn trading_style(&self) -> TradingStyle {
        self.trading_state.style
    }

    pub fn apply_trading_style(&mut self, options: TradingStyleOptions) -> Result<(), ChartError> {
        let mut style = self.trading_state.style;
        macro_rules! apply {
            ($field:ident) => {
                if let Some(color) =
                    parse_style_color(stringify!($field), options.$field.as_deref())?
                {
                    style.$field = color;
                }
            };
        }
        apply!(position);
        apply!(working_order);
        apply!(buy);
        apply!(sell);
        apply!(profit);
        apply!(risk);
        apply!(take_profit);
        apply!(stop_loss);
        apply!(pending);
        apply!(rejected);
        apply!(control);
        apply!(label);
        apply!(execution_buy);
        apply!(execution_sell);
        self.trading_state.style = style;
        self.invalidate_frame_trading();
        Ok(())
    }

    pub(crate) fn remove_trading_pane(&mut self, index: usize) {
        let remap = |pane: usize| {
            if pane == index {
                PANELESS
            } else if pane != PANELESS && pane > index {
                pane - 1
            } else {
                pane
            }
        };
        for pane in self
            .trading_state
            .positions
            .iter_mut()
            .map(|value| &mut value.pane_index)
            .chain(
                self.trading_state
                    .orders
                    .iter_mut()
                    .map(|value| &mut value.pane_index),
            )
            .chain(
                self.trading_state
                    .executions
                    .iter_mut()
                    .map(|value| &mut value.pane_index),
            )
        {
            *pane = remap(*pane);
        }
        if let Some(preview) = self.trading_state.interaction.dragging_preview_mut() {
            preview.pane_index = remap(preview.pane_index);
        }
    }

    pub(crate) fn swap_trading_panes(&mut self, first: usize, second: usize) {
        for pane in self
            .trading_state
            .positions
            .iter_mut()
            .map(|value| &mut value.pane_index)
            .chain(
                self.trading_state
                    .orders
                    .iter_mut()
                    .map(|value| &mut value.pane_index),
            )
            .chain(
                self.trading_state
                    .executions
                    .iter_mut()
                    .map(|value| &mut value.pane_index),
            )
        {
            if *pane == first {
                *pane = second;
            } else if *pane == second {
                *pane = first;
            }
        }
        if let Some(preview) = self.trading_state.interaction.dragging_preview_mut() {
            let mut pane = preview.pane_index;
            if pane == first {
                pane = second;
            } else if pane == second {
                pane = first;
            }
            preview.pane_index = pane;
        }
    }

    pub(crate) fn move_trading_pane(&mut self, from: usize, to: usize) {
        let remap = |pane: usize| {
            if pane == PANELESS {
                pane
            } else if pane == from {
                to
            } else if from < to && pane > from && pane <= to {
                pane - 1
            } else if to < from && pane >= to && pane < from {
                pane + 1
            } else {
                pane
            }
        };
        for pane in self
            .trading_state
            .positions
            .iter_mut()
            .map(|value| &mut value.pane_index)
            .chain(
                self.trading_state
                    .orders
                    .iter_mut()
                    .map(|value| &mut value.pane_index),
            )
            .chain(
                self.trading_state
                    .executions
                    .iter_mut()
                    .map(|value| &mut value.pane_index),
            )
        {
            *pane = remap(*pane);
        }
        if let Some(preview) = self.trading_state.interaction.dragging_preview_mut() {
            preview.pane_index = remap(preview.pane_index);
        }
    }

    fn ensure_trading_capacity(&self, additional: usize) -> Result<(), ChartError> {
        let total = self.trading_state.positions.len()
            + self.trading_state.orders.len()
            + self.trading_state.executions.len();
        if total.saturating_add(additional) > MAX_TRADING_OBJECTS {
            return Err(ChartError::new(
                ErrorCode::ResourceLimit,
                format!("trading state exceeds {MAX_TRADING_OBJECTS} objects"),
            ));
        }
        Ok(())
    }

    fn trading_preview_source_exists(&self, preview: &TradingPreview) -> bool {
        match &preview.source {
            TradingPreviewSource::Order { order_id }
            | TradingPreviewSource::OrderStopLoss { order_id }
            | TradingPreviewSource::OrderTakeProfit { order_id } => self
                .trading_state
                .orders
                .iter()
                .any(|order| &order.id == order_id),
            TradingPreviewSource::StopLoss { position_id }
            | TradingPreviewSource::TakeProfit { position_id } => self
                .trading_state
                .positions
                .iter()
                .any(|position| &position.id == position_id),
        }
    }

    fn trading_group_key_for_order(&self, order: &WorkingOrder) -> Option<TradingGroupKey> {
        order
            .bracket_id
            .clone()
            .map(TradingGroupKey::Bracket)
            .or_else(|| order.position_id.clone().map(TradingGroupKey::Position))
            .or_else(|| {
                order
                    .parent_order_id
                    .clone()
                    .map(TradingGroupKey::ParentOrder)
            })
    }

    fn trading_group_key_for_preview(&self, preview: &TradingPreview) -> Option<TradingGroupKey> {
        let tolerance =
            self.trading_half_tick_on(preview.pane_index, preview.price_scale, preview.price);
        match &preview.source {
            TradingPreviewSource::Order { order_id } => self
                .trading_state
                .orders
                .iter()
                .find(|order| &order.id == order_id && order.role != OrderRole::Working)
                .and_then(|order| self.trading_group_key_for_order(order)),
            TradingPreviewSource::StopLoss { position_id }
            | TradingPreviewSource::TakeProfit { position_id } => self
                .trading_state
                .orders
                .iter()
                .find(|order| {
                    order.position_id.as_ref() == Some(position_id)
                        && order.role == preview.role
                        && (order.price - preview.price).abs() <= tolerance
                })
                .and_then(|order| self.trading_group_key_for_order(order))
                .or_else(|| {
                    self.trading_state
                        .positions
                        .iter()
                        .any(|position| &position.id == position_id)
                        .then(|| TradingGroupKey::Position(position_id.clone()))
                }),
            TradingPreviewSource::OrderStopLoss { order_id }
            | TradingPreviewSource::OrderTakeProfit { order_id } => self
                .trading_state
                .orders
                .iter()
                .find(|order| {
                    order.parent_order_id.as_ref() == Some(order_id)
                        && order.role == preview.role
                        && (order.price - preview.price).abs() <= tolerance
                })
                .and_then(|order| self.trading_group_key_for_order(order))
                .or_else(|| {
                    self.trading_state
                        .orders
                        .iter()
                        .any(|order| &order.id == order_id)
                        .then(|| TradingGroupKey::ParentOrder(order_id.clone()))
                }),
        }
    }

    pub(crate) fn trading_group_contains_order(
        &self,
        group: &TradingGroupKey,
        order: &WorkingOrder,
    ) -> bool {
        match group {
            TradingGroupKey::Bracket(id) => order.bracket_id.as_ref() == Some(id),
            TradingGroupKey::Position(id) => order.position_id.as_ref() == Some(id),
            TradingGroupKey::ParentOrder(id) => {
                &order.id == id || order.parent_order_id.as_ref() == Some(id)
            }
        }
    }

    pub(crate) fn trading_group_contains_position(
        &self,
        group: &TradingGroupKey,
        position: &TradingPosition,
    ) -> bool {
        match group {
            TradingGroupKey::Position(id) => &position.id == id,
            TradingGroupKey::Bracket(_) => self.trading_state.orders.iter().any(|order| {
                self.trading_group_contains_order(group, order)
                    && order.position_id.as_ref() == Some(&position.id)
            }),
            TradingGroupKey::ParentOrder(_) => false,
        }
    }

    fn trading_group_exists(&self, group: &TradingGroupKey) -> bool {
        self.trading_state
            .orders
            .iter()
            .any(|order| self.trading_group_contains_order(group, order))
    }

    fn reconcile_trading_group_visual(&mut self) {
        let TradingGroupVisualState::Active(group) = &self.trading_state.group_visual else {
            return;
        };
        if !self.trading_group_exists(group) {
            self.trading_state.group_visual = TradingGroupVisualState::Inactive;
        }
    }

    pub fn deactivate_trading_group(&mut self) -> bool {
        if self.trading_state.group_visual == TradingGroupVisualState::Inactive {
            return false;
        }
        self.trading_state.group_visual = TradingGroupVisualState::Inactive;
        self.invalidate_frame_trading();
        true
    }

    fn trading_hit_source_exists(&self, hit: &TradingHit) -> bool {
        match &hit.object {
            TradingObjectId::Position(id) => self
                .trading_state
                .positions
                .iter()
                .any(|position| &position.id == id),
            TradingObjectId::Order(id) => self
                .trading_state
                .orders
                .iter()
                .any(|order| &order.id == id),
            TradingObjectId::Execution(id) => self
                .trading_state
                .executions
                .iter()
                .any(|execution| &execution.id == id),
        }
    }

    /// Keep interaction state honest when the host replaces the snapshot underneath it. There is
    /// no pending object to reconcile any more: a released change is already applied, so this only
    /// drops feedback and drags whose object the host removed.
    fn reconcile_trading_interaction(&mut self) {
        if self
            .trading_state
            .feedback_hover
            .as_ref()
            .is_some_and(|hit| !self.trading_hit_source_exists(hit))
        {
            self.trading_state.feedback_hover = None;
            self.trading_state.tooltip_armed = false;
        }
        if self
            .trading_state
            .feedback_pressed
            .as_ref()
            .is_some_and(|hit| !self.trading_hit_source_exists(hit))
        {
            self.trading_state.feedback_pressed = None;
        }
        let source_exists = match &self.trading_state.interaction {
            TradingInteractionState::Idle | TradingInteractionState::PendingHostAck { .. } => {
                return
            }
            TradingInteractionState::Hovering { hit } => self.trading_hit_source_exists(hit),
            TradingInteractionState::DraggingOrder { preview, .. } => {
                self.trading_preview_source_exists(preview)
            }
        };
        if !source_exists {
            self.trading_state.interaction = TradingInteractionState::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ChartEngine, CrosshairSyncPosition, DrawingPoint, PointerInput, TradingPriceScale,
    };
    use aeris_charts_render::draw_list::Prim;

    fn id<T>(value: &str, constructor: impl FnOnce(String) -> Result<T, ChartError>) -> T {
        constructor(value.to_string()).unwrap()
    }

    fn chart_with_market() -> ChartEngine {
        let mut chart = ChartEngine::new(400.0, 240.0, 1.0);
        chart
            .set_series_data(
                0,
                &[10.0, 20.0, 30.0],
                &[99.0, 100.0, 101.0],
                &[102.0, 103.0, 104.0],
                &[98.0, 99.0, 100.0],
                &[101.0, 102.0, 103.0],
            )
            .unwrap();
        chart.time_scale.set_width(400.0);
        chart
    }

    fn protection_button_x(chart: &ChartEngine, y: f64, expected: TradingHitKind) -> f64 {
        (0..=(chart.pane_w * 2.0) as usize)
            .map(|step| step as f64 / 2.0)
            .find(|x| {
                chart
                    .trading_hit_at(*x, y)
                    .is_some_and(|hit| hit.kind == expected)
            })
            .expect("visible protection button")
    }

    #[test]
    fn position_drawing_emits_one_atomic_host_bracket_request() {
        let mut chart = chart_with_market();
        chart
            .set_instrument_metadata(InstrumentMetadata {
                tick_size: Some(0.25),
                minimum_quantity: Some(2.0),
                ..InstrumentMetadata::default()
            })
            .unwrap();
        let drawing_id = chart
            .add_drawing(
                DrawingKind::LongPosition,
                0,
                vec![
                    DrawingPoint {
                        logical: 1.0,
                        price: 101.12,
                    },
                    DrawingPoint {
                        logical: 2.0,
                        price: 104.13,
                    },
                    DrawingPoint {
                        logical: 1.0,
                        price: 98.11,
                    },
                ],
                None,
            )
            .unwrap();
        let authoritative = chart.trading_snapshot();

        chart
            .place_bracket_order_from_drawing(drawing_id, 3.0)
            .unwrap();
        assert_eq!(chart.trading_snapshot(), authoritative);
        let intents = chart.take_trading_intents();
        assert_eq!(intents.len(), 1);
        let intent = &intents[0];
        assert_eq!(intent.action, TradingIntentAction::PlaceBracketOrder);
        assert_eq!(intent.drawing_id, Some(drawing_id));
        assert_eq!(intent.pane_index, Some(0));
        assert_eq!(intent.price_scale, Some(TradingPriceScale::Right));
        assert_eq!(intent.side, Some(OrderSide::Buy));
        assert_eq!(intent.kind, Some(OrderKind::Limit));
        assert_eq!(intent.role, Some(OrderRole::Working));
        assert_eq!(intent.price, Some(101.0));
        assert_eq!(intent.take_profit_price, Some(104.25));
        assert_eq!(intent.stop_loss_price, Some(98.0));
        assert_eq!(intent.quantity, Some(3.0));
        assert_eq!(intent.bracket_id, None);
        assert_eq!(intent.oco_group_id, None);

        let busy = chart
            .place_bracket_order_from_drawing(drawing_id, 3.0)
            .unwrap_err();
        assert_eq!(busy.code(), ErrorCode::UnsupportedOperation);
        assert!(chart.resolve_trading_intent(intent.sequence, false));
        assert_eq!(chart.trading_snapshot(), authoritative);

        let too_small = chart
            .place_bracket_order_from_drawing(drawing_id, 1.0)
            .unwrap_err();
        assert_eq!(too_small.code(), ErrorCode::InvalidData);
        assert!(chart.take_trading_intents().is_empty());

        let short_drawing_id = chart
            .add_drawing(
                DrawingKind::ShortPosition,
                0,
                vec![
                    DrawingPoint {
                        logical: 1.0,
                        price: 101.12,
                    },
                    DrawingPoint {
                        logical: 2.0,
                        price: 98.11,
                    },
                    DrawingPoint {
                        logical: 1.0,
                        price: 104.13,
                    },
                ],
                None,
            )
            .unwrap();
        chart
            .place_bracket_order_from_drawing(short_drawing_id, 2.0)
            .unwrap();
        let short = chart.take_trading_intents().pop().unwrap();
        assert_eq!(short.side, Some(OrderSide::Sell));
        assert_eq!(short.price, Some(101.0));
        assert_eq!(short.take_profit_price, Some(98.0));
        assert_eq!(short.stop_loss_price, Some(104.25));
    }

    /// Center of the close cell that terminates the object's control cluster.
    fn cancel_center(chart: &mut ChartEngine, object: TradingObjectId) -> (f64, f64) {
        chart.build_frame();
        let (price, price_scale, pane_index, width) = match &object {
            TradingObjectId::Position(id) => {
                let position = chart
                    .trading_state
                    .positions
                    .iter()
                    .find(|position| &position.id == id)
                    .expect("position");
                (
                    position.average_price,
                    position.price_scale,
                    position.pane_index,
                    chart.trading_position_cluster_width(position),
                )
            }
            TradingObjectId::Order(id) => {
                let order = chart
                    .trading_state
                    .orders
                    .iter()
                    .find(|order| &order.id == id)
                    .expect("order");
                (
                    chart.trading_effective_order_price(order),
                    order.price_scale,
                    order.pane_index,
                    chart.trading_order_cluster_width(order),
                )
            }
            TradingObjectId::Execution(_) => panic!("executions carry no close control"),
        };
        let y = chart
            .trading_price_coordinate(pane_index, price_scale, price)
            .expect("price coordinate");
        let end = chart.trading_marker_start() + width;
        (end - chart.trading_close_width() / 2.0, y)
    }

    fn position(side: PositionSide) -> TradingPosition {
        TradingPosition {
            id: id("position-1", PositionId::new),
            account_id: None,
            pane_index: 0,
            price_scale: TradingPriceScale::Right,
            side,
            average_price: 101.0,
            quantity: 12.0,
            display_pnl: None,
            currency: None,
            annotations: Vec::new(),
        }
    }

    fn order(id_value: &str, role: OrderRole, price: f64) -> WorkingOrder {
        WorkingOrder {
            id: id(id_value, OrderId::new),
            account_id: None,
            pane_index: 0,
            price_scale: TradingPriceScale::Right,
            side: OrderSide::Sell,
            kind: OrderKind::Limit,
            role,
            status: OrderStatus::Working,
            price,
            stop_price: None,
            trailing_trigger_price: None,
            break_even_trigger_price: None,
            quantity: 12.0,
            filled_quantity: 0.0,
            position_id: Some(id("position-1", PositionId::new)),
            parent_order_id: None,
            bracket_id: Some(id("bracket-1", TradingGroupId::new)),
            oco_group_id: Some(id("oco-1", TradingGroupId::new)),
            revision: 1,
            annotations: Vec::new(),
        }
    }

    #[test]
    fn snapshot_is_transactional_bounded_and_incremental() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![position(PositionSide::Long)],
                orders: vec![order("tp-1", OrderRole::TakeProfit, 103.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let before = chart.trading_snapshot();
        let mut invalid = before.clone();
        invalid.orders[0].filled_quantity = 20.0;
        assert!(chart.set_trading_snapshot(invalid).is_err());
        assert_eq!(chart.trading_snapshot(), before);

        let mut moved = position(PositionSide::Long);
        moved.average_price = 102.0;
        chart.update_trading_position(moved).unwrap();
        assert_eq!(chart.trading_snapshot().positions.len(), 1);
        assert_eq!(chart.trading_snapshot().positions[0].average_price, 102.0);
        assert!(chart.remove_trading_position(&id("position-1", PositionId::new)));
        assert!(chart.trading_snapshot().positions.is_empty());
    }

    #[test]
    fn active_order_preview_follows_pane_move_swap_and_removal() {
        let mut chart = chart_with_market();
        assert_eq!(chart.add_pane(false), Some(1));
        assert_eq!(chart.add_pane(false), Some(2));
        let mut working = order("working-1", OrderRole::Working, 102.0);
        working.pane_index = 2;
        let order_id = working.id.clone();
        chart
            .set_trading_snapshot(TradingSnapshot {
                orders: vec![working],
                ..TradingSnapshot::default()
            })
            .unwrap();
        assert!(chart.trading_keyboard_start_order(&order_id));

        assert!(chart.move_pane(2, 0));
        assert_eq!(chart.trading_state.orders[0].pane_index, 0);
        assert_eq!(
            chart
                .trading_state
                .interaction
                .preview()
                .unwrap()
                .pane_index,
            0
        );
        assert!(chart.swap_panes(0, 1));
        assert_eq!(chart.trading_state.orders[0].pane_index, 1);
        assert_eq!(
            chart
                .trading_state
                .interaction
                .preview()
                .unwrap()
                .pane_index,
            1
        );
        assert!(chart.remove_pane(1));
        assert_eq!(chart.trading_state.orders[0].pane_index, PANELESS);
        assert_eq!(
            chart
                .trading_state
                .interaction
                .preview()
                .unwrap()
                .pane_index,
            PANELESS
        );
    }

    #[test]
    fn trading_frame_owns_regions_lines_labels_executions_and_axis_tags() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    point_value: Some(1.0),
                    currency: Some("USD".to_string()),
                    ..InstrumentMetadata::default()
                },
                positions: vec![position(PositionSide::Long)],
                orders: vec![
                    order("tp-1", OrderRole::TakeProfit, 103.0),
                    order("sl-1", OrderRole::StopLoss, 99.0),
                ],
                executions: vec![TradingExecution {
                    id: id("fill-1", ExecutionId::new),
                    account_id: None,
                    pane_index: 0,
                    price_scale: TradingPriceScale::Right,
                    side: OrderSide::Buy,
                    kind: ExecutionKind::Entry,
                    time: 20,
                    price: 101.0,
                    quantity: 5.0,
                    order_id: None,
                    position_id: Some(id("position-1", PositionId::new)),
                    marker_shape: ExecutionMarkerShape::Circle,
                    size_by_quantity: false,
                }],
                round_trips: Vec::new(),
            })
            .unwrap();
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        assert_eq!(segments.trading_regions_end, segments.series_end);
        assert!(segments.trading_end > segments.drawings_end);
        assert!(
            frame.panes[0].main[segments.series_end..segments.trading_regions_end]
                .iter()
                .all(|primitive| !matches!(primitive, Prim::Rect { .. }))
        );
        let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
        assert!(
            trading
                .iter()
                .filter(|primitive| matches!(primitive, Prim::HLine { .. }))
                .count()
                >= 3
        );
        assert!(trading
            .iter()
            .any(|primitive| matches!(primitive, Prim::Circle { .. })));
        assert!(!trading.iter().any(
            |primitive| matches!(primitive, Prim::Text { text, .. } if text == "B" || text == "S")
        ));

        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        for (price, color) in [
            ("99.00", chart.trading_style().stop_loss),
            ("103.00", chart.trading_style().take_profit),
        ] {
            assert!(axis
                .labels
                .iter()
                .any(|label| label.text == price && label.border == Some((1.0, color))));
        }
        let mut axis_primitives = Vec::new();
        chart.build_axis_primitives_into(&axis, &mut axis_primitives);
        for (name, color) in [
            ("SL", chart.trading_style().stop_loss),
            ("TP", chart.trading_style().take_profit),
        ] {
            assert!(
                axis_primitives.iter().any(|primitive| matches!(
                    primitive,
                    Prim::RoundRect { fill, border_width, .. }
                        if *fill == color && *border_width == 0.0
                )),
                "{name} axis tag must paint a same-color outer border"
            );
        }
        let position_label = axis
            .labels
            .iter()
            .find(|label| {
                label.text == "101.00"
                    && label.background.is_some_and(|(_, _, _, _, color)| {
                        color == chart.trading_position_color(PositionSide::Long)
                    })
            })
            .expect("solid position price label");
        assert!(position_label.border.is_none());
    }

    fn fill(name: &str, side: OrderSide, time: i64, price: f64, quantity: f64) -> TradingExecution {
        TradingExecution {
            id: id(name, ExecutionId::new),
            account_id: None,
            pane_index: 0,
            price_scale: TradingPriceScale::Right,
            side,
            kind: ExecutionKind::Entry,
            time,
            price,
            quantity,
            order_id: None,
            position_id: None,
            marker_shape: ExecutionMarkerShape::default(),
            size_by_quantity: false,
        }
    }

    fn chart_with_fills(executions: Vec<TradingExecution>) -> ChartEngine {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                executions,
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        chart
    }

    fn price_y(chart: &ChartEngine, price: f64) -> f64 {
        chart
            .trading_price_coordinate(0, TradingPriceScale::Right, price)
            .unwrap()
    }

    #[test]
    fn execution_arrows_sit_outside_the_rendered_bar_on_every_series_kind() {
        // Bar at time 20 paints high 103 / low 99 / close 102. Both fills are inside the candle.
        let executions = vec![
            fill("buy", OrderSide::Buy, 20, 100.0, 1.0),
            fill("sell", OrderSide::Sell, 20, 102.5, 1.0),
        ];
        let mut chart = chart_with_fills(executions.clone());
        let layout = chart.trading_execution_layout(0);
        assert_eq!(layout.marks.len(), 2);
        let bar_x = chart.time_scale.index_to_coordinate(1);
        for mark in &layout.marks {
            assert_eq!(mark.x, bar_x);
            match mark.side {
                OrderSide::Buy => assert!(mark.y - mark.height / 2.0 > price_y(&chart, 99.0)),
                OrderSide::Sell => assert!(mark.y + mark.height / 2.0 < price_y(&chart, 103.0)),
            }
        }

        // A line paints only the close. The arrows clear the stroked line across their whole
        // width (the line rises toward bar 2), not just the point on their own bar.
        chart.convert_series_kind(0, crate::SeriesKind::Line);
        chart
            .set_trading_snapshot(TradingSnapshot {
                executions,
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let layout = chart.trading_execution_layout(0);
        let points: Vec<(f64, f64)> = [101.0, 102.0, 103.0]
            .iter()
            .enumerate()
            .map(|(index, &close)| {
                (
                    chart.time_scale.index_to_coordinate(index as i64),
                    price_y(&chart, close),
                )
            })
            .collect();
        let line_y = |x: f64| {
            let segment = points.windows(2).find(|pair| x <= pair[1].0).unwrap();
            let t = (x - segment[0].0) / (segment[1].0 - segment[0].0);
            segment[0].1 + (segment[1].1 - segment[0].1) * t
        };
        let half_line = crate::frame::LINE_WIDTH / 2.0;
        for mark in &layout.marks {
            for step in 0..=20 {
                // Only where the line is actually drawn: between the first and last point.
                let x = (mark.x - mark.size / 2.0 + mark.size * f64::from(step) / 20.0)
                    .clamp(points[0].0, points[2].0);
                match mark.side {
                    OrderSide::Buy => {
                        assert!(mark.y - mark.height / 2.0 >= line_y(x) + half_line);
                    }
                    OrderSide::Sell => {
                        assert!(mark.y + mark.height / 2.0 <= line_y(x) - half_line);
                    }
                }
            }
        }
        let buy = layout
            .marks
            .iter()
            .find(|mark| mark.side == OrderSide::Buy)
            .unwrap();
        assert!(
            (buy.y - price_y(&chart, 100.0)).abs() > buy.size,
            "a line-chart buy arrow must not float at the fill price"
        );

        // Hover marks the exact fill price even where the line draws nothing: the fill really
        // happened there. A dotted lead joins the arrow to it and the tooltip clears both.
        let (x, arrow_bottom) = (buy.x, buy.y + buy.height / 2.0);
        let fill_y = price_y(&chart, 100.0);
        assert_eq!(
            chart.trading_cursor_at(x, buy.y),
            Some(TradingCursor::Pointer)
        );
        assert!(chart.set_trading_hover(x, buy.y));
        let mut regions = Vec::new();
        let mut lines = Vec::new();
        chart.build_trading_frame_for_test(0, 1.0, 1.0, &mut regions, &mut lines);
        assert!(lines.iter().any(|p| matches!(
            p,
            Prim::HLine { y, .. } if *y == fill_y.round() as i32
        )));
        assert!(lines.iter().any(|p| matches!(
            p,
            Prim::VLine { y0, y1, .. }
                if *y0 == arrow_bottom.round() as i32 && *y1 == fill_y.round() as i32
        )));
        assert!(lines.iter().any(|p| matches!(
            p,
            Prim::Text { text, .. } if text == "Buy 1 @ 100.00"
        )));
        assert!(lines.iter().any(|p| matches!(
            p,
            Prim::RoundRect { y, .. } if f64::from(*y) > fill_y
        )));
        // The arrow is an open stroke in the execution color, not a filled glyph.
        let execution_buy = chart.trading_style().execution_buy;
        assert_eq!(
            lines
                .iter()
                .filter(|p| matches!(p, Prim::Polyline { color, .. } if *color == execution_buy))
                .count(),
            2
        );
    }

    #[test]
    fn execution_on_last_bar_keeps_line_like_series_frame_in_bounds() {
        for kind in [
            crate::SeriesKind::Line,
            crate::SeriesKind::Area,
            crate::SeriesKind::Baseline,
        ] {
            let mut chart = chart_with_market();
            chart.convert_series_kind(0, kind);
            chart.fit_content();
            chart
                .set_trading_snapshot(TradingSnapshot {
                    executions: vec![fill("last", OrderSide::Buy, 30, 102.0, 1.0)],
                    ..TradingSnapshot::default()
                })
                .unwrap();

            chart.build_frame();
            assert_eq!(chart.trading_execution_layout(0).marks.len(), 1);
        }
    }

    #[test]
    fn execution_arrows_clear_a_stepped_line_riser() {
        // Closes 101 → 102 → 103. A stepped line holds 101 until bar 1, then rises there, so the
        // riser from 101 stands on bar 1's x even at a wide spacing where a slope would not reach.
        let mut chart = chart_with_market();
        chart.convert_series_kind(0, crate::SeriesKind::Line);
        chart.series[0].line_type = aeris_charts_render::draw_list::LineType::WithSteps;
        chart.set_bar_spacing(60.0);
        chart
            .set_trading_snapshot(TradingSnapshot {
                executions: vec![fill("buy", OrderSide::Buy, 20, 100.0, 1.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let mark = &chart.trading_execution_layout(0).marks[0];
        let riser_bottom = price_y(&chart, 101.0) + crate::frame::LINE_WIDTH / 2.0;
        assert!(mark.y - mark.height / 2.0 >= riser_bottom);
    }

    #[test]
    fn execution_arrows_anchor_to_the_primary_bar_not_overlays_on_its_scale() {
        // Bar 1 paints high 103 / low 99. A host overlay on the same scale (an EMA ribbon output
        // or a compare line) runs far above and below it; the arrows must still hug the candle.
        let executions = vec![
            fill("buy", OrderSide::Buy, 20, 100.0, 1.0),
            fill("sell", OrderSide::Sell, 20, 102.5, 1.0),
        ];
        let expected = chart_with_fills(executions.clone())
            .trading_execution_layout(0)
            .marks;
        let mut chart = chart_with_market();
        for closes in [[110.0, 111.0, 112.0], [90.0, 89.0, 88.0]] {
            let overlay = chart.add_series(crate::SeriesKind::Line);
            chart
                .set_series_data(
                    overlay,
                    &[10.0, 20.0, 30.0],
                    &closes,
                    &closes,
                    &closes,
                    &closes,
                )
                .unwrap();
        }
        chart
            .set_trading_snapshot(TradingSnapshot {
                executions,
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let marks = chart.trading_execution_layout(0).marks;
        assert_eq!(marks.len(), expected.len());
        for (mark, expected) in marks.iter().zip(&expected) {
            assert_eq!(mark.side, expected.side);
            // The overlays widen the autoscaled range, so compare against the candle itself.
            match mark.side {
                OrderSide::Buy => {
                    let gap = mark.y - mark.height / 2.0 - price_y(&chart, 99.0);
                    assert!(gap > 0.0 && gap < mark.size, "buy gap {gap}");
                }
                OrderSide::Sell => {
                    let gap = price_y(&chart, 103.0) - (mark.y + mark.height / 2.0);
                    assert!(gap > 0.0 && gap < mark.size, "sell gap {gap}");
                }
            }
        }
    }

    #[test]
    fn executions_land_on_the_bar_that_contains_their_time() {
        // Bars open at 10, 20, 30. A fill at 29 belongs to the bar opened at 20, never the next.
        let chart = chart_with_fills(vec![fill("f", OrderSide::Buy, 29, 100.0, 1.0)]);
        let layout = chart.trading_execution_layout(0);
        assert_eq!(layout.marks[0].x, chart.time_scale.index_to_coordinate(1));
        // Before the loaded history there is no bar to sit on.
        let chart = chart_with_fills(vec![fill("early", OrderSide::Buy, 5, 100.0, 1.0)]);
        assert!(chart.trading_execution_layout(0).marks.is_empty());
    }

    #[test]
    fn execution_timestamps_reject_nanoseconds_instead_of_binding_to_the_last_bar() {
        let mut chart = chart_with_market();
        let nanos = 1_725_000_000_i64 * 1_000_000_000;
        let error = chart
            .set_trading_snapshot(TradingSnapshot {
                executions: vec![fill("nanos", OrderSide::Buy, nanos, 100.0, 1.0)],
                ..TradingSnapshot::default()
            })
            .expect_err("nanosecond timestamps must fail closed");
        assert!(error.to_string().contains("UTC seconds"));
        assert!(error.to_string().contains("nanoseconds"));
    }

    #[test]
    fn same_bar_fills_share_one_arrow_that_reveals_each_exact_price() {
        let chart = &mut chart_with_fills(vec![
            fill("a", OrderSide::Buy, 20, 100.0, 1.0),
            fill("b", OrderSide::Buy, 25, 101.0, 3.0),
        ]);
        let layout = chart.trading_execution_layout(0);
        assert_eq!(layout.marks.len(), 1);
        let (x, y) = (layout.marks[0].x, layout.marks[0].y);

        // The fill price inside the candle is not the mark; the arrow is.
        assert!(chart.trading_hit_at(x, price_y(chart, 100.0)).is_none());
        let hit = chart.trading_hit_at(x, y).expect("arrow hit");
        assert_eq!(hit.kind, TradingHitKind::ExecutionMarker);
        assert_eq!(
            hit.object,
            TradingObjectId::Execution(id("b", ExecutionId::new))
        );

        assert!(chart.set_trading_hover(x, y));
        let mut regions = Vec::new();
        let mut lines = Vec::new();
        chart.build_trading_frame_for_test(0, 1.0, 1.0, &mut regions, &mut lines);
        for price in [100.0, 101.0] {
            let tick_y = price_y(chart, price).round() as i32;
            assert!(
                lines
                    .iter()
                    .any(|p| matches!(p, Prim::HLine { y, .. } if *y == tick_y)),
                "hover must mark the exact fill price {price} on the bar"
            );
        }
        assert!(lines.iter().any(|p| matches!(
            p,
            Prim::Text { text, .. } if text == "Buy 4 @ 100.75 avg · 2 fills"
        )));
        // Two fills draw one stacked mark: one shaft plus one chevron per fill, under the low.
        let mark = &chart.trading_execution_layout(0).marks[0];
        assert_eq!(mark.chevrons, 2);
        assert!((mark.height - mark.size * 94.0 / 70.0).abs() < 1e-9);
        assert!(mark.y - mark.height / 2.0 > price_y(chart, 99.0));
        let execution_buy = chart.trading_style().execution_buy;
        assert_eq!(
            lines
                .iter()
                .filter(|p| matches!(p, Prim::Polyline { color, .. } if *color == execution_buy))
                .count(),
            3
        );
    }

    #[test]
    fn stacked_execution_marks_draw_one_single_sized_chevron_per_fill_with_one_tail() {
        let single = chart_with_fills(vec![fill("a", OrderSide::Buy, 20, 100.0, 1.0)]);
        let single_mark = &single.trading_execution_layout(0).marks[0];
        let strokes = |chart: &ChartEngine| {
            let mut regions = Vec::new();
            let mut lines = Vec::new();
            chart.build_trading_frame_for_test(0, 1.0, 1.0, &mut regions, &mut lines);
            let color = chart.trading_style().execution_buy;
            lines
                .into_iter()
                .filter_map(|p| match p {
                    Prim::Polyline {
                        color: c,
                        point_count,
                        width,
                        ..
                    } if c == color => Some((point_count, width)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let single_strokes = strokes(&single);
        for count in 2..=7 {
            let fills = (0..count)
                .map(|i| fill(&format!("f{i}"), OrderSide::Buy, 20, 100.0, 1.0))
                .collect();
            let chart = chart_with_fills(fills);
            let mark = &chart.trading_execution_layout(0).marks[0];
            let chevrons = count.min(5);
            assert_eq!(mark.chevrons, chevrons);
            // Same arrow size as one fill; only the stack height grows, one pitch per chevron.
            assert_eq!(mark.size, single_mark.size);
            let expected = mark.size * (70.0 + 24.0 * (chevrons - 1) as f64) / 70.0;
            assert!((mark.height - expected).abs() < 1e-9);
            let strokes = strokes(&chart);
            // Exactly one two-point shaft; every chevron is the single arrow's three-point stroke.
            assert_eq!(strokes.iter().filter(|(n, _)| *n == 2).count(), 1);
            assert_eq!(strokes.iter().filter(|(n, _)| *n == 3).count(), chevrons);
            assert!(strokes.iter().all(|(_, w)| *w == single_strokes[0].1));
        }
    }

    #[test]
    fn long_and_short_regions_follow_side_semantics_and_hits_are_trading_local() {
        for (side, take_profit, stop_loss) in [
            (PositionSide::Long, 103.0, 99.0),
            (PositionSide::Short, 99.0, 103.0),
        ] {
            let mut chart = chart_with_market();
            chart
                .set_trading_snapshot(TradingSnapshot {
                    positions: vec![position(side)],
                    orders: vec![
                        order("tp-1", OrderRole::TakeProfit, take_profit),
                        order("sl-1", OrderRole::StopLoss, stop_loss),
                    ],
                    ..TradingSnapshot::default()
                })
                .unwrap();
            chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            let frame = chart.build_frame();
            assert_eq!(
                frame.panes[0].main[segments.series_end..segments.trading_regions_end]
                    .iter()
                    .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                    .count(),
                0,
                "confirmed protection orders must not leave persistent risk/reward fill"
            );
            let y = chart
                .trading_price_coordinate(0, TradingPriceScale::Right, take_profit)
                .unwrap();
            let line_x = 24.0;
            assert!(line_x < chart.trading_marker_start());
            let hit = chart.trading_hit_at(line_x, y).unwrap();
            assert_eq!(hit.kind, TradingHitKind::OrderLine);
            assert!(matches!(hit.object, TradingObjectId::Order(_)));
            assert_eq!(
                chart.trading_cursor_at(line_x, y),
                Some(TradingCursor::Grab)
            );
            assert!(chart.trading_drag_start_at(line_x, y));
            let preview_price = if side == PositionSide::Long {
                take_profit + 0.5
            } else {
                take_profit - 0.5
            };
            let preview_y = chart
                .trading_price_coordinate(0, TradingPriceScale::Right, preview_price)
                .unwrap();
            assert!(chart.trading_drag_to(preview_y));
            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            assert_eq!(
                frame.panes[0].main[segments.series_end..segments.trading_regions_end]
                    .iter()
                    .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                    .count(),
                1,
                "risk/reward fill is transient preview geometry"
            );
            assert!(chart.cancel_trading_drag());
        }
    }

    #[test]
    fn position_axis_tag_hollows_at_live_price_without_leaving_its_line() {
        let mut chart = chart_with_market();
        let secondary = chart.add_series(crate::SeriesKind::Line);
        chart
            .set_series_data(
                secondary,
                &[10.0, 20.0, 30.0],
                &[102.0; 3],
                &[102.0; 3],
                &[102.0; 3],
                &[102.0; 3],
            )
            .unwrap();
        assert!(chart.series_apply_options_json(secondary, r##"{"color":"#123456"}"##));
        // This test is about the TRADING tag hollowing where it meets the live price, so the main
        // series' own chip must be live and sit AT that price: close the last bar at 102 and keep
        // it on screen (a series chip is itself outlined once its final bar scrolls away).
        chart
            .set_series_data(
                0,
                &[10.0, 20.0, 30.0],
                &[99.0, 100.0, 101.0],
                &[102.0, 103.0, 104.0],
                &[98.0, 99.0, 100.0],
                &[101.0, 102.0, 102.0],
            )
            .unwrap();
        chart.fit_content();
        let mut live_position = position(PositionSide::Long);
        live_position.average_price = 102.0;
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![live_position],
                ..TradingSnapshot::default()
            })
            .unwrap();

        chart.build_frame();
        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        let color = chart.trading_position_color(PositionSide::Long);
        let trading_index = axis
            .labels
            .iter()
            .position(|label| label.text == "102.00" && label.border == Some((1.0, color)))
            .expect("position tag at the live price must be hollow");
        let colliding = &axis.labels[trading_index];
        assert_ne!(colliding.background.unwrap().4, color);
        let expected_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 102.0)
            .unwrap();
        assert!((colliding.y - expected_y).abs() <= f64::EPSILON);
        let primary_index = axis
            .labels
            .iter()
            .rposition(|label| {
                label.text == "102.00" && label.background.is_some() && label.border.is_none()
            })
            .expect("primary live-price label");
        assert!(trading_index < primary_index, "primary label paints last");

        let mut separated_position = position(PositionSide::Long);
        separated_position.average_price = 99.0;
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![separated_position],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        let separated = axis
            .labels
            .iter()
            .find(|label| label.text == "99.00" && label.background.is_some())
            .expect("separated position tag");
        assert!(separated.border.is_none());
        assert_eq!(separated.background.unwrap().4, color);
    }

    #[test]
    fn touch_profile_expands_order_line_hits_to_a_44px_target() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                orders: vec![order("order-1", OrderRole::Working, 103.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        assert!(chart
            .trading_hit_at_with_profile(
                chart.trading_marker_start() + 20.0,
                y + 15.0,
                HitProfile::PRECISION,
            )
            .is_none());
        assert_eq!(
            chart
                .trading_hit_at_with_profile(
                    chart.trading_marker_start() + 20.0,
                    y + 15.0,
                    HitProfile::TOUCH,
                )
                .expect("44px touch order target")
                .kind,
            TradingHitKind::OrderLine
        );
    }

    #[test]
    fn keyboard_entry_adjustment_creates_tick_snapped_protection() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    tick_size: Some(0.25),
                    ..InstrumentMetadata::default()
                },
                orders: vec![order("keyboard-order", OrderRole::Working, 103.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let id = id("keyboard-order", OrderId::new);
        assert!(chart.trading_keyboard_start_order(&id));
        assert!(chart.trading_keyboard_adjust(10));
        let intent = chart
            .trading_drag_end()
            .expect("instant mode emits an intent");
        assert_eq!(intent.action, TradingIntentAction::CreateStopLoss);
        assert_eq!(intent.role, Some(OrderRole::StopLoss));
        assert_eq!(intent.price, Some(105.5));
        // Creation leaves the host-owned entry exactly where it was.
        assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
        assert!(chart.resolve_trading_intent(intent.sequence, false));
        assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);

        // Moving toward profit instead creates a take profit: for this sell entry, below it.
        assert!(chart.trading_keyboard_start_order(&id));
        assert!(chart.trading_keyboard_adjust(-4));
        let intent = chart
            .trading_drag_end()
            .expect("instant mode emits an intent");
        assert_eq!(intent.action, TradingIntentAction::CreateTakeProfit);
        assert_eq!(intent.role, Some(OrderRole::TakeProfit));
        assert_eq!(intent.side, Some(OrderSide::Buy));
        assert_eq!(intent.price, Some(102.0));
    }

    #[test]
    fn unlinked_protection_order_remains_draggable_and_preserves_modify_fields() {
        let mut chart = chart_with_market();
        let mut protection = order("orphan-stop", OrderRole::StopLoss, 99.0);
        protection.kind = OrderKind::StopLimit;
        protection.stop_price = Some(98.5);
        protection.position_id = None;
        protection.parent_order_id = None;
        protection.bracket_id = None;
        protection.oco_group_id = None;
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    tick_size: Some(0.25),
                    ..InstrumentMetadata::default()
                },
                orders: vec![protection],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();

        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 99.0)
            .unwrap();
        let chip_x = chart.trading_marker_start() + 20.0;
        assert_eq!(
            chart.trading_hit_at(chip_x, y).unwrap().kind,
            TradingHitKind::OrderLine,
            "the protection chip body is a drag surface"
        );
        assert!(chart.trading_drag_start_at(chip_x, y));
        let next_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 97.5)
            .unwrap();
        assert!(chart.trading_drag_to(next_y));
        let intent = chart
            .trading_drag_end()
            .expect("unlinked broker protection remains modifiable");
        assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
        assert_eq!(intent.kind, Some(OrderKind::StopLimit));
        assert_eq!(intent.stop_price, Some(98.5));
        assert_eq!(intent.price, Some(97.5));
    }

    #[test]
    fn trading_state_is_chart_local_and_absent_from_drawing_persistence() {
        let mut first = chart_with_market();
        let second = chart_with_market();
        first
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        assert_eq!(first.trading_snapshot().positions.len(), 1);
        assert!(second.trading_snapshot().positions.is_empty());
        assert!(!first.export_state_json().unwrap().contains("position-1"));
    }

    #[test]
    fn order_drag_snaps_previews_and_emits_once_with_a_rollback_on_rejection() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    tick_size: Some(0.25),
                    ..InstrumentMetadata::default()
                },
                positions: vec![position(PositionSide::Long)],
                orders: vec![order("tp-1", OrderRole::TakeProfit, 103.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let start_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        let target_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 102.37)
            .unwrap();
        assert!(chart.trading_drag_start_at(chart.trading_marker_start() + 20.0, start_y));
        assert!(chart.trading_drag_to(target_y));
        assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
        assert_eq!(chart.trading_preview().unwrap().price, 102.25);

        let intent = chart.trading_drag_end().unwrap();
        assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
        assert_eq!(intent.order_id.as_ref().unwrap().as_str(), "tp-1");
        assert_eq!(intent.price, Some(102.25));
        assert_eq!(intent.base_revision, 1);
        // Release applies the snapped price and emits exactly one intent for the whole gesture.
        assert_eq!(chart.trading_snapshot().orders[0].price, 102.25);
        assert_eq!(chart.take_trading_intents(), vec![intent.clone()]);
        assert!(chart.take_trading_intents().is_empty());
        assert!(chart.resolve_trading_intent(intent.sequence, false));
        assert!(chart.trading_preview().is_none());
        assert_eq!(chart.trading_snapshot().orders[0].price, 103.0);
    }

    #[test]
    fn accepted_preview_waits_for_authoritative_update_and_then_reconciles() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    tick_size: Some(0.5),
                    ..InstrumentMetadata::default()
                },
                positions: vec![position(PositionSide::Long)],
                orders: vec![order("tp-1", OrderRole::TakeProfit, 103.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let start_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        let target_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 104.0)
            .unwrap();
        assert!(chart.trading_drag_start_at(chart.trading_marker_start() + 20.0, start_y));
        assert!(chart.trading_drag_to(target_y));
        let intent = chart.trading_drag_end().unwrap();
        // The move is applied the moment it is released; acceptance just releases the rollback.
        assert_eq!(chart.trading_snapshot().orders[0].price, 104.0);
        assert!(chart.resolve_trading_intent(intent.sequence, true));
        assert!(chart.trading_preview().is_none());

        let mut confirmed = order("tp-1", OrderRole::TakeProfit, 104.0);
        confirmed.revision = 2;
        chart.update_working_order(confirmed).unwrap();
        assert_eq!(chart.trading_snapshot().orders[0].price, 104.0);
    }

    #[test]
    fn semantic_trading_style_updates_transactionally() {
        let mut chart = chart_with_market();
        let before = chart.trading_style();
        assert!(chart
            .apply_trading_style(TradingStyleOptions {
                position: Some("not-a-color".to_string()),
                ..TradingStyleOptions::default()
            })
            .is_err());
        assert_eq!(chart.trading_style(), before);
        chart
            .apply_trading_style(TradingStyleOptions {
                position: Some("#123456".to_string()),
                risk: Some("rgba(200, 10, 20, 0.5)".to_string()),
                ..TradingStyleOptions::default()
            })
            .unwrap();
        assert_eq!(chart.trading_style().position, Color::rgb(0x12, 0x34, 0x56));
        assert_eq!(chart.trading_style().risk, Color::rgba(200, 10, 20, 128));
    }

    #[test]
    fn retained_trading_frame_stays_bounded_at_acceptance_object_counts() {
        for count in [10, 50, 100, 500] {
            let mut chart = chart_with_market();
            let orders = (0..count)
                .map(|index| {
                    order(
                        &format!("stress-{index}"),
                        if index % 2 == 0 {
                            OrderRole::TakeProfit
                        } else {
                            OrderRole::StopLoss
                        },
                        98.0 + index as f64 * 0.01,
                    )
                })
                .collect();
            chart
                .set_trading_snapshot(TradingSnapshot {
                    positions: vec![position(PositionSide::Long)],
                    orders,
                    ..TradingSnapshot::default()
                })
                .unwrap();
            chart.build_frame();
            assert_eq!(chart.frame_build_stats().trading_rebuilds, 1);
            assert_eq!(chart.trading_snapshot().orders.len(), count);
            chart.build_frame();
            assert_eq!(chart.frame_build_stats().trading_rebuilds, 0);
            assert!(chart.memory_usage().trading_capacity_bytes > 0);
        }
    }

    #[test]
    fn cancel_intent_is_pending_until_rejected_or_authoritatively_updated() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                orders: vec![order("order-1", OrderRole::Working, 102.0)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Order(id("order-1", OrderId::new)),
        );
        // Closing removes the order outright and emits the intent — no pending copy is left behind.
        assert!(chart.trading_activate_at(cancel_x, cancel_y));
        let rejected = chart.take_trading_intents().pop().unwrap();
        assert_eq!(rejected.action, TradingIntentAction::CancelOrder);
        assert!(chart.trading_snapshot().orders.is_empty());
        assert!(chart.trading_preview().is_none());

        // A rejecting host puts it back exactly as it was.
        assert!(chart.resolve_trading_intent(rejected.sequence, false));
        assert_eq!(chart.trading_snapshot().orders.len(), 1);
        assert_eq!(
            chart.trading_snapshot().orders[0].status,
            OrderStatus::Working
        );
        assert_eq!(chart.trading_snapshot().orders[0].price, 102.0);

        // An accepting host leaves it gone.
        assert!(chart.trading_activate_at(cancel_x, cancel_y));
        let accepted = chart.take_trading_intents().pop().unwrap();
        assert!(chart.resolve_trading_intent(accepted.sequence, true));
        assert!(chart.trading_snapshot().orders.is_empty());
        assert!(matches!(
            chart.trading_state.interaction,
            TradingInteractionState::Idle
        ));
        // A stale answer for an already-settled intent changes nothing.
        assert!(!chart.resolve_trading_intent(accepted.sequence, false));
        assert!(chart.trading_snapshot().orders.is_empty());
    }

    #[test]
    fn closing_a_position_removes_it_and_a_rejection_puts_it_back() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![position(PositionSide::Long)],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        assert!(chart.trading_activate_at(cancel_x, cancel_y));
        let intent = chart.take_trading_intents().pop().unwrap();
        assert_eq!(intent.action, TradingIntentAction::ClosePosition);
        assert!(chart.trading_snapshot().positions.is_empty());
        assert_eq!(
            match &chart.trading_state.interaction {
                TradingInteractionState::PendingHostAck { sequence, .. } => *sequence,
                state => panic!("expected pending host acknowledgement, got {state:?}"),
            },
            intent.sequence
        );
        assert!(chart.resolve_trading_intent(intent.sequence, false));
        assert!(matches!(
            chart.trading_state.interaction,
            TradingInteractionState::Idle
        ));
        let restored = chart.trading_snapshot();
        assert_eq!(restored.positions.len(), 1);
        assert_eq!(restored.positions[0].average_price, 101.0);
    }

    #[test]
    fn working_orders_use_compact_labels_and_dedicated_tp_sl_controls() {
        for (side, kind, expected) in [
            (OrderSide::Buy, OrderKind::Limit, "Buy Limit"),
            (OrderSide::Sell, OrderKind::Stop, "Sell Stop"),
        ] {
            let mut chart = chart_with_market();
            let mut working = order("working-1", OrderRole::Working, 102.0);
            working.position_id = None;
            working.side = side;
            working.kind = kind;
            working.quantity = 1.0;
            chart.update_working_order(working).unwrap();
            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
            let texts = trading
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for text in ["1", expected] {
                assert!(texts.contains(&text), "missing segment {text:?}: {texts:?}");
            }
            for text in ["TP", "SL"] {
                assert!(texts.contains(&text), "missing protection control {text:?}");
            }
            assert!(texts.iter().all(|text| *text != "×"));
            let old_axis_label_height = (chart.options.get().layout.font_size + 5.0) as f32;
            let expected_height = chart.trading_control_height() as f32;
            assert!(expected_height > old_axis_label_height);
            let round_rect_heights = trading
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::RoundRect { h, .. } => Some(*h),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(
                round_rect_heights
                    .iter()
                    .filter(|height| (**height - expected_height).abs() < f32::EPSILON)
                    .count()
                    >= 1,
                "visible outer controls must use the taller trading-label height"
            );
            assert!(
                round_rect_heights
                    .iter()
                    .all(|height| *height <= expected_height),
                "trading controls must not exceed the canonical trading-label height"
            );
        }
    }

    #[test]
    fn dragging_a_buy_entry_up_creates_take_profit_without_moving_the_entry() {
        let mut chart = chart_with_market();
        let mut working = order("working-1", OrderRole::Working, 102.0);
        working.position_id = None;
        working.side = OrderSide::Buy;
        working.kind = OrderKind::Market;
        working.status = OrderStatus::Filled;
        working.filled_quantity = working.quantity;
        chart.update_working_order(working).unwrap();
        chart.build_frame();
        let start_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 102.0)
            .unwrap();
        let target_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        let take_profit_x = protection_button_x(&chart, start_y, TradingHitKind::TakeProfitButton);
        assert!(chart.set_trading_hover(take_profit_x, start_y));
        assert_eq!(
            chart.trading_cursor_at(take_profit_x, start_y),
            Some(TradingCursor::Pointer)
        );
        assert!(chart.trading_drag_start_at(take_profit_x, start_y));
        assert!(chart.trading_drag_to(target_y));

        // The entry becomes dashed and the TP preview is dotted; no Confirm/Discard surface is
        // inserted into the engine-owned marker.
        let dragging = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &dragging.panes[0].main[segments.drawings_end..segments.trading_end];
        assert!(trading
            .iter()
            .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == "TP")));
        assert!(trading.iter().all(|primitive| !matches!(
            primitive,
            Prim::Text { text, .. } if matches!(text.as_str(), "Confirm" | "Discard")
        )));
        assert!(trading.iter().any(|primitive| matches!(
            primitive,
            Prim::HLine {
                style: aeris_charts_render::draw_list::LineStyle::Dotted,
                ..
            }
        )));

        // Release emits creation directly. The broker-owned entry never moves.
        let intent = chart
            .trading_drag_end()
            .expect("release emits the take-profit intent");
        assert_eq!(intent.action, TradingIntentAction::CreateTakeProfit);
        assert_eq!(intent.role, Some(OrderRole::TakeProfit));
        assert_eq!(intent.kind, Some(OrderKind::Limit));
        assert_eq!(intent.order_id.as_ref().unwrap().as_str(), "working-1");
        assert_eq!(chart.take_trading_intents().len(), 1);
        assert!(chart.trading_preview().is_none());
        assert_eq!(chart.trading_snapshot().orders[0].price, 102.0);
        let released = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &released.panes[0].main[segments.drawings_end..segments.trading_end];
        let pending = chart.trading_style().pending;
        assert!(trading.iter().all(|primitive| !matches!(
            primitive,
            Prim::HLine { style, color, .. }
                if *style == aeris_charts_render::draw_list::LineStyle::Dotted
                    || *color == pending
        )));

        // Rejecting an unmaterialized protection request is a no-op on the entry.
        assert!(chart.resolve_trading_intent(intent.sequence, false));
        assert_eq!(chart.trading_snapshot().orders[0].price, 102.0);
    }

    #[test]
    fn dragging_a_sell_entry_up_creates_a_stop_loss() {
        let mut chart = chart_with_market();
        let mut working = order("sell-entry", OrderRole::Working, 102.0);
        working.position_id = None;
        working.side = OrderSide::Sell;
        working.kind = OrderKind::Limit;
        chart.update_working_order(working).unwrap();
        chart.build_frame();

        let start_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 102.0)
            .unwrap();
        let stop_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        let stop_loss_x = protection_button_x(&chart, start_y, TradingHitKind::StopLossButton);
        assert!(chart.trading_drag_start_at(stop_loss_x, start_y));
        assert!(chart.trading_drag_to(stop_y));

        let preview = chart.trading_preview().expect("sell stop-loss preview");
        assert_eq!(preview.role, OrderRole::StopLoss);
        assert_eq!(preview.side, OrderSide::Buy);
        let intent = chart.trading_drag_end().expect("stop-loss intent");
        assert_eq!(intent.action, TradingIntentAction::CreateStopLoss);
        assert_eq!(intent.role, Some(OrderRole::StopLoss));
        assert_eq!(intent.kind, Some(OrderKind::Stop));
        assert_eq!(intent.side, Some(OrderSide::Buy));
        assert_eq!(intent.order_id.as_ref().unwrap().as_str(), "sell-entry");
    }

    #[test]
    fn protection_drags_release_the_same_way_as_working_orders() {
        for (order_id, role, price, target) in [
            ("tp-1", OrderRole::TakeProfit, 103.0, 103.5),
            // Moving a confirmed SL above a long entry retains its SL identity.
            ("sl-1", OrderRole::StopLoss, 99.0, 103.5),
        ] {
            let mut chart = chart_with_market();
            chart
                .update_trading_position(position(PositionSide::Long))
                .unwrap();
            let mut protection = order(order_id, role, price);
            protection.position_id = Some(id("position-1", PositionId::new));
            chart.update_working_order(protection).unwrap();
            chart.build_frame();

            let start_y = chart
                .trading_price_coordinate(0, TradingPriceScale::Right, price)
                .unwrap();
            let target_y = chart
                .trading_price_coordinate(0, TradingPriceScale::Right, target)
                .unwrap();
            assert!(chart.trading_drag_start_at(chart.trading_marker_start() + 20.0, start_y));
            assert!(chart.trading_drag_to(target_y));
            let intent = chart.trading_drag_end().expect("release emits the intent");
            assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
            assert_eq!(intent.order_id.as_ref().unwrap().as_str(), order_id);
            assert_eq!(intent.role, Some(role));
            assert!(chart.trading_preview().is_none());
            assert_eq!(
                chart
                    .trading_snapshot()
                    .orders
                    .iter()
                    .find(|order| order.id.as_str() == order_id)
                    .unwrap()
                    .price,
                target
            );

            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
            assert!(trading.iter().all(|primitive| !matches!(
                primitive,
                Prim::Text { text, .. } if matches!(text.as_str(), "Confirm" | "Discard")
            )));
        }
    }

    #[test]
    fn close_tooltip_names_the_exact_action_for_each_order_state() {
        let partially_filled = WorkingOrder {
            status: OrderStatus::PartiallyFilled,
            filled_quantity: 7.0,
            ..order("order-1", OrderRole::Working, 102.0)
        };
        let pending = WorkingOrder {
            status: OrderStatus::PendingSubmit,
            ..order("order-1", OrderRole::Working, 102.0)
        };
        let remaining = format!(
            "Cancel remaining {}",
            chart_with_market().format_trading_quantity(5.0)
        );
        for (order, expected) in [
            (
                order("order-1", OrderRole::Working, 102.0),
                Some("Cancel order"),
            ),
            (partially_filled, Some(remaining.as_str())),
            (
                order("order-1", OrderRole::TakeProfit, 102.0),
                Some("Cancel take profit"),
            ),
            (
                order("order-1", OrderRole::StopLoss, 102.0),
                Some("Cancel stop loss"),
            ),
            // The close control does nothing on a pending order, so it must not claim an action.
            (pending, None),
        ] {
            let mut chart = chart_with_market();
            chart
                .set_trading_snapshot(TradingSnapshot {
                    orders: vec![order],
                    ..TradingSnapshot::default()
                })
                .unwrap();
            let (cancel_x, cancel_y) = cancel_center(
                &mut chart,
                TradingObjectId::Order(id("order-1", OrderId::new)),
            );
            chart.set_trading_hover(cancel_x, cancel_y);
            chart.arm_trading_tooltip();
            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            let labels = frame.panes[0].main[segments.drawings_end..segments.trading_end]
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Text { text, .. }
                        if text.starts_with("Cancel") || text.starts_with("Close") =>
                    {
                        Some(text.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(labels, expected.into_iter().collect::<Vec<_>>());
        }
    }

    #[test]
    fn close_feedback_uses_brand_hover_and_active_surfaces_not_the_order_color() {
        use aeris_charts_core::style::{DARK_ACCENT_RGB, DARK_ACTIVE_RGB};
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        let close_surface = |chart: &mut ChartEngine| {
            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            frame.panes[0].main[segments.drawings_end..segments.trading_end]
                .iter()
                .find_map(|primitive| match primitive {
                    Prim::RoundRect { x, w, h, fill, .. }
                        if w == h && f64::from(*x) <= cancel_x && f64::from(x + w) >= cancel_x =>
                    {
                        Some(*fill)
                    }
                    _ => None,
                })
        };
        let rgb = |token: (u8, u8, u8)| Color::rgb(token.0, token.1, token.2);
        // At rest the close control has no surface of its own.
        assert_eq!(close_surface(&mut chart), None);
        assert!(chart.set_trading_hover(cancel_x, cancel_y));
        assert_eq!(close_surface(&mut chart), Some(rgb(DARK_ACCENT_RGB)));
        assert!(chart.set_trading_pressed(cancel_x, cancel_y));
        assert_eq!(close_surface(&mut chart), Some(rgb(DARK_ACTIVE_RGB)));
    }

    #[test]
    fn pill_outline_is_a_solid_theme_hairline_at_every_dpr() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        for vpr in [1.0_f64, 1.5, 2.0, 3.0] {
            let stroke = (aeris_charts_core::style::BORDER_WIDTH * vpr)
                .floor()
                .max(1.0) as f32;
            let (mut regions, mut out) = (Vec::new(), Vec::new());
            chart.build_trading_frame_for_test(0, vpr, vpr, &mut regions, &mut out);
            let expected = chart.trading_position_color(PositionSide::Long);
            let outlines = out
                .iter()
                .filter(|primitive| {
                    matches!(
                        primitive,
                        Prim::RoundRect { x, border_width, border_color, .. }
                            if *border_width == stroke
                                && *border_color == expected
                                && (f64::from(*x)
                                    - chart.trading_marker_start() * vpr)
                                    .abs()
                                    <= 0.5
                    )
                })
                .count();
            assert_eq!(outlines, 1, "one solid pill outline at dpr {vpr}");
        }
    }

    #[test]
    fn action_tooltips_follow_the_theme_and_hollow_outlines_stay_hairline() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        assert!(chart.set_trading_hover(cancel_x, cancel_y));
        assert!(chart.arm_trading_tooltip());
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];

        let tooltip_y = trading
            .iter()
            .find_map(|primitive| match primitive {
                Prim::Text { text, y, .. } if text == "Close position" => Some(*y),
                _ => None,
            })
            .expect("armed tooltip");
        let (fill, border_color) = trading
            .iter()
            .find_map(|primitive| match primitive {
                Prim::RoundRect {
                    y,
                    h,
                    fill,
                    border_width,
                    border_color,
                    ..
                } if *border_width > 0.0 && *y <= tooltip_y && *y + *h >= tooltip_y => {
                    Some((*fill, *border_color))
                }
                _ => None,
            })
            .expect("tooltip box");
        // Chart chrome, not the object's chrome: the box takes the theme's surface and border,
        // never the position's buy/sell color.
        let position_color = chart.trading_position_color(PositionSide::Long);
        assert_ne!(border_color, position_color);
        assert_ne!(fill, position_color.solid());
        let axis_border =
            Color::parse_css(&chart.options.get().right_price_scale.border_color).unwrap();
        assert_eq!(border_color, axis_border.solid());

        // Every outlined chip uses the brand's 0.5 CSS px `--border-width`, snapped like a browser
        // border: whole device pixels, never below one.
        for vpr in [1.0_f64, 1.5, 2.0, 3.0, 4.0] {
            let hairline = (aeris_charts_core::style::BORDER_WIDTH * vpr)
                .floor()
                .max(1.0) as f32;
            let mut out = Vec::new();
            let mut regions = Vec::new();
            chart.build_trading_frame_for_test(0, vpr, vpr, &mut regions, &mut out);
            for primitive in &out {
                if let Prim::RoundRect { border_width, .. } = primitive {
                    if *border_width > 0.0 {
                        assert_eq!(
                            *border_width, hairline,
                            "outline is not a hairline at dpr {vpr}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn protection_buttons_share_the_marker_pill_box_at_fractional_line_positions() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        // Sweep line positions so the line lands on fractional device pixels at several DPRs.
        for price in [100.13, 100.37, 100.5, 100.71, 101.0] {
            let mut position = position(PositionSide::Long);
            position.average_price = price;
            chart.update_trading_position(position).unwrap();
            for dpr in [1.0_f64, 1.25, 1.5, 2.0] {
                let mut out = Vec::new();
                chart.build_trading_frame_for_test(0, dpr, dpr, &mut Vec::new(), &mut out);
                let outlined: Vec<_> = out
                    .iter()
                    .filter_map(|primitive| match primitive {
                        Prim::RoundRect {
                            y, h, border_width, ..
                        } if *border_width > 0.0 => Some((*y, *h)),
                        _ => None,
                    })
                    .collect();
                // TP button, SL button, and the marker pill.
                assert_eq!(outlined.len(), 3, "price {price} dpr {dpr}");
                for (y, h) in &outlined {
                    assert_eq!((*y, *h), outlined[0], "price {price} dpr {dpr}");
                    assert_eq!(y.fract(), 0.0);
                    assert_eq!(h.fract(), 0.0);
                }
            }
        }
    }

    #[test]
    fn trading_control_text_is_optically_centered_in_its_outlined_box() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        let texts_and_boxes = |chart: &ChartEngine, dpr: f64| {
            let mut out = Vec::new();
            chart.build_trading_frame_for_test(0, dpr, dpr, &mut Vec::new(), &mut out);
            let boxes: Vec<_> = out
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::RoundRect {
                        x,
                        y,
                        w,
                        h,
                        border_width,
                        ..
                    } if *border_width > 0.0 => Some((*x, *y, *w, *h)),
                    _ => None,
                })
                .collect();
            let texts: Vec<_> = out
                .iter()
                .filter_map(|primitive| match primitive {
                    Prim::Text { x, y, text, .. } => Some((text.clone(), *x, *y)),
                    _ => None,
                })
                .collect();
            (texts, boxes)
        };
        let box_center_of = |boxes: &[(f32, f32, f32, f32)], x: f32| {
            boxes
                .iter()
                .find(|(left, _, width, _)| x > *left && x < *left + *width)
                .map(|(_, top, _, height)| *top + *height / 2.0)
                .expect("text sits inside an outlined control")
        };

        // TP, SL, quantity, and P&L text all anchor on their box center plus one shared host
        // cap-height correction, so each has equal visual padding above and below.
        let dpr = 2.0;
        chart.set_text_cap_center(Some(Box::new(|size, _, _, _| size / 12.0 * 1.25)));
        let (texts, boxes) = texts_and_boxes(&chart, dpr);
        for label in ["TP", "SL"] {
            assert!(
                texts.iter().any(|(text, ..)| text == label),
                "{label} missing"
            );
        }
        assert!(texts.len() >= 4);
        for (text, x, y) in &texts {
            let expected = box_center_of(&boxes, *x) + (1.25 * dpr) as f32;
            assert!(
                (*y - expected).abs() <= 0.01,
                "{text:?} anchored at {y}, expected {expected}"
            );
        }

        // Without a host metric, text stays on the geometric center.
        chart.set_text_cap_center(None);
        let (texts, boxes) = texts_and_boxes(&chart, dpr);
        for (text, x, y) in &texts {
            let expected = box_center_of(&boxes, *x);
            assert!(
                (*y - expected).abs() <= 0.01,
                "{text:?} anchored at {y}, expected {expected}"
            );
        }
    }

    #[test]
    fn trading_object_geometry_uses_one_projected_y_and_a_bounded_marker_span() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        let entry_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
        let marker_end = chart.trading_marker_end().round() as i32;
        for primitive in trading {
            match primitive {
                Prim::HLine {
                    y, x0, x1, color, ..
                } if *color == chart.trading_position_color(PositionSide::Long) => {
                    assert_eq!(*y, entry_y.round() as i32);
                    // Marker rules extend across the whole pane; only the marker is interactive.
                    assert_eq!(*x0, 0);
                    assert_eq!(*x1, marker_end);
                    assert_eq!(*x1, chart.pane_w.round() as i32);
                }
                Prim::RoundRect { y, h, .. } => {
                    assert!((*y + *h / 2.0 - entry_y as f32).abs() <= 0.5);
                }
                Prim::Text { y, text, .. } if matches!(text.as_str(), "12" | "—") => {
                    assert!((*y - entry_y as f32).abs() <= 0.5);
                }
                _ => {}
            }
        }

        chart.set_price_scale_visible_range(0, false, 95.0, 110.0);
        let reprojected_entry = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        assert!(
            frame.panes[0].main[segments.drawings_end..segments.trading_end]
                .iter()
                .any(|primitive| matches!(
                    primitive,
                    Prim::HLine { y, x0, x1, color, .. }
                        if *color == chart.trading_position_color(PositionSide::Long)
                            && *y == reprojected_entry.round() as i32
                            && *x0 == 0
                            && *x1 == marker_end
                ))
        );
    }

    #[test]
    fn trading_close_controls_remain_at_their_exact_price_coordinates() {
        let mut chart = chart_with_market();
        let mut working = order("working-1", OrderRole::Working, 102.0);
        working.position_id = None;
        let mut partial = order("partial-1", OrderRole::Working, 100.0);
        partial.position_id = None;
        partial.quantity = 12.0;
        partial.filled_quantity = 5.0;
        partial.status = OrderStatus::PartiallyFilled;
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![position(PositionSide::Long)],
                orders: vec![
                    order("tp-1", OrderRole::TakeProfit, 103.0),
                    order("sl-1", OrderRole::StopLoss, 99.0),
                    working,
                    partial,
                ],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();

        for (object, price) in [
            (
                TradingObjectId::Position(id("position-1", PositionId::new)),
                101.0,
            ),
            (TradingObjectId::Order(id("tp-1", OrderId::new)), 103.0),
            (TradingObjectId::Order(id("sl-1", OrderId::new)), 99.0),
            (TradingObjectId::Order(id("working-1", OrderId::new)), 102.0),
            (TradingObjectId::Order(id("partial-1", OrderId::new)), 100.0),
        ] {
            let expected_y = chart
                .trading_price_coordinate(0, TradingPriceScale::Right, price)
                .unwrap();
            let (cancel_x, cancel_y) = cancel_center(&mut chart, object.clone());
            assert!(
                (cancel_y - expected_y).abs() <= f64::EPSILON,
                "{object:?} moved away from its price: expected {expected_y}, got {cancel_y}"
            );
            assert_eq!(
                chart.trading_hit_at(cancel_x, cancel_y),
                Some(TradingHit {
                    object: object.clone(),
                    kind: TradingHitKind::CancelButton,
                    distance: 0.0,
                    annotation_id: None,
                }),
                "{object:?} close control is not reachable at its own price"
            );
        }
    }

    #[test]
    fn order_colors_preserve_limit_side_and_protection_semantics() {
        let mut chart = chart_with_market();
        let mut sell_limit = order("sell-limit", OrderRole::Working, 103.0);
        sell_limit.side = OrderSide::Sell;
        sell_limit.kind = OrderKind::Limit;
        let mut sell_stop = order("sell-stop", OrderRole::Working, 101.5);
        sell_stop.side = OrderSide::Sell;
        sell_stop.kind = OrderKind::Stop;
        let mut filled_sell_limit = order("filled-sell-limit", OrderRole::Working, 99.0);
        filled_sell_limit.position_id = None;
        filled_sell_limit.side = OrderSide::Sell;
        filled_sell_limit.kind = OrderKind::Limit;
        filled_sell_limit.status = OrderStatus::Filled;
        filled_sell_limit.filled_quantity = filled_sell_limit.quantity;
        let mut buy_limit = order("buy-limit", OrderRole::Working, 100.5);
        buy_limit.position_id = None;
        buy_limit.side = OrderSide::Buy;
        buy_limit.kind = OrderKind::Limit;
        let mut take_profit = order("take-profit", OrderRole::TakeProfit, 104.0);
        take_profit.side = OrderSide::Sell;
        take_profit.kind = OrderKind::Limit;
        let mut stop_loss = order("stop-loss", OrderRole::StopLoss, 98.0);
        stop_loss.side = OrderSide::Buy;
        stop_loss.kind = OrderKind::Stop;
        chart
            .set_trading_snapshot(TradingSnapshot {
                orders: vec![
                    sell_limit,
                    sell_stop,
                    filled_sell_limit,
                    buy_limit,
                    take_profit,
                    stop_loss,
                ],
                ..TradingSnapshot::default()
            })
            .unwrap();

        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
        let style = chart.trading_style();
        for (price, expected) in [
            (103.0, style.sell),
            (100.5, style.buy),
            (101.5, style.sell),
            (99.0, style.sell),
            (104.0, style.take_profit),
            (98.0, style.stop_loss),
        ] {
            let y = chart
                .trading_price_coordinate(0, TradingPriceScale::Right, price)
                .unwrap()
                .round() as i32;
            assert!(
                trading.iter().any(|primitive| matches!(
                    primitive,
                    Prim::HLine { y: line_y, color, .. } if *line_y == y && *color == expected
                )),
                "order at {price} did not paint its expected color"
            );
        }

        // The price tag is solid only once the order is actually filled.
        let axis = chart.build_axis_frame(
            100.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        for (price, filled) in [("103.00", false), ("100.50", false), ("99.00", true)] {
            // Plain scale ticks carry the same text; the action tag is the one with a chip.
            let label = axis
                .labels
                .iter()
                .find(|label| label.text == price && label.background.is_some())
                .unwrap_or_else(|| panic!("axis tag for {price}"));
            assert_eq!(
                label.border.is_none(),
                filled,
                "{price} tag solidity does not follow its fill state"
            );
        }
    }

    #[test]
    fn compact_markers_share_one_start_and_keep_protection_details() {
        let mut chart = chart_with_market();
        let mut working = order("working-1", OrderRole::Working, 102.0);
        working.position_id = None;
        working.bracket_id = None;
        working.oco_group_id = None;
        working.side = OrderSide::Buy;
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    point_value: Some(1.0),
                    currency: Some("USD".to_string()),
                    ..InstrumentMetadata::default()
                },
                positions: vec![position(PositionSide::Long)],
                orders: vec![
                    order("tp-1", OrderRole::TakeProfit, 103.0),
                    order("sl-1", OrderRole::StopLoss, 99.0),
                    working,
                ],
                ..TradingSnapshot::default()
            })
            .unwrap();

        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
        // One pill contains quantity, detail, and the integrated close cell for every marker.
        let pill_radius = (chart.trading_control_height() / 2.0)
            .min(aeris_charts_core::style::RADIUS_LARGE) as f32;
        let marker_start = chart.trading_marker_start() as f32;
        let chips = trading
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::RoundRect {
                    x,
                    y,
                    w,
                    radii,
                    border_width,
                    ..
                } if *border_width > 0.0
                    && *radii == [pill_radius; 4]
                    && (*x - marker_start).abs() <= 0.5 =>
                {
                    Some((*x, *y, *w, *radii))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(chips.len(), 4);
        for (start, _, width, radii) in &chips {
            assert!(*width > 0.0);
            assert!((*start - marker_start).abs() <= 0.5);
            assert_eq!(
                *radii, [pill_radius; 4],
                "chip at {start} must use the shared pill-radius token"
            );
        }
        // Each marker has one quantity fill at the shared start, behind the 1px inside border
        // and the 2px surface gap.
        assert_eq!(
            trading
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::RoundRect { x, fill, border_width, radii, .. }
                        if *border_width == 0.0
                            && fill.a() == 255
                            && *radii != [pill_radius; 4]
                            && (*x - (marker_start.round() + 3.0)).abs() <= 0.5
                ))
                .count(),
            4
        );
        for expected in ["Buy Limit", "+24.00 USD", "-24.00 USD"] {
            assert!(trading
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == expected)));
        }
        for expected in ["TP", "SL"] {
            assert!(trading
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == expected)));
        }
        assert!(trading
            .iter()
            .all(|primitive| !matches!(primitive, Prim::Text { text, .. } if matches!(text.as_str(), "×" | "↕"))));
        let marker_end = chart.trading_marker_end().round() as i32;
        assert_eq!(
            (chart.trading_marker_end() - chart.trading_marker_start()).round(),
            304.0,
            "the complete marker cluster moves left with its line"
        );
        assert_eq!(
            trading
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::HLine { x0, x1, .. } if *x0 == 0 && *x1 == marker_end
                ))
                .count(),
            4,
            "every marker line must extend from the pane's left edge to its marker"
        );
        let working_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 102.0)
            .expect("working-order coordinate");
        let marker_start_hit = chart
            .trading_hit_at(chart.trading_marker_start() + 1.0, working_y)
            .expect("the shifted order marker must remain interactive");
        assert_eq!(marker_start_hit.kind, TradingHitKind::OrderLine);
        assert!(matches!(
            marker_start_hit.object,
            TradingObjectId::Order(ref order_id) if order_id.as_str() == "working-1"
        ));
    }

    #[test]
    fn quantity_cell_and_close_hit_follow_the_formatted_text_width() {
        let mut chart = chart_with_market();
        chart.set_text_measure(Some(Box::new(|text, _size, _family, _weight, _italic| {
            text.chars().count() as f64 * 6.0
        })));

        let quantity_fill_width = |chart: &mut ChartEngine| {
            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            frame.panes[0].main[segments.drawings_end..segments.trading_end]
                .iter()
                .find_map(|primitive| match primitive {
                    Prim::RoundRect {
                        x, w, border_width, ..
                    } if *border_width > 0.0
                        && (f64::from(*x) - chart.trading_marker_start()).abs() <= 0.5 =>
                    {
                        Some(*w)
                    }
                    _ => None,
                })
                .expect("solid quantity cell")
        };

        let short = position(PositionSide::Long);
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![short.clone()],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let short_cluster = chart.trading_position_cluster_width(&short);
        let short_fill = quantity_fill_width(&mut chart);
        let old_close_center =
            chart.trading_marker_start() + short_cluster - chart.trading_close_width() / 2.0;

        let mut long = short;
        long.quantity = 123_456.0;
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![long.clone()],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let long_cluster = chart.trading_position_cluster_width(&long);
        let long_fill = quantity_fill_width(&mut chart);
        assert_eq!(long_fill - short_fill, 24.0);
        assert_eq!(long_cluster - short_cluster, 24.0);
        assert_eq!(
            chart.trading_position_chip_hit(&long, old_close_center),
            TradingHitKind::PositionLine,
            "the former close location becomes part of the resized readout"
        );
        let new_close_center =
            chart.trading_marker_start() + long_cluster - chart.trading_close_width() / 2.0;
        assert_eq!(
            chart.trading_position_chip_hit(&long, new_close_center),
            TradingHitKind::CancelButton
        );
    }

    #[test]
    fn bracket_connector_disappears_after_host_ack() {
        let mut chart = chart_with_market();
        chart
            .set_trading_snapshot(TradingSnapshot {
                instrument: InstrumentMetadata {
                    tick_size: Some(0.25),
                    ..InstrumentMetadata::default()
                },
                positions: vec![position(PositionSide::Long)],
                orders: vec![
                    order("tp-1", OrderRole::TakeProfit, 103.0),
                    order("sl-1", OrderRole::StopLoss, 99.0),
                ],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.build_frame();
        let start_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 99.0)
            .unwrap();
        let stop_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 98.0)
            .unwrap();
        assert!(chart.trading_drag_start_at(chart.trading_marker_start() + 20.0, start_y));
        assert!(chart.trading_drag_to(stop_y));
        let intent = chart
            .trading_drag_end()
            .expect("stop-loss modification intent");
        assert_eq!(intent.action, TradingIntentAction::ModifyOrder);
        assert!(chart.resolve_trading_intent(intent.sequence, true));
        let mut stop = order("sl-1", OrderRole::StopLoss, 98.0);
        stop.kind = OrderKind::Stop;
        stop.revision = 2;
        chart.update_working_order(stop).unwrap();
        assert_eq!(
            chart.trading_state.group_visual,
            TradingGroupVisualState::Inactive
        );

        let connector_x = (chart.pane_w - 8.0).round() as i32;
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        assert!(
            !frame.panes[0].main[segments.drawings_end..segments.trading_end]
                .iter()
                .any(|primitive| matches!(primitive, Prim::VLine { x, .. } if *x == connector_x))
        );

        assert!(!chart.deactivate_trading_group());
        assert_eq!(chart.trading_snapshot().positions.len(), 1);
        assert_eq!(chart.trading_snapshot().orders.len(), 2);
        let frame = chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let trading = &frame.panes[0].main[segments.drawings_end..segments.trading_end];
        assert!(!trading
            .iter()
            .any(|primitive| matches!(primitive, Prim::VLine { x, .. } if *x == connector_x)));
        for expected in ["+24.00", "-36.00"] {
            assert!(trading
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == expected)));
        }
    }

    #[test]
    fn cancel_hover_adds_tooltip_without_moving_the_position_cluster() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Short))
            .unwrap();
        let pill_radius = (chart.trading_control_height() / 2.0)
            .min(aeris_charts_core::style::RADIUS_LARGE) as f32;
        let baseline = chart.build_frame();
        let pane_segments = chart.frame_pane_segments(0).unwrap();
        let bounds = baseline.panes[0].main[pane_segments.drawings_end..pane_segments.trading_end]
            .iter()
            .find_map(|primitive| match primitive {
                Prim::RoundRect {
                    x,
                    y,
                    w,
                    h,
                    border_width,
                    radii,
                    ..
                } if *border_width > 0.0
                    && *radii == [pill_radius; 4]
                    && (f64::from(*x) - chart.trading_marker_start()).abs() <= 0.5 =>
                {
                    Some((*x, *y, *w, *h))
                }
                _ => None,
            })
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        assert!(chart.set_trading_hover(cancel_x, cancel_y));
        let tooltip_shown = |chart: &mut ChartEngine| {
            let frame = chart.build_frame();
            let pane_segments = chart.frame_pane_segments(0).unwrap();
            frame.panes[0].main[pane_segments.drawings_end..pane_segments.trading_end]
                .iter()
                .any(|primitive| {
                    matches!(primitive, Prim::Text { text, .. } if text == "Close position")
                })
        };
        // Contact alone shows nothing: the tooltip waits for the host to arm it after its dwell.
        assert!(!tooltip_shown(&mut chart));
        assert!(chart.arm_trading_tooltip());
        assert!(!chart.arm_trading_tooltip(), "arming twice is a no-op");
        let hovered = chart.build_frame();
        let pane_segments = chart.frame_pane_segments(0).unwrap();
        let trading = &hovered.panes[0].main[pane_segments.drawings_end..pane_segments.trading_end];
        assert!(trading.iter().any(
            |primitive| matches!(primitive, Prim::Text { text, .. } if text == "Close position")
        ));
        assert!(trading
            .iter()
            .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == "-12")));
        let hovered_bounds = trading
            .iter()
            .find_map(|primitive| match primitive {
                Prim::RoundRect {
                    x,
                    y,
                    w,
                    h,
                    border_width,
                    radii,
                    ..
                } if *border_width > 0.0
                    && *radii == [pill_radius; 4]
                    && (*w - bounds.2).abs() <= 0.5 =>
                {
                    Some((*x, *y, *w, *h))
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(hovered_bounds, bounds);
    }

    #[test]
    fn quantity_without_host_precision_keeps_fractional_sizes_exact() {
        let chart = chart_with_market();
        assert_eq!(chart.format_trading_quantity(1.0), "1");
        assert_eq!(chart.format_trading_quantity(0.00012), "0.00012");
        assert_eq!(chart.format_trading_quantity(2.5), "2.5");
    }

    #[test]
    fn dedicated_position_buttons_create_fixed_role_protection() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        let line_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        assert_eq!(chart.trading_cursor_at(24.0, line_y), None);
        let stop_x = protection_button_x(&chart, line_y, TradingHitKind::StopLossButton);
        assert!(chart.set_trading_hover(stop_x, line_y));
        assert_eq!(
            chart.trading_cursor_at(stop_x, line_y),
            Some(TradingCursor::Pointer)
        );

        // The SL handle creates only a stop loss.
        let below = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 99.0)
            .unwrap();
        assert!(chart.trading_drag_start_at(stop_x, line_y));
        assert!(chart.trading_drag_to(below));
        let intent = chart.trading_drag_end().expect("stop-loss intent");
        assert_eq!(intent.action, TradingIntentAction::CreateStopLoss);
        assert_eq!(intent.role, Some(OrderRole::StopLoss));
        assert_eq!(intent.kind, Some(OrderKind::Stop));
        assert_eq!(intent.side, Some(OrderSide::Sell));
        assert_eq!(intent.price, Some(99.0));
        assert_eq!(intent.position_id.as_ref().unwrap().as_str(), "position-1");
        chart.resolve_trading_intent(intent.sequence, true);

        // Once the stop exists, only the TP handle remains and keeps its explicit role.
        let mut stop = order("position-stop", OrderRole::StopLoss, 99.0);
        stop.position_id = Some(id("position-1", PositionId::new));
        chart.update_working_order(stop).unwrap();
        chart.build_frame();
        let above = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 103.0)
            .unwrap();
        let take_profit_x = protection_button_x(&chart, line_y, TradingHitKind::TakeProfitButton);
        assert!(chart.trading_drag_start_at(take_profit_x, line_y));
        assert!(chart.trading_drag_to(above));
        let intent = chart.trading_drag_end().expect("take-profit intent");
        assert_eq!(intent.action, TradingIntentAction::CreateTakeProfit);
        assert_eq!(intent.kind, Some(OrderKind::Limit));
        chart.resolve_trading_intent(intent.sequence, true);

        assert!(!(0..=(chart.pane_w * 2.0) as usize).any(|step| chart
            .trading_hit_at(step as f64 / 2.0, line_y)
            .is_some_and(|hit| hit.kind == TradingHitKind::StopLossButton)));

        // With both protections attached, the position line has nothing left to create.
        let mut target = order("position-target", OrderRole::TakeProfit, 103.0);
        target.position_id = Some(id("position-1", PositionId::new));
        chart.update_working_order(target).unwrap();
        chart.build_frame();
        assert!(!chart.trading_drag_start_at(24.0, line_y));
    }

    #[test]
    fn close_control_terminates_the_marker_and_protection_buttons_are_separate() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        let line_y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        assert!(matches!(
            chart.trading_hit_at(cancel_x, cancel_y),
            Some(TradingHit {
                kind: TradingHitKind::CancelButton,
                ..
            })
        ));
        assert!(chart
            .trading_hit_at_with_profile(cancel_x, cancel_y + 18.0, HitProfile::PRECISION)
            .is_none());
        assert!(matches!(
            chart.trading_hit_at_with_profile(cancel_x, cancel_y + 18.0, HitProfile::TOUCH),
            Some(TradingHit {
                kind: TradingHitKind::CancelButton,
                ..
            })
        ));
        assert!(matches!(
            chart.trading_hit_at(20.0, line_y),
            Some(TradingHit {
                kind: TradingHitKind::PositionLine,
                ..
            })
        ));

        let marker = |chart: &mut ChartEngine| {
            let frame = chart.build_frame();
            let segments = chart.frame_pane_segments(0).unwrap();
            frame.panes[0].main[segments.drawings_end..segments.trading_end].to_vec()
        };
        let trading = marker(&mut chart);
        let pill_radius = (chart.trading_control_height() / 2.0)
            .min(aeris_charts_core::style::RADIUS_LARGE) as f32;
        let (container_x, container_y, container_w, container_h) = trading
            .iter()
            .find_map(|primitive| match primitive {
                Prim::RoundRect {
                    x,
                    y,
                    w,
                    h,
                    border_width,
                    radii,
                    ..
                } if *border_width > 0.0
                    && *radii == [pill_radius; 4]
                    && (f64::from(*x) - chart.trading_marker_start()).abs() <= 0.5 =>
                {
                    Some((*x, *y, *w, *h))
                }
                _ => None,
            })
            .expect("marker container");
        // The close cell is integrated at the container's right edge, immediately after PnL.
        let close_left = f64::from(container_x + container_w) - chart.trading_close_width();
        let close_surface_left = close_left + 3.0;
        assert!((cancel_x - (close_left + chart.trading_close_width() / 2.0)).abs() <= 0.5);
        assert!((f64::from(container_x) - chart.trading_marker_start()).abs() <= 0.5);
        // At rest the integrated close cell paints no surface of its own.
        assert!(!trading.iter().any(|primitive| matches!(
            primitive,
            Prim::RoundRect { x, border_width, .. }
                if *border_width == 0.0 && (f64::from(*x) - close_surface_left).abs() <= 0.5
        )));
        // The solid quantity cell keeps a deliberate surface gap inside the outline, with a
        // concentric rounded end.
        let quantity_fill = chart.trading_position_color(PositionSide::Long).solid();
        let (qx, qy, qw, qh, qradii) = trading
            .iter()
            .find_map(|primitive| match primitive {
                Prim::RoundRect {
                    x,
                    y,
                    w,
                    h,
                    radii,
                    fill,
                    ..
                } if *fill == quantity_fill => Some((*x, *y, *w, *h, *radii)),
                _ => None,
            })
            .expect("filled quantity cell");
        // At DPR 1: the 1px inside border, then the 1.5px gap rounded to 2 device px.
        assert_eq!(qx, container_x + 3.0);
        assert_eq!(qy, container_y + 3.0);
        assert_eq!(qy + qh, container_y + container_h - 3.0);
        assert!(qx + qw < close_left as f32);
        assert_eq!(qradii, [qh / 2.0, 0.0, 0.0, qh / 2.0]);
        assert!(trading.iter().any(|primitive| matches!(
            primitive,
            Prim::RoundRect { border_width, border_color, .. }
                if *border_width == 1.0
                    && *border_color == chart.trading_position_color(PositionSide::Long)
        )));

        for expected in ["TP", "SL"] {
            assert!(trading
                .iter()
                .any(|primitive| matches!(primitive, Prim::Text { text, .. } if text == expected)));
        }
        // The close icon is stroked geometry — never a font glyph the host's `font_family`
        // might not carry.
        assert!(trading.iter().all(|primitive| !matches!(
            primitive,
            Prim::Text { text, .. } if matches!(text.as_str(), "×" | "✕" | "↕")
        )));
        // The close icon is two anti-aliased strokes in the marker's semantic color. Separate
        // triangles and cap discs rendered unevenly across executors.
        let line_color = chart.trading_position_color(PositionSide::Long);
        assert!(!trading
            .iter()
            .any(|primitive| matches!(primitive, Prim::Triangle { .. } | Prim::Circle { .. })));
        let frame = chart.build_frame();
        let icon_strokes = trading
            .iter()
            .filter_map(|primitive| match primitive {
                Prim::Polyline {
                    first_point,
                    point_count: 2,
                    color,
                    ..
                } if *color == line_color => Some(*first_point as usize),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(icon_strokes.len(), 2, "the close icon has two strokes");
        for first in icon_strokes {
            for [x, y] in &frame.panes[0].points[first..first + 2] {
                assert!(
                    (f64::from(*x) - cancel_x).abs() <= 4.5
                        && (f64::from(*y) - cancel_y).abs() <= 4.5,
                    "the close glyph must stay compact inside its hover surface"
                );
            }
        }
        assert!(trading.iter().any(|primitive| matches!(
            primitive,
            Prim::Text { text, weight, .. } if text == "12" && *weight == 400
        )));

        // At rest the quantity cell is the only borderless fill under the outlined container.
        assert_eq!(
            trading
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::RoundRect { border_width, .. } if *border_width == 0.0
                ))
                .count(),
            1
        );
        let close_fill = |primitives: &[Prim]| {
            primitives.iter().find_map(|primitive| match primitive {
                Prim::RoundRect {
                    x,
                    fill,
                    border_width,
                    ..
                } if *border_width == 0.0 && (f64::from(*x) - close_surface_left).abs() <= 0.5 => {
                    Some(*fill)
                }
                _ => None,
            })
        };
        assert_eq!(close_fill(&trading), None);

        assert!(chart.set_trading_hover(cancel_x, cancel_y));
        let hover_fill = close_fill(&marker(&mut chart)).expect("hover surface");

        assert!(chart.set_trading_pressed(cancel_x, cancel_y));
        let pressed_fill = close_fill(&marker(&mut chart)).expect("pressed surface");
        assert_ne!(pressed_fill, hover_fill);
        assert!(chart.clear_trading_pressed());
    }

    #[test]
    fn close_control_requires_the_press_to_start_on_that_control() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        let (start_x, end_x) = (0..=(chart.pane_w * 2.0) as usize)
            .map(|step| step as f64 / 2.0)
            .find_map(|x| {
                let before = chart.trading_hit_at(x, y)?;
                let after = chart.trading_hit_at(x + 1.0, y)?;
                (before.object == after.object
                    && before.kind != TradingHitKind::CancelButton
                    && after.kind == TradingHitKind::CancelButton)
                    .then_some((x, x + 1.0))
            })
            .expect("close control has an adjacent press surface");
        chart.input_pointer_down(
            PointerInput {
                x: start_x,
                y,
                ..PointerInput::default()
            },
            1,
        );
        chart.input_pointer_up(PointerInput {
            x: end_x,
            y,
            ..PointerInput::default()
        });
        assert_eq!(chart.trading_snapshot().positions.len(), 1);
        assert!(chart.take_trading_intents().is_empty());
    }

    #[test]
    fn trading_tooltip_dwell_does_not_restart_during_a_held_drag() {
        let mut chart = chart_with_market();
        chart
            .update_trading_position(position(PositionSide::Long))
            .unwrap();
        chart.build_frame();
        let (close_x, y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        let start_x = (0..=(close_x * 2.0) as usize)
            .map(|step| step as f64 / 2.0)
            .rev()
            .find(|&x| {
                chart.trading_hit_at(x, y).is_some_and(|hit| {
                    hit.kind != TradingHitKind::CancelButton
                        && hit.object
                            == TradingObjectId::Position(id("position-1", PositionId::new))
                })
            })
            .expect("adjacent marker press surface");
        let pointer = |x, timestamp_ms| PointerInput {
            x,
            y,
            timestamp_ms,
            ..PointerInput::default()
        };
        chart.input_pointer_move(pointer(close_x, 100.0), false);
        assert_eq!(
            chart.input_wake_deadline_ms(),
            Some(100.0 + crate::chart_input::TRADING_TOOLTIP_DWELL_MS)
        );
        chart.input_pointer_down(pointer(start_x, 200.0), 1);
        assert_eq!(chart.input_wake_deadline_ms(), None);
        chart.input_pointer_move(pointer(close_x, 210.0), true);
        assert_eq!(chart.input_wake_deadline_ms(), None);
        chart.input_tick(1_000.0);
        assert!(!chart.trading_state.tooltip_armed);
    }

    #[test]
    fn visible_account_filter_is_host_owned_and_hit_tested() {
        let mut chart = chart_with_market();
        let account_a = AccountId::new("account-a").unwrap();
        let account_b = AccountId::new("account-b").unwrap();
        let mut first = position(PositionSide::Long);
        first.account_id = Some(account_a.clone());
        let mut second = position(PositionSide::Short);
        second.id = PositionId::new("position-2").unwrap();
        second.account_id = Some(account_b);
        second.average_price = 99.0;
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![first, second],
                ..TradingSnapshot::default()
            })
            .unwrap();
        chart.set_trading_visible_account(Some(account_a));
        chart.build_frame();
        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        assert!(chart
            .trading_hit_at(chart.trading_marker_start() + 10.0, y)
            .is_some());
        chart.set_trading_visible_account(Some(AccountId::new("account-b").unwrap()));
        assert!(chart
            .trading_hit_at(chart.trading_marker_start() + 10.0, y)
            .is_none());
        assert_eq!(chart.trading_snapshot().positions.len(), 2);
    }

    #[test]
    fn trading_intents_expose_exact_tick_indices_when_metadata_is_available() {
        let mut chart = chart_with_market();
        chart
            .set_instrument_metadata(InstrumentMetadata {
                tick_size: Some(0.25),
                ..InstrumentMetadata::default()
            })
            .unwrap();
        assert_eq!(chart.trading_price_tick_index(102.5), Some(410));
        assert_eq!(chart.trading_price_tick_index(102.6), None);
    }

    /// Bordered trading chrome (markers, buttons, the action tooltip, annotation chips) must put
    /// every edge on a whole device pixel. A `RoundRect` border paints inside the box, so a
    /// fractional edge smears the 1 px border over two pixel rows on some sides only.
    #[test]
    fn bordered_trading_chrome_and_tooltips_land_on_whole_device_pixels() {
        let mut chart = chart_with_market();
        let mut value = position(PositionSide::Long);
        value.annotations = ["Q 12", "slipped 3", "w"]
            .iter()
            .enumerate()
            .map(|(index, text)| TradingAnnotation {
                id: format!("a-{index}"),
                text: text.to_string(),
                tone: TradingAnnotationTone::Info,
                tooltip: None,
                placement: [
                    TradingAnnotationPlacement::Above,
                    TradingAnnotationPlacement::Inline,
                    TradingAnnotationPlacement::Below,
                ][index],
            })
            .collect();
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![value],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let (cancel_x, cancel_y) = cancel_center(
            &mut chart,
            TradingObjectId::Position(id("position-1", PositionId::new)),
        );
        assert!(chart.set_trading_hover(cancel_x, cancel_y));
        assert!(chart.arm_trading_tooltip());
        for ratio in [1.0_f64, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let mut out = Vec::new();
            let mut regions = Vec::new();
            chart.build_trading_frame_for_test(0, ratio, ratio, &mut regions, &mut out);
            let mut boxes = 0;
            let mut tooltip = false;
            for primitive in &out {
                match primitive {
                    Prim::RoundRect {
                        x,
                        y,
                        w,
                        h,
                        border_width,
                        ..
                    } if *border_width > 0.0 => {
                        boxes += 1;
                        for edge in [*x, *y, *x + *w, *y + *h] {
                            assert_eq!(
                                edge.fract(),
                                0.0,
                                "bordered box {x},{y} {w}x{h} at ratio {ratio}"
                            );
                        }
                    }
                    Prim::Text { text, .. } if text == "Close position" => tooltip = true,
                    _ => {}
                }
            }
            assert!(tooltip, "the armed action tooltip renders at ratio {ratio}");
            assert!(
                boxes >= 5,
                "marker, annotations and tooltip at ratio {ratio}"
            );
        }
    }

    #[test]
    fn annotations_are_bounded_rendered_and_hit_tested_atomically() {
        let mut chart = chart_with_market();
        let mut value = position(PositionSide::Long);
        value.annotations = vec![TradingAnnotation {
            id: "queue".to_string(),
            text: "Q 12".to_string(),
            tone: TradingAnnotationTone::Info,
            tooltip: Some("queue position".to_string()),
            placement: TradingAnnotationPlacement::Inline,
        }];
        chart
            .set_trading_snapshot(TradingSnapshot {
                positions: vec![value],
                ..TradingSnapshot::default()
            })
            .unwrap();
        let before = chart.trading_snapshot();
        let mut invalid = before.clone();
        invalid.positions[0].annotations = (0..=MAX_TRADING_ANNOTATIONS)
            .map(|index| TradingAnnotation {
                id: format!("a-{index}"),
                text: "warning".to_string(),
                ..TradingAnnotation::default()
            })
            .collect();
        assert!(chart.set_trading_snapshot(invalid).is_err());
        assert_eq!(chart.trading_snapshot(), before);
        chart.build_frame();
        let y = chart
            .trading_price_coordinate(0, TradingPriceScale::Right, 101.0)
            .unwrap();
        assert!(matches!(
            chart.trading_hit_at(chart.trading_marker_start() + 12.0, y),
            Some(TradingHit {
                kind: TradingHitKind::Annotation,
                annotation_id: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn host_overlay_and_round_trip_layers_are_runtime_only() {
        let mut chart = chart_with_market();
        let time = chart.series_data(0)[2].time;
        chart
            .set_host_overlay(HostOverlaySnapshot {
                events: vec![HostEventMarker {
                    id: "release".to_string(),
                    time,
                    importance: 2,
                    label: "CPI".to_string(),
                    icon: None,
                }],
                windows: vec![
                    HostTimeWindow {
                        id: "risk".to_string(),
                        start_time: 10,
                        end_time: time + 10,
                        label: "risk".to_string(),
                    },
                    HostTimeWindow {
                        id: "future-risk".to_string(),
                        start_time: time,
                        end_time: time + 10,
                        label: "future risk".to_string(),
                    },
                ],
            })
            .unwrap();
        chart.build_frame();
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            Prim::VLine {
                style: aeris_charts_render::draw_list::LineStyle::Dotted,
                ..
            }
        )));
        assert!(chart
            .host_event_hit_at(chart.time_scale.index_to_coordinate(2), 20.0)
            .is_some());
        assert!(!chart.export_state_json().unwrap().contains("release"));

        chart.set_replay_clock_micros(Some(20_000_000)).unwrap();
        chart.build_frame();
        let segments = chart.frame_pane_segments(0).unwrap();
        let frame = chart.build_frame();
        assert_eq!(
            frame.panes[0].main[segments.series_end..segments.trading_regions_end]
                .iter()
                .filter(|primitive| matches!(primitive, Prim::Rect { .. }))
                .count(),
            1,
            "a crossing host window is clipped and a wholly future window stays hidden"
        );
        assert_eq!(
            frame.panes[0].main[segments.trading_regions_end..segments.trading_end]
                .iter()
                .filter(|primitive| matches!(
                    primitive,
                    Prim::VLine {
                        style: aeris_charts_render::draw_list::LineStyle::Dotted,
                        ..
                    }
                ))
                .count(),
            0,
            "future release markers stay hidden behind the replay clock"
        );
        assert!(chart
            .host_event_hit_at(chart.time_scale.index_to_coordinate(2), 20.0)
            .is_none());
        assert_eq!(
            chart
                .host_event_hit_at(chart.time_scale.index_to_coordinate(0), 20.0)
                .map(|hit| hit.id),
            Some("risk".to_string())
        );
    }

    #[test]
    fn sync_events_are_semantic_and_external_application_does_not_echo() {
        let mut chart = chart_with_market();
        chart.fit_content();
        chart.build_frame();
        let time = chart.series_data(0)[1].time as f64;
        assert!(chart.set_crosshair_position(102.0, time, 0));
        let events = chart.take_sync_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "local");
        assert!(!chart.apply_external_crosshair(Some(CrosshairSyncPosition {
            time,
            price: 102.0,
            pane_index: 0,
        })));
        assert!(chart.take_sync_events().is_empty());
    }

    #[test]
    fn external_sync_application_preserves_pending_local_events() {
        let mut chart = chart_with_market();
        chart.fit_content();
        chart.build_frame();
        let data = chart.series_data(0);
        let time = data[1].time as f64;
        assert!(chart.set_crosshair_position(102.0, time, 0));

        let range = crate::VisibleTimeRangeSync {
            from: data[0].time as f64,
            to: data[1].time as f64,
        };
        assert!(
            chart.apply_external_sync_event(&crate::ChartSyncEventKind::VisibleTimeRange { range })
        );

        let events = chart.take_sync_events();
        assert_eq!(
            events.len(),
            1,
            "external application must not drain local work"
        );
        assert!(matches!(
            events[0].kind,
            crate::ChartSyncEventKind::Crosshair { .. }
        ));
    }
}
