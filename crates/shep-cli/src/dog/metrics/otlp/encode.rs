//! The OTLP/JSON mapping for a [`Reading`]: the same series
//! [`crate::dog::metrics::exposition::render`] serves, in the shape a
//! collector reads, the channel samples included.
//!
//! A sample lands after every built-in group, in the store's own name order,
//! the way `render` appends them: the sender's kind decides `gauge` or
//! `sum`, its help text is the `description`, and its labels are attributes
//! in the order the store built them.
//!
//! The two renderers share no code on purpose. `render` is allocation-tuned
//! (one escaped label prefix per sheep, shared by its six status series) and
//! a neutral intermediate model would spend allocations on every scrape to
//! serve a path that is not the scrape. The drift guard is
//! `the_payload_names_every_series_the_exposition_names` in this module's
//! tests, which compares names, types, descriptions, point counts and every
//! series' own labels between the two, over a reading that carries channel
//! samples as well as a flock.
//!
//! Where the mapping differs from plain serde: 64-bit integers are decimal
//! strings ([`Nanos`], [`IntStr`]), doubles are numbers, and
//! `aggregationTemporality` is the integer `2` rather than a name.

use std::borrow::Cow;

use serde::Serialize;
use shep_core::protocol::MetricType;
use shep_core::status::ProcStatus;

use crate::dog::metrics::Reading;
use crate::dog::metrics::exposition::{ALL_STATUSES, status_label};
use crate::dog::metrics::grammar::REFUSED_TOTAL;
use crate::dog::metrics::samples::{SampleFamily, SamplePoint};

/// `service.name`, the resource attribute a collector groups every series
/// under.
const SERVICE_NAME: &str = "shep";

/// `scope.name`: the instrumentation that produced these points, which is
/// this dog, not the shepherd that answered the listing.
const SCOPE_NAME: &str = "shep.metrics";

/// `aggregationTemporality.CUMULATIVE`, the only OTLP enum this mapping
/// writes, and the one its JSON form carries as an integer.
const TEMPORALITY_CUMULATIVE: u8 = 2;

/// What the encoder stamps its points with, passed in rather than read from
/// the clock, so an exact-payload test can pin the timestamps.
#[derive(Debug, Clone, Copy)]
pub struct ExportContext<'a> {
    /// Nanoseconds since the Unix epoch, stamped as every point's
    /// `timeUnixNano`.
    pub now_unix_nano: u64,
    /// Nanoseconds since the Unix epoch, when this exporter started: the
    /// cumulative sum's `startTimeUnixNano`, so a collector can difference
    /// two exports and get the restarts inside the interval.
    pub exporter_start_unix_nano: u64,
    /// `host.name`, when the OS reports one. `None` leaves the attribute
    /// out rather than sending an empty one.
    pub host_name: Option<&'a str>,
}

/// A 64-bit nanosecond timestamp, on the wire as a decimal string: OTLP's
/// JSON mapping spells `timeUnixNano` and `startTimeUnixNano` as strings
/// because a 64-bit count of nanos is not exact as a JSON number.
#[derive(Debug, Clone, Copy)]
struct Nanos(u64);

impl Serialize for Nanos {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

/// A 64-bit integer point value, on the wire as a decimal string, for the
/// same reason as [`Nanos`].
#[derive(Debug, Clone, Copy)]
struct IntStr(u64);

impl Serialize for IntStr {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

/// `resourceMetrics[]`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest<'a> {
    resource_metrics: Vec<ResourceMetrics<'a>>,
}

/// One `resourceMetrics` entry: what produced the telemetry, and the
/// instrumentation that produced it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceMetrics<'a> {
    resource: Resource<'a>,
    scope_metrics: Vec<ScopeMetrics<'a>>,
}

/// `resource.attributes`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Resource<'a> {
    attributes: Vec<KeyValue<'a>>,
}

/// `scopeMetrics[]`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopeMetrics<'a> {
    scope: Scope<'a>,
    metrics: Vec<Metric<'a>>,
}

/// `scope{name,version}`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Scope<'a> {
    name: &'a str,
    version: &'a str,
}

/// `metrics[]`. Exactly one of `gauge` or `sum` is set, which is how OTLP
/// spells a metric's type in JSON.
///
/// `name` and `description` are `Cow` because a channel sample's are
/// app-supplied text the store holds as `String`, while every built-in
/// series names itself with a `&'static str`.
///
/// `unit` is left out entirely. A unit is what makes a Prometheus-side
/// collector append `_bytes` to a name that already ends in it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metric<'a> {
    name: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gauge: Option<Gauge<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sum: Option<Sum<'a>>,
}

/// `gauge{dataPoints}`: a value read at an instant.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Gauge<'a> {
    data_points: Vec<DataPoint<'a>>,
}

/// `sum{dataPoints,aggregationTemporality,isMonotonic}`: a count that only
/// ever grows, reset only when this dog restarts.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sum<'a> {
    data_points: Vec<DataPoint<'a>>,
    aggregation_temporality: u8,
    is_monotonic: bool,
}

/// One `dataPoints[]` entry. Exactly one of `as_int` or `as_double` is set.
///
/// `startTimeUnixNano` is present on the cumulative sum and absent on every
/// gauge: OTLP gives a gauge no start, and a sum without one has nothing to
/// difference against.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DataPoint<'a> {
    attributes: Vec<KeyValue<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_time_unix_nano: Option<Nanos>,
    time_unix_nano: Nanos,
    #[serde(skip_serializing_if = "Option::is_none")]
    as_int: Option<IntStr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    as_double: Option<f64>,
}

