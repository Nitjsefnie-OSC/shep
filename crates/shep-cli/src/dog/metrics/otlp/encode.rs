//! The OTLP/JSON mapping for a [`Reading`]: the same series
//! [`crate::dog::metrics::exposition::render`] serves, in the shape a
//! collector reads.
//!
//! The two renderers share no code on purpose. `render` is allocation-tuned
//! (one escaped label prefix per sheep, shared by its six status series) and
//! a neutral intermediate model would spend allocations on every scrape to
//! serve a path that is not the scrape. The drift guard is
//! `the_payload_names_every_series_the_exposition_names` in this module's
//! tests, which compares names, types, descriptions and point counts
//! between the two.
//!
//! Where the mapping differs from plain serde: 64-bit integers are decimal
//! strings ([`Nanos`], [`IntStr`]), doubles are numbers, and
//! `aggregationTemporality` is the integer `2` rather than a name.

use std::borrow::Cow;

use serde::Serialize;
use shep_core::status::ProcStatus;

use crate::dog::metrics::Reading;
use crate::dog::metrics::exposition::{ALL_STATUSES, status_label};

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
/// `unit` is left out entirely. A unit is what makes a Prometheus-side
/// collector append `_bytes` to a name that already ends in it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metric<'a> {
    name: &'static str,
    description: &'static str,
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

/// A `metrics[]` entry, or nothing at all when it has no points, which is
/// how `render` drops a group with no series.
fn gauge<'a>(
    name: &'static str,
    description: &'static str,
    data_points: Vec<DataPoint<'a>>,
) -> Option<Metric<'a>> {
    (!data_points.is_empty()).then_some(Metric {
        name,
        description,
        gauge: Some(Gauge { data_points }),
        sum: None,
    })
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
/// non-finite `cpu_percent` is dropped rather than encoded, since OTLP has
/// no number for it.
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
            "Tree CPU as a percentage of one core, over the last sampling window.",
            cpu,
        ),
        gauge(
            "shep_sheep_memory_bytes",
            "Tree resident set size in bytes.",
            memory,
        ),
        (!restarts.is_empty()).then_some(Metric {
            name: "shep_sheep_restart_total",
            description: "Restart count since registration.",
            gauge: None,
            sum: Some(Sum {
                data_points: restarts,
                aggregation_temporality: TEMPORALITY_CUMULATIVE,
                is_monotonic: true,
            }),
        }),
        gauge(
            "shep_sheep_uptime_seconds",
            "Seconds since this sheep's last successful start.",
            uptime,
        ),
        gauge(
            "shep_sheep_status",
            "1 for the status this sheep is currently in, 0 for every other status.",
            status,
        ),
        gauge(
            "shep_dog_up",
            "1 when this dog is online, 0 otherwise.",
            dog_up,
        ),
        gauge(
            "shep_daemon_up",
            "Always 1: the scrape reached the shepherd.",
            vec![daemon_up],
        ),
        gauge(
            "shep_daemon_pid",
            "The shepherd's own pid, so a restart is visible as a step change.",
            vec![daemon_pid],
        ),
    ] {
        metrics.extend(group);
    }

    if let Some(host) = &reading.host {
        for group in [
            gauge(
                "shep_host_memory_total_bytes",
                "Total physical memory on the host.",
                vec![gauge_point(Vec::new(), now, host.memory_total_bytes)],
            ),
            gauge(
                "shep_host_memory_used_bytes",
                "Memory in use on the host, as the platform reports it.",
                vec![gauge_point(Vec::new(), now, host.memory_used_bytes)],
            ),
            gauge(
                "shep_host_processes",
                "Number of processes running on the host, the flock included.",
                vec![gauge_point(
                    Vec::new(),
                    now,
                    u64::try_from(host.processes).unwrap_or(u64::MAX),
                )],
            ),
            gauge(
                "shep_host_uptime_seconds",
                "Seconds since the host booted.",
                vec![gauge_point(Vec::new(), now, host.uptime_seconds)],
            ),
        ] {
            metrics.extend(group);
        }
    }

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
    use shep_core::protocol::{DogSource, ProcessInfo};
    use shep_core::status::ProcStatus;

    use crate::dog::metrics::{HostReading, Reading, exposition};

    use super::*;

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
    /// number of series lines under it.
    struct Group {
        name: String,
        help: String,
        kind: String,
        series: usize,
    }

    /// The group `name` belongs to, created on whichever of its three lines
    /// arrives first.
    fn group_at<'a>(groups: &'a mut Vec<Group>, name: &str) -> &'a mut Group {
        if !groups.iter().any(|group| group.name == name) {
            groups.push(Group {
                name: name.to_owned(),
                help: String::new(),
                kind: String::new(),
                series: 0,
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
                group_at(&mut groups, name).series += 1;
            }
        }
        groups
    }

    /// The drift guard: every name the exposition emits, with its `# TYPE`,
    /// its `# HELP` text and its series count, appears in the OTLP payload
    /// as the same kind with the same number of points.
    ///
    /// Fails if a series lands in one renderer and not the other, which is
    /// the defect two renderers that share no code grow by default.
    #[test]
    fn the_payload_names_every_series_the_exposition_names() {
        let reading = exposition::tests::golden_reading();
        let text = exposition::render(&reading);
        let metrics = metrics_by_name(&reading);
        let expected = groups(&text);
        assert!(
            !expected.is_empty(),
            "the golden exposition named no metric at all: {text}"
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
            assert_eq!(
                metric["description"].as_str().expect("description"),
                group.help,
                "{}: the description drifted from the HELP text",
                group.name
            );
            let points = metric[field]["dataPoints"]
                .as_array()
                .expect("dataPoints")
                .len();
            assert_eq!(points, group.series, "{}: point count", group.name);
        }
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
