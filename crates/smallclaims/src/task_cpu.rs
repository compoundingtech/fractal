//! Thread CPU consumed by daemon tasks, excluding time parked between polls.
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::LazyLock;
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;

struct Instruments {
    cpu: Counter<f64>,
    polls: Counter<u64>,
}

static METRICS: LazyLock<Instruments> =
    LazyLock::new(|| instruments(&opentelemetry::global::meter("st3")));

fn instruments(meter: &opentelemetry::metrics::Meter) -> Instruments {
    Instruments {
        cpu: meter.f64_counter("st.task.cpu_time")
            .with_description("Thread CPU consumed while polling a daemon task or running its blocking work")
            .with_unit("s").build(),
        polls: meter.u64_counter("st.task.polls")
            .with_description("Daemon task polls and blocking invocations")
            .with_unit("{poll}").build(),
    }
}

fn thread_cpu() -> Option<Duration> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: time is writable and the clock measures only the calling thread.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return None;
    }
    Some(Duration::new(time.tv_sec.try_into().ok()?, time.tv_nsec.try_into().ok()?))
}

struct Sample<'a> {
    started: Option<Duration>,
    attributes: &'a [KeyValue; 1],
    metrics: &'a Instruments,
}

impl Drop for Sample<'_> {
    fn drop(&mut self) {
        let ended = thread_cpu();
        self.metrics.polls.add(1, self.attributes);
        if let Some(elapsed) = self.started.zip(ended).and_then(|(start, end)| end.checked_sub(start)) {
            self.metrics.cpu.add(elapsed.as_secs_f64(), self.attributes);
        }
    }
}

/// Measure each poll on its current thread; never retain a clock sample across an await.
/// The name must describe an operation, never an instance, seat, peer, or request.
pub async fn account<F: Future>(name: &'static str, future: F) -> F::Output {
    measured(name, future, &METRICS).await
}

async fn measured<F: Future>(name: &'static str, future: F, metrics: &Instruments) -> F::Output {
    let attributes = [KeyValue::new("st.task", name)];
    let mut future = pin!(future);
    poll_fn(|cx| {
        let _sample = Sample { started: thread_cpu(), attributes: &attributes, metrics };
        let _origin = crate::sqlite::telemetry::enter_task_origin(name);
        crate::profile::profiled_task(name, || future.as_mut().poll(cx))
    }).await
}

/// Measure one blocking invocation, and label any writer work it performs.
pub fn blocking<T>(name: &'static str, work: impl FnOnce() -> T) -> T {
    let attributes = [KeyValue::new("st.task", name)];
    let metrics = &*METRICS;
    let _sample = Sample { started: thread_cpu(), attributes: &attributes, metrics };
    crate::profile::task(name, work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};

    #[tokio::test]
    async fn busy_future_accrues_cpu_but_sleep_does_not() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build()).build();
        let metrics = instruments(&provider.meter("task-cpu-test"));
        measured("test.busy", async {
            let started = thread_cpu().unwrap();
            while thread_cpu().unwrap().saturating_sub(started) < Duration::from_millis(20) {
                std::hint::spin_loop();
            }
        }, &metrics).await;
        measured("test.sleep", tokio::time::sleep(Duration::from_millis(100)), &metrics).await;
        provider.force_flush().unwrap();
        let exported = exporter.get_finished_metrics().unwrap();
        let cpu = exported.iter().flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics())
            .find(|metric| metric.name() == "st.task.cpu_time")
            .and_then(|metric| match metric.data() {
                AggregatedMetrics::F64(MetricData::Sum(sum)) => Some(sum),
                _ => None,
            }).expect("CPU counter is exported");
        let value = |name: &'static str| cpu.data_points().filter(|point| point.attributes().any(|attribute| {
            attribute.key.as_str() == "st.task" && attribute.value == opentelemetry::Value::from(name)
        })).map(|point| point.value()).sum::<f64>();
        assert!(value("test.busy") >= 0.02);
        assert!(value("test.sleep") < 0.01);
        let polls = exported.iter().flat_map(|resource| resource.scope_metrics())
            .flat_map(|scope| scope.metrics())
            .find(|metric| metric.name() == "st.task.polls")
            .and_then(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => Some(sum),
                _ => None,
            }).expect("poll counter is exported");
        assert!(polls.data_points().map(|point| point.value()).sum::<u64>() >= 3);
    }
}