/// One attribute: `{key,value}`, every value a string, matching the
/// Prometheus label of the same name.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeyValue<'a> {
    key: &'a str,
    value: StringValue<'a>,
}

/// `value{stringValue}`.
///
/// A `Cow` because a label's value is usually a `&str` borrowed from the
/// reading, and sometimes a `String` this function built: a sheep's numeric
/// id, which no `Reading` field holds as text.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StringValue<'a> {
    string_value: Cow<'a, str>,
}

/// One `{key,value}` pair.
fn attr<'a>(key: &'a str, value: impl Into<Cow<'a, str>>) -> KeyValue<'a> {
    KeyValue {
        key,
        value: StringValue {
            string_value: value.into(),
        },
    }
}

/// One gauge point: an instant's value, with no start time.
fn gauge_point<'a>(attributes: Vec<KeyValue<'a>>, now: Nanos, value: u64) -> DataPoint<'a> {
    DataPoint {
        attributes,
        start_time_unix_nano: None,
        time_unix_nano: now,
        as_int: Some(IntStr(value)),
        as_double: None,
    }
}

/// One gauge point carrying a rate.
fn double_point<'a>(attributes: Vec<KeyValue<'a>>, now: Nanos, value: f64) -> DataPoint<'a> {
    DataPoint {
        attributes,
        start_time_unix_nano: None,
        time_unix_nano: now,
        as_int: None,
        as_double: Some(value),
    }
}

/// One cumulative monotonic sum point, started at the exporter's own start.
fn cumulative_point<'a>(
    attributes: Vec<KeyValue<'a>>,
    start: Nanos,
    now: Nanos,
    value: u64,
) -> DataPoint<'a> {
    DataPoint {
        attributes,
        start_time_unix_nano: Some(start),
        time_unix_nano: now,
        as_int: Some(IntStr(value)),
        as_double: None,
    }
}

/// One cumulative monotonic sum point carrying a channel sample's `f64`.
fn cumulative_double_point<'a>(
    attributes: Vec<KeyValue<'a>>,
    start: Nanos,
    now: Nanos,
    value: f64,
) -> DataPoint<'a> {
    DataPoint {
        attributes,
        start_time_unix_nano: Some(start),
        time_unix_nano: now,
        as_int: None,
        as_double: Some(value),
    }
}

/// One cumulative monotonic sum point carrying a count.
///
/// The value reached the store as an `f64`, because that is what the wire
/// carries, but a count was a `u64` before it was one, and `asInt` is what
/// the built-in counters use for the same reason.
fn cumulative_count_point<'a>(
    attributes: Vec<KeyValue<'a>>,
    start: Nanos,
    now: Nanos,
    value: f64,
) -> DataPoint<'a> {
    DataPoint {
        attributes,
        start_time_unix_nano: Some(start),
        time_unix_nano: now,
        // A count this dog only ever grew from 0, so a negative or
        // over-large value cannot come from it; saturating is what keeps one
        // a big number rather than a wrapped small one.
        as_int: Some(IntStr(value.max(0.0) as u64)),
        as_double: None,
    }
}

/// A `metrics[]` gauge entry, or nothing at all when it has no points, which
/// is how `render` drops a group with no series.
///
/// `description` is `Option` because a channel sample carries help text only
/// if its sender wrote any, and `render` writes no `# HELP` line for a family
/// that has none. Every built-in series names its own.
fn gauge<'a>(
    name: impl Into<Cow<'a, str>>,
    description: Option<impl Into<Cow<'a, str>>>,
    data_points: Vec<DataPoint<'a>>,
) -> Option<Metric<'a>> {
    (!data_points.is_empty()).then_some(Metric {
        name: name.into(),
        description: description.map(Into::into),
        gauge: Some(Gauge { data_points }),
        sum: None,
    })
}

/// A `metrics[]` cumulative monotonic sum entry, or nothing at all when it
/// has no points.
fn cumulative_sum<'a>(
    name: impl Into<Cow<'a, str>>,
    description: Option<impl Into<Cow<'a, str>>>,
    data_points: Vec<DataPoint<'a>>,
) -> Option<Metric<'a>> {
    (!data_points.is_empty()).then_some(Metric {
        name: name.into(),
        description: description.map(Into::into),
        gauge: None,
        sum: Some(Sum {
            data_points,
            aggregation_temporality: TEMPORALITY_CUMULATIVE,
            is_monotonic: true,
        }),
    })
}

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
fn finite<'a>(points: &'a [SamplePoint]) -> impl Iterator<Item = &'a SamplePoint> {
    points.iter().filter(|point| point.value.is_finite())
}

/// One `metrics[]` entry per [`SampleFamily`], in the store's own name order,
/// after every built-in group. The same rule `render` follows.
///
/// A counter's `startTimeUnixNano` is the exporter's own start, the stamp
/// every other cumulative sum in this payload carries. The store keeps the
/// time a retained value last arrived and never the time it first did, and a
/// start that moved with every sample would tell a collector the counter had
/// only been growing for a moment.
fn sample_metrics<'a>(samples: &'a [SampleFamily], start: Nanos, now: Nanos) -> Vec<Metric<'a>> {
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

