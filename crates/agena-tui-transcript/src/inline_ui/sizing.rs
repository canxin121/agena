//! Terminal-cell sizing, separate from painting. A grid has equal tracks; a row
//! combines compact labels/actions with flexible content. No pixels or new wire
//! properties are introduced, and measuring never renders or mutates a control.
use super::*;
use agena_domain::{UiKind, UiResolvedNode};

#[derive(Clone, Copy)]
pub(super) struct Measure {
    pub minimum: u16,
    pub preferred: u16,
    pub grow: bool,
}
fn cells(value: &str) -> u16 {
    crate::sanitize_terminal_text(value)
        .lines()
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
        .min(usize::from(u16::MAX)) as u16
}
fn property(node: &UiResolvedNode, key: &str) -> u16 {
    node.props
        .get(key)
        .map(display)
        .map_or(0, |value| cells(&value))
}
fn compact(length: u16, minimum: u16) -> Measure {
    Measure {
        minimum: length.min(minimum).max(1),
        preferred: length.max(1),
        grow: false,
    }
}
fn flexible(minimum: u16, preferred: u16) -> Measure {
    Measure {
        minimum: minimum.max(1),
        preferred: preferred.max(minimum).max(1),
        grow: true,
    }
}
pub(super) fn measure(node: &UiResolvedNode) -> Measure {
    use UiKind::*;
    let label = property(node, "label");
    match node.kind {
        Text | Heading => {
            let length = property(node, "text").saturating_add(u16::from(node.kind == Heading) * 2);
            if length > 48 {
                flexible(24, length.min(96))
            } else {
                compact(length, 24)
            }
        }
        Badge => compact(property(node, "text").saturating_add(2), 24),
        Button => compact(label.saturating_add(4), 24),
        Checkbox | Switch => {
            let length = label
                .saturating_add(if node.kind == Switch { 9 } else { 4 })
                .max(property(node, "description"));
            compact(length, 28)
        }
        Tabs => {
            let length = node.field.as_ref().map_or(label, |field| {
                field.options.iter().fold(0u16, |sum, option| {
                    sum.saturating_add(cells(&option.label)).saturating_add(5)
                })
            });
            compact(length, 24)
        }
        Metric => {
            let value = property(node, "value")
                .saturating_add(property(node, "prefix"))
                .saturating_add(property(node, "suffix"));
            let core = label.max(value);
            flexible(
                core.clamp(14, 24).saturating_add(4),
                core.max(property(node, "detail"))
                    .clamp(18, 44)
                    .saturating_add(4),
            )
        }
        Card | Stack | Row | Grid => {
            let children = node.children.iter().map(measure).collect::<Vec<_>>();
            let inset = if node.kind == Card { 4 } else { 0 };
            let minimum = children
                .iter()
                .map(|m| m.minimum)
                .max()
                .unwrap_or(1)
                .max(property(node, "title").min(24))
                .saturating_add(inset);
            let preferred = if node.kind == Row {
                children
                    .iter()
                    .fold(0u16, |sum, m| {
                        sum.saturating_add(m.preferred).saturating_add(2)
                    })
                    .saturating_sub(2)
            } else if node.kind == Grid {
                let columns = node
                    .props
                    .get("columns")
                    .and_then(Value::as_u64)
                    .unwrap_or(2)
                    .clamp(1, 4) as u16;
                children
                    .iter()
                    .map(|m| m.preferred)
                    .max()
                    .unwrap_or(1)
                    .saturating_mul(columns)
                    .saturating_add(columns - 1)
            } else {
                children.iter().map(|m| m.preferred).max().unwrap_or(1)
            }
            .max(property(node, "title"))
            .saturating_add(inset);
            flexible(minimum, preferred)
        }
        Slider | Progress => flexible(24, label.saturating_add(12).max(40)),
        Number | Input => flexible(
            18,
            label
                .max(cells(&display(&node.value)))
                .saturating_add(6)
                .clamp(24, 48),
        ),
        Select => {
            let options = node
                .field
                .as_ref()
                .and_then(|field| {
                    field
                        .options
                        .iter()
                        .map(|option| cells(&option.label))
                        .max()
                })
                .unwrap_or(0);
            flexible(
                label.max(options).saturating_add(6).clamp(18, 28),
                label.max(options).saturating_add(6).clamp(24, 48),
            )
        }
        Chart => flexible(32, 64),
        Table => {
            let count = node
                .props
                .get("columns")
                .and_then(Value::as_array)
                .map_or(1, Vec::len) as u16;
            flexible(
                count
                    .saturating_mul(8)
                    .saturating_add(count.saturating_sub(1))
                    .clamp(24, 56),
                64,
            )
        }
        Tasks | Timeline | Details | Callout => flexible(24, 48),
        Divider => flexible(1, 1),
    }
}

/// Allocate minima first, then content demand, then spare width to flexible
/// cells. Cumulative integer rounding keeps the sum exact at every column count.
pub(super) fn allocate(measures: &[Measure], width: u16, gap: u16) -> Vec<u16> {
    if measures.is_empty() {
        return Vec::new();
    }
    if measures.len() == 1 {
        return vec![if measures[0].grow {
            width
        } else {
            width.min(measures[0].preferred)
        }];
    }
    let available = usize::from(width).saturating_sub((measures.len() - 1) * usize::from(gap));
    let minimum = measures
        .iter()
        .map(|m| usize::from(m.minimum))
        .collect::<Vec<_>>();
    let total = minimum.iter().sum::<usize>();
    if total > available {
        return (0..measures.len())
            .map(|index| {
                (available / measures.len() + usize::from(index < available % measures.len()))
                    as u16
            })
            .collect();
    }
    let mut sizes = minimum;
    let demand = measures
        .iter()
        .map(|m| usize::from(m.preferred.saturating_sub(m.minimum)))
        .collect::<Vec<_>>();
    let demand_sum = demand.iter().sum::<usize>();
    let growth = (available - total).min(demand_sum);
    distribute(&mut sizes, &demand, growth);
    let remainder = available - sizes.iter().sum::<usize>();
    let flexible = measures
        .iter()
        .map(|m| usize::from(m.grow))
        .collect::<Vec<_>>();
    distribute(&mut sizes, &flexible, remainder);
    sizes.into_iter().map(|size| size as u16).collect()
}
fn distribute(sizes: &mut [usize], weights: &[usize], amount: usize) {
    let total = weights.iter().sum::<usize>();
    if total == 0 || amount == 0 {
        return;
    }
    let mut cumulative = 0;
    let mut assigned = 0;
    for (size, weight) in sizes.iter_mut().zip(weights) {
        cumulative += weight;
        let next = amount * cumulative / total;
        *size += next - assigned;
        assigned = next;
    }
}
