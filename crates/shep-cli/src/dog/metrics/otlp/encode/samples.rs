//! The channel samples a sheep or a dog sent on its shepherd channel, as
//! `metrics[]` entries: the same families
//! [`crate::dog::metrics::exposition::render`] appends to a scrape, in the
//! OTLP shape.
//!
//! The sender's kind decides `gauge` or `sum`, its help text is the
//! `description`, and its labels are attributes in the order the store built
//! them. Nothing here decides where in the payload the entries land; that is
//! [`super`].

use shep_core::protocol::MetricType;

use crate::dog::metrics::grammar::REFUSED_TOTAL;
use crate::dog::metrics::samples::{SampleFamily, SamplePoint};

use super::payload::{
    KeyValue, Metric, Nanos, attr, cumulative_count_point, cumulative_double_point, cumulative_sum,
    double_point, gauge,
};

/// One sample point's attributes: the sender's own labels, then the sample's
/// own, in the order the store built them.
fn sample_attributes<'a>(point: &'a SamplePoint) -> Vec<KeyValue<'a>> {
    point
        .labels
        .iter()
        .map(|(key, value)| attr(key, value.as_str()))
        .collect()
}

/// The samples OTLP can carry a number for.
///
/// The text exposition has its own spelling for a non-finite value (`NaN`,
/// `+Inf`) and OTLP/JSON has none, so the point is dropped rather than
/// encoded, the same rule a non-finite `cpu_percent` follows.
fn finite(points: &[SamplePoint]) -> impl Iterator<Item = &SamplePoint> {
    points.iter().filter(|point| point.value.is_finite())
}

/// One `metrics[]` entry per [`SampleFamily`], in the store's own name order,
/// the same order `render` appends them in.
///
/// A counter's `startTimeUnixNano` is the exporter's own start, the stamp
/// every other cumulative sum in this payload carries. The store keeps the
/// time a retained value last arrived and never the time it first did, and a
/// start that moved with every sample would tell a collector the counter had
/// only been growing for a moment.
pub(super) fn sample_metrics<'a>(
    samples: &'a [SampleFamily],
    start: Nanos,
    now: Nanos,
) -> Vec<Metric<'a>> {
    samples
        .iter()
        .filter_map(|family| match family.kind {
            MetricType::Gauge => gauge(
                family.name.as_str(),
                family.help.as_deref(),
                finite(&family.points)
                    .map(|point| double_point(sample_attributes(point), now, point.value))
                    .collect(),
            ),
            MetricType::Counter => cumulative_sum(
                family.name.as_str(),
                family.help.as_deref(),
                finite(&family.points)
                    .map(|point| {
                        let attributes = sample_attributes(point);
                        // The dog's own refusal counter, named by the store
                        // rather than by a sender, and a count rather than a
                        // measurement. A sender cannot take this name: the
                        // grammar refuses it as reserved.
                        if family.name == REFUSED_TOTAL {
                            cumulative_count_point(attributes, start, now, point.value)
                        } else {
                            cumulative_double_point(attributes, start, now, point.value)
                        }
                    })
                    .collect(),
            ),
        })
        .collect()
}