/// Encodes `reading` as one OTLP/JSON `ExportMetricsServiceRequest` body.
///
/// Timestamps and `host.name` come from `context`, so the payload is a pure
/// function of its inputs.
///
/// # Panics
///
/// Never in practice: every value here is a string, a `u64`, a finite `f64`
/// or a `bool`, and there is no writer for an I/O failure to come from. A
/// non-finite `cpu_percent` or sample value is dropped rather than encoded,
/// since OTLP has no number for it.
#[track_caller]
pub fn encode(reading: &Reading, context: &ExportContext<'_>) -> String {
    let now = Nanos(context.now_unix_nano);
    let start = Nanos(context.exporter_start_unix_nano);

    let mut cpu = Vec::new();
    let mut memory = Vec::new();
    let mut restarts = Vec::new();
    let mut uptime = Vec::new();
    let mut status = Vec::new();
    let mut dog_up = Vec::new();

    for info in &reading.flock {
        if let Some(source) = &info.dog {
            // The same rule `render` follows: a parked dog exports nothing,
            // so disabling one never reads as it going down.
            if info.disabled == Some(true) {
                continue;
            }
            let attributes = vec![
                attr("dog", info.name.as_str()),
                attr("source", <&'static str>::from(source)),
            ];
            dog_up.push(gauge_point(
                attributes,
                now,
                u64::from(info.status == ProcStatus::Online),
            ));
            continue;
        }

        let id = info.id.to_string();
        let fold = info.fold.as_deref().unwrap_or("");
        // A closure rather than one shared `Vec`, because every series
        // below needs its own copy to move into its own point.
        let sheep = || {
            vec![
                attr("sheep", info.name.as_str()),
                attr("id", id.clone()),
                attr("fold", fold),
            ]
        };

        // `None` covers a sheep that is not running and one the shepherd has
        // not sampled yet, and OTLP has no unknown: a point with no value
        // would read as zero, which `render` is careful never to say.
        if let Some(cpu_percent) = info.cpu_percent.filter(|percent| percent.is_finite()) {
            cpu.push(double_point(sheep(), now, f64::from(cpu_percent)));
        }
        if let Some(memory_bytes) = info.memory_bytes {
            memory.push(gauge_point(sheep(), now, memory_bytes));
        }
        restarts.push(cumulative_point(
            sheep(),
            start,
            now,
            u64::from(info.restarts),
        ));
        uptime.push(gauge_point(sheep(), now, info.uptime_ms / 1000));

        for candidate in ALL_STATUSES {
            let mut attributes = sheep();
            attributes.push(attr("status", status_label(candidate)));
            status.push(gauge_point(
                attributes,
                now,
                u64::from(info.status == candidate),
            ));
        }
    }

    let daemon_up = gauge_point(
        vec![attr("version", reading.daemon_version.as_str())],
        now,
        1,
    );
    let daemon_pid = gauge_point(Vec::new(), now, u64::from(reading.daemon_pid));

    let mut metrics = Vec::new();
    for group in [
        gauge(
            "shep_sheep_cpu_percent",
            Some("Tree CPU as a percentage of one core, over the last sampling window."),
            cpu,
        ),
        gauge(
            "shep_sheep_memory_bytes",
            Some("Tree resident set size in bytes."),
            memory,
        ),
        cumulative_sum(
            "shep_sheep_restart_total",
            Some("Restart count since registration."),
            restarts,
        ),
        gauge(
            "shep_sheep_uptime_seconds",
            Some("Seconds since this sheep's last successful start."),
            uptime,
        ),
        gauge(
            "shep_sheep_status",
            Some("1 for the status this sheep is currently in, 0 for every other status."),
            status,
        ),
        gauge(
            "shep_dog_up",
            Some("1 when this dog is online, 0 otherwise."),
            dog_up,
        ),
        gauge(
            "shep_daemon_up",
            Some("Always 1: the scrape reached the shepherd."),
            vec![daemon_up],
        ),
        gauge(
            "shep_daemon_pid",
            Some("The shepherd's own pid, so a restart is visible as a step change."),
            vec![daemon_pid],
        ),
    ] {
        metrics.extend(group);
    }

    if let Some(host) = &reading.host {
        for group in [
            gauge(
                "shep_host_memory_total_bytes",
                Some("Total physical memory on the host."),
                vec![gauge_point(Vec::new(), now, host.memory_total_bytes)],
            ),
            gauge(
                "shep_host_memory_used_bytes",
                Some("Memory in use on the host, as the platform reports it."),
                vec![gauge_point(Vec::new(), now, host.memory_used_bytes)],
            ),
            gauge(
                "shep_host_processes",
                Some("Number of processes running on the host, the flock included."),
                vec![gauge_point(
                    Vec::new(),
                    now,
                    u64::try_from(host.processes).unwrap_or(u64::MAX),
                )],
            ),
            gauge(
                "shep_host_uptime_seconds",
                Some("Seconds since the host booted."),
                vec![gauge_point(Vec::new(), now, host.uptime_seconds)],
            ),
        ] {
            metrics.extend(group);
        }
    }

    metrics.extend(sample_metrics(&reading.samples, start, now));

    let mut resource = vec![
        attr("service.name", SERVICE_NAME),
        attr("service.version", reading.daemon_version.as_str()),
    ];
    if let Some(host_name) = context.host_name {
        resource.push(attr("host.name", host_name));
    }

    let body = ExportRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Resource {
                attributes: resource,
            },
            scope_metrics: vec![ScopeMetrics {
                scope: Scope {
                    name: SCOPE_NAME,
                    version: &reading.daemon_version,
                },
                metrics,
            }],
        }],
    };
    serde_json::to_string(&body)
        .expect("every value here is a string, an integer, a finite double or a bool")
}

#[cfg(test)]
mod tests {
    use shep_core::protocol::{DogSource, MetricType, ProcessInfo};
    use shep_core::status::ProcStatus;

    use crate::dog::metrics::samples::{SampleFamily, SamplePoint};
    use crate::dog::metrics::{HostReading, Reading, exposition};

    use super::*;

    /// One [`SamplePoint`] with `labels`, which is the store's own shape:
    /// the sender's labels first, the sample's own after them.
    fn point(labels: &[(&str, &str)], value: f64) -> SamplePoint {
        SamplePoint {
            labels: labels
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            value,
        }
    }

    /// One [`SampleFamily`] as [`crate::dog::metrics::samples::SampleStore`]'s
    /// snapshot hands it over.
    fn family(
        name: &str,
        kind: MetricType,
        help: Option<&str>,
        points: Vec<SamplePoint>,
    ) -> SampleFamily {
        SampleFamily {
            name: name.to_string(),
            help: help.map(str::to_string),
            kind,
            points,
        }
    }

    /// A [`Reading`] with `samples` on it and nothing else changed.
    fn with_samples(samples: Vec<SampleFamily>) -> Reading {
        Reading {
            samples,
            ..reading()
        }
    }

    /// One `dataPoints[]` entry's attributes, as `{key, stringValue}` pairs.
    fn attributes_of(point: &serde_json::Value) -> Vec<(&str, &str)> {
        point["attributes"]
            .as_array()
            .expect("attributes")
            .iter()
            .map(|attribute| {
                (
                    attribute["key"].as_str().expect("key"),
                    attribute["value"]["stringValue"]
                        .as_str()
                        .expect("stringValue"),
                )
            })
            .collect()
    }

    /// Timestamps every test below pins, so an exact payload is a string
    /// literal rather than a clock reading.
    const NOW: u64 = 1_700_000_000_000_000_000;
    const START: u64 = 1_699_999_900_000_000_000;

    fn context() -> ExportContext<'static> {
        ExportContext {
            now_unix_nano: NOW,
            exporter_start_unix_nano: START,
            host_name: Some("sheepbox"),
        }
    }

    /// A baseline [`Reading`]: no flock, no host sample. Tests override what
    /// they care about with `..reading()`.
    fn reading() -> Reading {
        Reading {
            flock: vec![],
            daemon_version: "9.9.9".to_string(),
            daemon_pid: 12345,
            samples: vec![],
            host: None,
        }
    }

    /// The `sheep` fixture the exposition's own tests use: id `3`, fold
    /// `backend`, online, with a CPU and memory sample.
    fn sample_info(name: &str) -> ProcessInfo {
        ProcessInfo::builder(3, name, ProcStatus::Online)
            .pid(Some(4242))
            .restarts(2)
            .uptime_ms(65_000)
            .fold(Some("backend".to_string()))
            .cpu_percent(Some(1.5))
            .memory_bytes(Some(2048))
            .build()
    }

    fn payload_with(reading: &Reading, context: &ExportContext<'_>) -> serde_json::Value {
        let text = encode(reading, context);
        serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}: {text}"))
    }

    fn payload(reading: &Reading) -> serde_json::Value {
        payload_with(reading, &context())
    }

    /// `scopeMetrics[0].metrics[]`, keyed by metric name.
    fn metrics_by_name(reading: &Reading) -> serde_json::Map<String, serde_json::Value> {
        payload(reading)
            .pointer("/resourceMetrics/0/scopeMetrics/0/metrics")
            .and_then(serde_json::Value::as_array)
            .expect("the payload carries one scope")
            .clone()
            .into_iter()
            .map(|metric| {
                let name = metric["name"]
                    .as_str()
                    .expect("every metric has a name")
                    .to_owned();
                (name, metric)
            })
            .collect()
    }

    /// One `# HELP`/`# TYPE` group out of the exposition text, with the
    /// series lines under it.
    struct Group {
        name: String,
        help: String,
        kind: String,
        lines: Vec<String>,
    }

    /// The group `name` belongs to, created on whichever of its three lines
    /// arrives first.
    fn group_at<'a>(groups: &'a mut Vec<Group>, name: &str) -> &'a mut Group {
        if !groups.iter().any(|group| group.name == name) {
            groups.push(Group {
                name: name.to_owned(),
                help: String::new(),
                kind: String::new(),
                lines: Vec::new(),
            });
        }
        groups
            .iter_mut()
            .find(|group| group.name == name)
            .expect("just listed")
    }

    /// The exposition text read as groups, which is the half of the drift
    /// guard that has no equivalent in the payload.
    fn groups(text: &str) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("# HELP ") {
                let (name, help) = rest.split_once(' ').expect("a HELP line carries its text");
                group_at(&mut groups, name).help = help.to_owned();
            } else if let Some(rest) = line.strip_prefix("# TYPE ") {
                let (name, kind) = rest.split_once(' ').expect("a TYPE line carries its type");
                group_at(&mut groups, name).kind = kind.to_owned();
            } else if !line.is_empty() {
                let name = line
                    .split(['{', ' '])
                    .next()
                    .expect("a series line starts with its name");
                group_at(&mut groups, name).lines.push(line.to_owned());
            }
        }
        groups
    }

    /// The inverse of the exposition's two escapers: `\\`, `\"` and `\n` are
    /// what `escape_label_value` and `escape_help` write, and the text the
    /// sender wrote is what the payload carries verbatim.
    fn unescape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => break,
                },
                other => out.push(other),
            }
        }
        out
    }

    /// One exposition series line's labels, unescaped and in the order the
    /// line writes them: the pairs the payload has to carry as attributes.
    ///
    /// A quoted value is read to its closing quote, so a `}` inside one does
    /// not end the label list early.
    fn series_labels(line: &str) -> Vec<(String, String)> {
        let Some((_name, rest)) = line.split_once('{') else {
            return Vec::new();
        };
        let mut pairs = Vec::new();
        let mut chars = rest.chars().peekable();
        loop {
            while chars.peek().is_some_and(|ch| *ch == ',') {
                chars.next();
            }
            if chars.peek().copied().is_none_or(|ch| ch == '}') {
                break;
            }
            let mut key = String::new();
            for ch in chars.by_ref().take_while(|ch| *ch != '=') {
                key.push(ch);
            }
            let mut value = String::new();
            if chars.next().is_some_and(|ch| ch == '"') {
                while let Some(ch) = chars.next() {
                    match ch {
                        '\\' => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some(other) => value.push(other),
                            None => break,
                        },
                        '"' => break,
                        other => value.push(other),
                    }
                }
            }
            pairs.push((key, value));
        }
        pairs
    }

    /// The drift guard: every name the exposition emits, with its `# TYPE`,
    /// its `# HELP` text, its series count and every one of its series' own
    /// labels, appears in the OTLP payload as the same kind with the same
    /// points.
    ///
    /// Fails if a series lands in one renderer and not the other, which is
    /// the defect two renderers that share no code grow by default. The
    /// reading carries channel samples beside the flock, because the sample
    /// path is the one that renders nothing at all when nobody writes it, and
    /// a gauge and a counter, help text and none, and the dog's own refusal
    /// counter are all on it.
    #[test]
    fn the_payload_names_every_series_the_exposition_names() {
        let mut reading = exposition::tests::golden_reading();
        reading.samples = vec![
            family(
                "requests_total",
                MetricType::Counter,
                Some("Requests served."),
                vec![
                    point(
                        &[("sheep", "web"), ("id", "3"), ("fold", "backend"), ("route", "/x")],
                        4.0,
                    ),
                    point(&[("dog", "bark"), ("source", "built-in")], 2.0),
                ],
            ),
            family(
                "queue_depth",
                MetricType::Gauge,
                Some("Jobs waiting.\nSecond line, with a backslash \\ in it."),
                vec![point(&[("model", "qwen"), ("note", "a\"b\\x\n")], 1.5)],
            ),
            family(
                "in_flight",
                MetricType::Gauge,
                None,
                vec![point(&[("model", "qwen")], 2.0)],
            ),
            family(
                REFUSED_TOTAL,
                MetricType::Counter,
                Some("Channel samples this dog refused, by reason."),
                vec![
                    point(&[("reason", "invalid_name")], 3.0),
                    point(&[("reason", "type_conflict")], 1.0),
                ],
            ),
        ];
        let text = exposition::render(&reading);
        let metrics = metrics_by_name(&reading);
        let expected = groups(&text);
        assert!(
            !expected.is_empty(),
            "the golden exposition named no metric at all: {text}"
        );
        assert!(
            expected.iter().any(|group| group.name == REFUSED_TOTAL),
            "the fixture put no sample on the reading: {text}"
        );
        assert_eq!(
            metrics.len(),
            expected.len(),
            "the two renderers disagree about how many metrics there are: \
             exposition {:?}, otlp {:?}",
            expected.iter().map(|group| &group.name).collect::<Vec<_>>(),
            metrics.keys().collect::<Vec<_>>(),
        );

        for group in expected {
            let metric = metrics.get(&group.name).unwrap_or_else(|| {
                panic!("{} is in the exposition and not in the payload", group.name)
            });
            let field = match group.kind.as_str() {
                "gauge" => "gauge",
                "counter" => "sum",
                other => panic!("{}: unhandled Prometheus type {other:?}", group.name),
            };
            assert!(
                metric.get(field).is_some(),
                "{}: the payload has no {field} for it",
                group.name
            );
            // An empty `group.help` is the exposition writing no `# HELP`
            // line, which is the payload writing no `description`.
            let description = metric.get("description").and_then(serde_json::Value::as_str);
            assert_eq!(
                description.unwrap_or(""),
                unescape(&group.help),
                "{}: the description drifted from the HELP text",
                group.name
            );
            let points = metric[field]["dataPoints"]
                .as_array()
                .expect("dataPoints");
            assert_eq!(
                points.len(),
                group.lines.len(),
                "{}: point count",
                group.name
            );
            for (line, point) in group.lines.iter().zip(points) {
                let actual: Vec<(String, String)> = attributes_of(point)
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
                    .collect();
                assert_eq!(
                    actual,
                    series_labels(line),
                    "{}: the labels drifted, exposition line {line:?}",
                    group.name
                );
            }
        }
    }

    /// A sample the sender reported as a gauge is an OTLP gauge, and its
    /// labels arrive in the order the exposition renders them: the sender's
    /// own first, the sample's own after.
    #[test]
    fn a_gauge_sample_exports_as_a_gauge_with_its_labels_in_order() {
        let metrics = metrics_by_name(&with_samples(vec![family(
            "queue_depth",
            MetricType::Gauge,
            Some("Jobs waiting."),
            vec![point(
                &[("sheep", "web"), ("id", "3"), ("fold", "backend"), ("model", "qwen")],
                7.5,
            )],
        )]));
        let metric = &metrics["queue_depth"];
        assert!(
            metric.get("sum").is_none(),
            "a gauge sample is not a sum: {metric}"
        );
        assert!(
            metric.get("unit").is_none(),
            "the payload names no unit for a sample either: {metric}"
        );

        let points = metric["gauge"]["dataPoints"]
            .as_array()
            .expect("dataPoints");
        assert_eq!(points.len(), 1, "{metric}");
        assert_eq!(
            attributes_of(&points[0]),
            vec![
                ("sheep", "web"),
                ("id", "3"),
                ("fold", "backend"),
                ("model", "qwen"),
            ]
        );
        // A sample is an `f64` on the wire, so its point is an `asDouble`
        // even when the value happens to be a whole number.
        assert_eq!(points[0]["asDouble"], serde_json::Value::from(7.5f64));
        assert!(points[0].get("asInt").is_none(), "{}", points[0]);
        assert_eq!(
            points[0]["timeUnixNano"],
            serde_json::Value::String(NOW.to_string())
        );
        assert!(
            points[0].get("startTimeUnixNano").is_none(),
            "OTLP gives a gauge no start: {}",
            points[0]
        );
    }

    /// A sample the sender reported as a counter is a monotonic cumulative
    /// sum, so a collector differences two exports instead of reading one
    /// instant. Its `startTimeUnixNano` is the exporter's own start, the same
    /// stamp `shep_sheep_restart_total` carries: the store keeps the time a
    /// retained value last arrived, never the time it first did, and a start
    /// that moved with every sample would tell a collector the counter had
    /// only been growing for a moment.
    #[test]
    fn a_counter_sample_exports_as_a_monotonic_cumulative_sum() {
        let metrics = metrics_by_name(&with_samples(vec![family(
            "requests_total",
            MetricType::Counter,
            Some("Requests served."),
            vec![
                point(&[("sheep", "web"), ("id", "3"), ("fold", "backend")], 4.0),
                point(&[("dog", "bark"), ("source", "built-in")], 2.0),
            ],
        )]));
        let metric = &metrics["requests_total"];
        assert!(
            metric.get("gauge").is_none(),
            "a counter sample is not a gauge: {metric}"
        );

        let sum = metric["sum"].as_object().expect("a counter sample is a sum");
        assert_eq!(
            sum["aggregationTemporality"],
            serde_json::Value::from(2u8),
            "{sum:?}"
        );
        assert_eq!(sum["isMonotonic"], serde_json::Value::Bool(true));

        let points = sum["dataPoints"].as_array().expect("dataPoints");
        assert_eq!(points.len(), 2, "{metric}");
        assert_eq!(
            attributes_of(&points[1]),
            vec![("dog", "bark"), ("source", "built-in")]
        );
        for point in points {
            assert_eq!(
                point["startTimeUnixNano"],
                serde_json::Value::String(START.to_string()),
                "{point}"
            );
            assert_eq!(
                point["timeUnixNano"],
                serde_json::Value::String(NOW.to_string()),
                "{point}"
            );
            assert!(
                point["asDouble"].is_number(),
                "a sample is an f64 whatever its kind: {point}"
            );
            assert!(point.get("asInt").is_none(), "{point}");
        }
        assert_eq!(points[0]["asDouble"], serde_json::Value::from(4.0f64));
        assert_eq!(points[1]["asDouble"], serde_json::Value::from(2.0f64));
    }

    /// A sample's help text is the payload's `description`, and a family the
    /// sender never gave any has no `description` at all: an empty one tells a
    /// collector nothing, the way a family with no help writes no `# HELP`
    /// line in the exposition.
    #[test]
    fn a_sample_help_becomes_the_description_and_no_help_omits_it() {
        let metrics = metrics_by_name(&with_samples(vec![
            family(
                "queue_depth",
                MetricType::Gauge,
                Some("Jobs waiting."),
                vec![point(&[("model", "qwen")], 1.0)],
            ),
            family(
                "in_flight",
                MetricType::Gauge,
                None,
                vec![point(&[("model", "qwen")], 1.0)],
            ),
            family(
                "requests_total",
                MetricType::Counter,
                None,
                vec![point(&[("model", "qwen")], 1.0)],
            ),
        ]));
        assert_eq!(
            metrics["queue_depth"]["description"],
            serde_json::Value::String("Jobs waiting.".to_owned())
        );
        for name in ["in_flight", "requests_total"] {
            assert!(
                metrics[name].get("description").is_none(),
                "{}: {}",
                name,
                metrics[name]
            );
        }
    }

    /// The counter the dog keeps for its own refusals is a count, not a
    /// measurement, so it goes out as `asInt` on a monotonic cumulative sum:
    /// the same spelling `shep_sheep_restart_total` uses for the same reason.
    #[test]
    fn the_refused_counter_is_exported_when_nonzero() {
        let metrics = metrics_by_name(&with_samples(vec![family(
            REFUSED_TOTAL,
            MetricType::Counter,
            Some("Channel samples this dog refused, by reason."),
            vec![
                point(&[("reason", "invalid_name")], 3.0),
                point(&[("reason", "type_conflict")], 1.0),
            ],
        )]));
        let metric = &metrics[REFUSED_TOTAL];
        let sum = metric["sum"]
            .as_object()
            .expect("a refused count is a monotonic sum");
        assert_eq!(sum["isMonotonic"], serde_json::Value::Bool(true));
        assert_eq!(
            sum["aggregationTemporality"],
            serde_json::Value::from(2u8),
            "{sum:?}"
        );

        let points = sum["dataPoints"].as_array().expect("dataPoints");
        assert_eq!(points.len(), 2, "{metric}");
        assert_eq!(attributes_of(&points[0]), vec![("reason", "invalid_name")]);
        assert_eq!(attributes_of(&points[1]), vec![("reason", "type_conflict")]);
        assert_eq!(points[0]["asInt"], serde_json::Value::String("3".to_owned()));
        assert_eq!(points[1]["asInt"], serde_json::Value::String("1".to_owned()));
        for point in points {
            assert!(point.get("asDouble").is_none(), "{point}");
            assert_eq!(
                point["startTimeUnixNano"],
                serde_json::Value::String(START.to_string()),
                "{point}"
            );
        }
    }

    /// Nothing has been refused, so the store put no such family on the
    /// reading, and the payload names nothing either: the exposition writes no
    /// `# TYPE shep_channel_samples_refused_total` block either.
    #[test]
    fn the_refused_counter_is_absent_until_the_first_refusal() {
        let metrics = metrics_by_name(&with_samples(vec![family(
            "queue_depth",
            MetricType::Gauge,
            None,
            vec![point(&[("model", "qwen")], 1.0)],
        )]));
        assert!(
            metrics.get(REFUSED_TOTAL).is_none(),
            "{:?}",
            metrics.keys().collect::<Vec<_>>()
        );
    }

    /// The whole body for one sheep, one dog and one host sample, byte for
    /// byte: the lowerCamelCase spelling, the string timestamps, the order a
    /// collector reads them in.
    #[test]
    fn a_one_sheep_reading_encodes_to_exactly_this_json() {
        let mut dog = sample_info("bark");
        dog.dog = Some(DogSource::BuiltIn);
        let reading = Reading {
            flock: vec![sample_info("web"), dog],
            host: Some(HostReading {
                memory_total_bytes: 16_000_000_000,
                memory_used_bytes: 8_000_000_000,
                processes: 200,
                uptime_seconds: 3600,
            }),
            ..reading()
        };
        assert_eq!(
            encode(&reading, &context()),
            r#"{"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"shep"}},{"key":"service.version","value":{"stringValue":"9.9.9"}},{"key":"host.name","value":{"stringValue":"sheepbox"}}]},"scopeMetrics":[{"scope":{"name":"shep.metrics","version":"9.9.9"},"metrics":[{"name":"shep_sheep_cpu_percent","description":"Tree CPU as a percentage of one core, over the last sampling window.","gauge":{"dataPoints":[{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}}],"timeUnixNano":"1700000000000000000","asDouble":1.5}]}},{"name":"shep_sheep_memory_bytes","description":"Tree resident set size in bytes.","gauge":{"dataPoints":[{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}}],"timeUnixNano":"1700000000000000000","asInt":"2048"}]}},{"name":"shep_sheep_restart_total","description":"Restart count since registration.","sum":{"dataPoints":[{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}}],"startTimeUnixNano":"1699999900000000000","timeUnixNano":"1700000000000000000","asInt":"2"}],"aggregationTemporality":2,"isMonotonic":true}},{"name":"shep_sheep_uptime_seconds","description":"Seconds since this sheep's last successful start.","gauge":{"dataPoints":[{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}}],"timeUnixNano":"1700000000000000000","asInt":"65"}]}},{"name":"shep_sheep_status","description":"1 for the status this sheep is currently in, 0 for every other status.","gauge":{"dataPoints":[{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}},{"key":"status","value":{"stringValue":"starting"}}],"timeUnixNano":"1700000000000000000","asInt":"0"},{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}},{"key":"status","value":{"stringValue":"online"}}],"timeUnixNano":"1700000000000000000","asInt":"1"},{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}},{"key":"status","value":{"stringValue":"stopping"}}],"timeUnixNano":"1700000000000000000","asInt":"0"},{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}},{"key":"status","value":{"stringValue":"stopped"}}],"timeUnixNano":"1700000000000000000","asInt":"0"},{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}},{"key":"status","value":{"stringValue":"errored"}}],"timeUnixNano":"1700000000000000000","asInt":"0"},{"attributes":[{"key":"sheep","value":{"stringValue":"web"}},{"key":"id","value":{"stringValue":"3"}},{"key":"fold","value":{"stringValue":"backend"}},{"key":"status","value":{"stringValue":"waiting-restart"}}],"timeUnixNano":"1700000000000000000","asInt":"0"}]}},{"name":"shep_dog_up","description":"1 when this dog is online, 0 otherwise.","gauge":{"dataPoints":[{"attributes":[{"key":"dog","value":{"stringValue":"bark"}},{"key":"source","value":{"stringValue":"built-in"}}],"timeUnixNano":"1700000000000000000","asInt":"1"}]}},{"name":"shep_daemon_up","description":"Always 1: the scrape reached the shepherd.","gauge":{"dataPoints":[{"attributes":[{"key":"version","value":{"stringValue":"9.9.9"}}],"timeUnixNano":"1700000000000000000","asInt":"1"}]}},{"name":"shep_daemon_pid","description":"The shepherd's own pid, so a restart is visible as a step change.","gauge":{"dataPoints":[{"attributes":[],"timeUnixNano":"1700000000000000000","asInt":"12345"}]}},{"name":"shep_host_memory_total_bytes","description":"Total physical memory on the host.","gauge":{"dataPoints":[{"attributes":[],"timeUnixNano":"1700000000000000000","asInt":"16000000000"}]}},{"name":"shep_host_memory_used_bytes","description":"Memory in use on the host, as the platform reports it.","gauge":{"dataPoints":[{"attributes":[],"timeUnixNano":"1700000000000000000","asInt":"8000000000"}]}},{"name":"shep_host_processes","description":"Number of processes running on the host, the flock included.","gauge":{"dataPoints":[{"attributes":[],"timeUnixNano":"1700000000000000000","asInt":"200"}]}},{"name":"shep_host_uptime_seconds","description":"Seconds since the host booted.","gauge":{"dataPoints":[{"attributes":[],"timeUnixNano":"1700000000000000000","asInt":"3600"}]}}]}]}]}"#,
        );
    }

    /// `startTimeUnixNano` is the exporter's own start, not the point's: a
    /// collector that differences two exports gets the restarts inside the
    /// interval, which is what a monotonic cumulative sum is for.
    #[test]
    fn restart_total_is_a_cumulative_monotonic_sum() {
        let reading = Reading {
            flock: vec![sample_info("web")],
            ..reading()
        };
        let metrics = metrics_by_name(&reading);
        let sum = metrics["shep_sheep_restart_total"]["sum"]
            .as_object()
            .expect("restart_total is a sum")
            .clone();
        assert_eq!(sum["aggregationTemporality"], serde_json::Value::from(2u8));
        assert!(
            sum["aggregationTemporality"].is_number(),
            "OTLP's JSON form carries temporality as an integer, not a name: {sum:?}"
        );
        assert_eq!(sum["isMonotonic"], serde_json::Value::Bool(true));

        let point = sum["dataPoints"].as_array().expect("dataPoints")[0].clone();
        assert_eq!(
            point["startTimeUnixNano"],
            serde_json::Value::String(START.to_string()),
            "the sum's start is the exporter's start"
        );
        assert_eq!(
            point["timeUnixNano"],
            serde_json::Value::String(NOW.to_string())
        );

        // A gauge carries no start: OTLP gives an instant's value none.
        let gauge = metrics["shep_sheep_memory_bytes"]["gauge"]
            .as_object()
            .expect("memory_bytes is a gauge")
            .clone();
        assert!(
            gauge["dataPoints"][0].get("startTimeUnixNano").is_none(),
            "{gauge:?}"
        );
    }

    /// 64-bit values are strings, because a JSON number cannot hold them
    /// exactly; a rate is a number, because it is not a 64-bit integer.
    #[test]
    fn integers_are_decimal_strings_and_doubles_are_numbers() {
        let reading = Reading {
            flock: vec![sample_info("web")],
            ..reading()
        };
        let metrics = metrics_by_name(&reading);
        let memory = &metrics["shep_sheep_memory_bytes"]["gauge"]["dataPoints"][0];
        assert_eq!(
            memory["asInt"],
            serde_json::Value::String("2048".to_owned())
        );
        let cpu = &metrics["shep_sheep_cpu_percent"]["gauge"]["dataPoints"][0];
        assert_eq!(cpu["asDouble"], serde_json::Value::from(1.5f64));
        assert!(
            cpu["asDouble"].is_number(),
            "a rate is a JSON number, not a string: {cpu}"
        );
        assert!(memory.get("asDouble").is_none(), "{memory}");
        assert!(cpu.get("asInt").is_none(), "{cpu}");
    }

    /// A sheep's name is operator-supplied and reaches the payload verbatim.
    /// JSON escaping is what keeps a quote in one name from corrupting the
    /// body a collector parses, and a newline from splitting a line.
    #[test]
    fn an_attribute_value_with_a_quote_and_a_newline_survives() {
        let reading = Reading {
            flock: vec![sample_info("we\"b\nx")],
            ..reading()
        };
        let text = encode(&reading, &context());
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        let name = parsed
            .pointer("/resourceMetrics/0/scopeMetrics/0/metrics/0/gauge/dataPoints/0/attributes/0/value/stringValue")
            .and_then(serde_json::Value::as_str)
            .expect("the first attribute is the sheep's name");
        assert_eq!(name, "we\"b\nx");
        assert!(
            text.contains(r#"{"key":"sheep","value":{"stringValue":"we\"b\nx"}}"#),
            "{text}"
        );
    }

    /// A parked dog exports no series, the rule the exposition follows so
    /// that disabling one never reads as it going down.
    #[test]
    fn a_disabled_dog_exports_nothing() {
        let mut parked = sample_info("bark");
        parked.dog = Some(DogSource::BuiltIn);
        parked.status = ProcStatus::Stopped;
        parked.disabled = Some(true);
        let metrics = metrics_by_name(&Reading {
            flock: vec![parked],
            ..reading()
        });
        assert!(
            metrics.get("shep_dog_up").is_none(),
            "{:?}",
            metrics.keys().collect::<Vec<_>>()
        );
    }

    /// A sheep the shepherd has not sampled answers "unknown", which OTLP
    /// has no value for. Encoding it as `0` would claim the sheep is using
    /// no memory, which `render` is careful never to claim either.
    #[test]
    fn a_sheep_with_no_sample_contributes_no_points() {
        let mut idle = sample_info("worker");
        idle.cpu_percent = None;
        idle.memory_bytes = None;
        let metrics = metrics_by_name(&Reading {
            flock: vec![idle],
            ..reading()
        });
        assert!(
            metrics.get("shep_sheep_cpu_percent").is_none(),
            "{:?}",
            metrics.keys().collect::<Vec<_>>()
        );
        assert!(metrics.get("shep_sheep_memory_bytes").is_none());
        // The counters need no sample and are still there.
        assert!(metrics.get("shep_sheep_restart_total").is_some());
    }

    /// `host.name` is a resource attribute, and only when the OS reports
    /// one: an empty one tells a collector nothing and costs a line.
    #[test]
    fn the_resource_names_the_service_and_the_host_only_when_there_is_one() {
        let reading = reading();
        let attributes = payload(&reading)["resourceMetrics"][0]["resource"]["attributes"]
            .as_array()
            .expect("attributes")
            .clone();
        let pairs: Vec<(&str, &str)> = attributes
            .iter()
            .map(|attribute| {
                (
                    attribute["key"].as_str().expect("key"),
                    attribute["value"]["stringValue"].as_str().expect("value"),
                )
            })
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("service.name", "shep"),
                ("service.version", "9.9.9"),
                ("host.name", "sheepbox"),
            ]
        );

        let no_host = ExportContext {
            host_name: None,
            ..context()
        };
        let without = payload_with(&reading, &no_host);
        let names: Vec<&str> = without["resourceMetrics"][0]["resource"]["attributes"]
            .as_array()
            .expect("attributes")
            .iter()
            .map(|attribute| attribute["key"].as_str().expect("key"))
            .collect();
        assert_eq!(names, vec!["service.name", "service.version"]);
    }
}
