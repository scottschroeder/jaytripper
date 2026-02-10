use std::time::{Duration, Instant};

use jaytripper_core::{
    events::{MovementEvent, MovementEventSink, MovementEventSource},
    time::Timestamp,
};
use rand::{Rng, SeedableRng, rngs::SmallRng};
use tokio::{sync::watch, time::sleep};

use crate::{
    EsiError, EsiResult,
    api::CharacterLocation,
    auth::{Clock, SystemClock},
    esi_client::EsiClient,
};

#[derive(Clone, Debug)]
pub struct LocationPollConfig {
    pub base_interval: Duration,
    pub jitter_factor: f32,
    pub api_failure_backoff_initial: Duration,
    pub api_failure_backoff_max: Duration,
}

impl Default for LocationPollConfig {
    fn default() -> Self {
        Self {
            base_interval: Duration::from_secs(5),
            jitter_factor: 0.2,
            api_failure_backoff_initial: Duration::from_secs(1),
            api_failure_backoff_max: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollMetrics {
    pub last_success_at: Option<Timestamp>,
    pub last_poll_latency: Option<Duration>,
}

pub struct LocationIngestor<C, S, T = SystemClock>
where
    C: EsiClient + Send + Sync,
    S: MovementEventSink + Send + Sync,
    <S as MovementEventSink>::Error: std::fmt::Display,
    T: Clock + Send + Sync,
{
    client: C,
    sink: S,
    clock: T,
    config: LocationPollConfig,
    last_location: Option<CharacterLocation>,
    api_consecutive_failures: u32,
    metrics: PollMetrics,
    rng: SmallRng,
}

impl<C, S> LocationIngestor<C, S, SystemClock>
where
    C: EsiClient + Send + Sync,
    S: MovementEventSink + Send + Sync,
    <S as MovementEventSink>::Error: std::fmt::Display,
{
    pub fn new(client: C, sink: S, config: LocationPollConfig) -> Self {
        Self::with_clock(client, sink, config, SystemClock)
    }
}

impl<C, S, T> LocationIngestor<C, S, T>
where
    C: EsiClient + Send + Sync,
    S: MovementEventSink + Send + Sync,
    <S as MovementEventSink>::Error: std::fmt::Display,
    T: Clock + Send + Sync,
{
    pub fn with_clock(client: C, sink: S, config: LocationPollConfig, clock: T) -> Self {
        let seed = 0xD1CE_F00D_u64 ^ client.character_id().0;

        Self {
            client,
            sink,
            clock,
            config,
            last_location: None,
            api_consecutive_failures: 0,
            metrics: PollMetrics::default(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    pub fn metrics(&self) -> PollMetrics {
        self.metrics.clone()
    }

    pub fn api_consecutive_failures(&self) -> u32 {
        self.api_consecutive_failures
    }

    pub async fn run_until_shutdown(
        &mut self,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> EsiResult<()> {
        log::debug!(
            "location ingestor starting for character {}",
            self.client.character_id()
        );
        loop {
            if *shutdown_rx.borrow() {
                log::debug!(
                    "location ingestor received shutdown before poll for character {}",
                    self.client.character_id()
                );
                return Ok(());
            }

            let outcome = tokio::select! {
                outcome = self.poll_once() => outcome,
                changed = shutdown_rx.changed() => {
                    if shutdown_signaled(changed, &shutdown_rx) {
                        return Ok(());
                    }
                    continue;
                }
            };

            let wait = match outcome {
                PollOutcome::Success => self.next_success_delay(),
                PollOutcome::ApiFailure(err) => {
                    let wait = self.next_api_failure_delay();
                    log::error!(
                        "poll API failure for character {} (consecutive failures: {}, retry in {:?}): {:?}",
                        self.client.character_id(),
                        self.api_consecutive_failures,
                        wait,
                        err.display_chain()
                    );
                    wait
                }
                PollOutcome::Terminal(err) => return Err(err),
            };

            tokio::select! {
                _ = sleep(wait) => {}
                changed = shutdown_rx.changed() => {
                    if shutdown_signaled(changed, &shutdown_rx) {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn poll_once(&mut self) -> PollOutcome {
        log::trace!(
            "polling current location for character {}",
            self.client.character_id()
        );
        let started = Instant::now();

        let location = match self.fetch_location().await {
            Ok(location) => location,
            Err(outcome) => return outcome,
        };

        let observed_at = self.clock.now();

        if let Err(outcome) = self.observe_location(location, observed_at).await {
            return outcome;
        }

        self.record_success(started.elapsed(), observed_at);
        log::trace!(
            "poll success for character {} at {}",
            self.client.character_id(),
            observed_at.as_epoch_secs()
        );
        PollOutcome::Success
    }

    async fn fetch_location(&mut self) -> Result<CharacterLocation, PollOutcome> {
        match self.client.get_current_location().await {
            Ok(location) => {
                log::trace!(
                    "fetched location for character {} in system {}",
                    self.client.character_id(),
                    location.solar_system_id
                );
                Ok(location)
            }
            Err(EsiError::NeedsReauth { reason }) => {
                log::debug!(
                    "poll terminal: reauth required for character {} ({reason})",
                    self.client.character_id()
                );
                Err(PollOutcome::Terminal(EsiError::NeedsReauth { reason }))
            }
            Err(err) => {
                self.record_api_failure();
                log::trace!(
                    "poll API failure for character {} (consecutive failures: {})",
                    self.client.character_id(),
                    self.api_consecutive_failures
                );
                Err(PollOutcome::ApiFailure(err))
            }
        }
    }

    async fn observe_location(
        &mut self,
        location: CharacterLocation,
        observed_at: Timestamp,
    ) -> Result<(), PollOutcome> {
        let should_emit_event = self
            .last_location
            .as_ref()
            .map(|previous| previous.solar_system_id != location.solar_system_id)
            .unwrap_or(true);

        if should_emit_event {
            let event = MovementEvent {
                character_id: self.client.character_id(),
                from_system_id: self
                    .last_location
                    .as_ref()
                    .map(|previous| previous.solar_system_id),
                to_system_id: location.solar_system_id,
                observed_at,
                source: MovementEventSource::Esi,
            };

            if let Err(err) = self.sink.emit_movement(event).await {
                return Err(PollOutcome::Terminal(EsiError::message(format!(
                    "failed to emit movement event: {err}"
                ))));
            }

            log::debug!(
                "emitted movement event for character {}",
                self.client.character_id()
            );
        }

        self.last_location = Some(location);
        Ok(())
    }

    fn record_success(&mut self, latency: Duration, observed_at: Timestamp) {
        self.api_consecutive_failures = 0;
        self.metrics.last_success_at = Some(observed_at);
        self.metrics.last_poll_latency = Some(latency);
    }

    fn record_api_failure(&mut self) {
        self.api_consecutive_failures = self.api_consecutive_failures.saturating_add(1);
    }

    fn next_success_delay(&mut self) -> Duration {
        self.jittered_duration(self.config.base_interval)
    }

    fn next_api_failure_delay(&self) -> Duration {
        exponential_backoff(
            self.api_consecutive_failures,
            self.config.api_failure_backoff_initial,
            self.config.api_failure_backoff_max,
        )
    }

    fn jittered_duration(&mut self, base: Duration) -> Duration {
        let jitter_factor = self.config.jitter_factor.clamp(0.0, 1.0) as f64;
        if jitter_factor <= 0.0 {
            return base;
        }

        let multiplier = self
            .rng
            .gen_range((1.0 - jitter_factor)..=(1.0 + jitter_factor));
        base.mul_f64(multiplier)
    }
}

fn shutdown_signaled(
    changed: Result<(), watch::error::RecvError>,
    shutdown_rx: &watch::Receiver<bool>,
) -> bool {
    changed.is_err() || *shutdown_rx.borrow()
}

fn exponential_backoff(attempts: u32, initial: Duration, max: Duration) -> Duration {
    let exponent = attempts.saturating_sub(1).min(31);
    let factor = 1_u128 << exponent;
    let initial_ms = initial.as_millis();
    let max_ms = max.as_millis();
    let backoff_ms = initial_ms.saturating_mul(factor).min(max_ms);
    Duration::from_millis(backoff_ms as u64)
}

enum PollOutcome {
    Success,
    ApiFailure(EsiError),
    Terminal(EsiError),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use async_trait::async_trait;
    use jaytripper_core::{
        MovementEvent, MovementEventSink, MovementEventSource,
        ids::{CharacterId, SolarSystemId, StationId},
        time::Timestamp,
    };
    use tokio::sync::watch;

    use super::{LocationIngestor, LocationPollConfig, PollOutcome};
    use crate::{EsiError, EsiResult, api::CharacterLocation, auth::Clock, esi_client::EsiClient};

    #[derive(Clone, Copy)]
    struct FixedClock {
        now: Timestamp,
    }

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            self.now
        }
    }

    struct MockEsiClient {
        character_id: CharacterId,
        responses: Mutex<VecDeque<EsiResult<CharacterLocation>>>,
    }

    #[async_trait]
    impl EsiClient for MockEsiClient {
        fn character_id(&self) -> CharacterId {
            self.character_id
        }

        fn requires_reauth(&self) -> bool {
            false
        }

        fn reauth_reason(&self) -> Option<String> {
            None
        }

        async fn get_current_location(&self) -> EsiResult<CharacterLocation> {
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .unwrap_or_else(|| Err(EsiError::message("no response configured")))
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<MovementEvent>>,
        fail_emits: Mutex<usize>,
    }

    #[derive(Clone)]
    struct SharedRecordingSink(Arc<RecordingSink>);

    #[async_trait]
    impl MovementEventSink for SharedRecordingSink {
        type Error = EsiError;

        async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error> {
            {
                let mut remaining = self.0.fail_emits.lock().expect("fail lock");
                if *remaining > 0 {
                    *remaining -= 1;
                    return Err(EsiError::message("sink unavailable"));
                }
            }

            self.0.events.lock().expect("events lock").push(event);
            Ok(())
        }
    }

    fn location(system: i32, station: Option<i32>) -> CharacterLocation {
        CharacterLocation {
            solar_system_id: SolarSystemId(system),
            station_id: station.map(StationId),
            structure_id: None,
        }
    }

    fn config_for_tests() -> LocationPollConfig {
        LocationPollConfig {
            base_interval: Duration::from_secs(5),
            jitter_factor: 0.0,
            api_failure_backoff_initial: Duration::from_secs(1),
            api_failure_backoff_max: Duration::from_secs(30),
        }
    }

    #[tokio::test]
    async fn emits_event_on_first_poll_and_transition_only() {
        let client = MockEsiClient {
            character_id: CharacterId(42),
            responses: Mutex::new(VecDeque::from(vec![
                Ok(location(30000142, Some(1))),
                Ok(location(30000142, Some(2))),
                Ok(location(30002510, None)),
            ])),
        };
        let sink = Arc::new(RecordingSink::default());
        let mut ingestor = LocationIngestor::with_clock(
            client,
            SharedRecordingSink(Arc::clone(&sink)),
            config_for_tests(),
            FixedClock {
                now: ts(1_700_000_000),
            },
        );

        assert!(matches!(ingestor.poll_once().await, PollOutcome::Success));
        assert!(matches!(ingestor.poll_once().await, PollOutcome::Success));
        assert!(matches!(ingestor.poll_once().await, PollOutcome::Success));

        let events = sink.events.lock().expect("events lock");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].character_id, CharacterId(42));
        assert_eq!(events[0].from_system_id, None);
        assert_eq!(events[0].to_system_id, SolarSystemId(30000142));
        assert_eq!(events[0].observed_at, ts(1_700_000_000));
        assert_eq!(events[0].source, MovementEventSource::Esi);
        assert_eq!(events[1].from_system_id, Some(SolarSystemId(30000142)));
        assert_eq!(events[1].to_system_id, SolarSystemId(30002510));
    }

    #[tokio::test]
    async fn sink_failure_is_terminal() {
        let client = MockEsiClient {
            character_id: CharacterId(42),
            responses: Mutex::new(VecDeque::from(vec![Ok(location(30000142, None))])),
        };
        let sink = Arc::new(RecordingSink::default());
        *sink.fail_emits.lock().expect("fail lock") = 1;

        let mut ingestor = LocationIngestor::with_clock(
            client,
            SharedRecordingSink(Arc::clone(&sink)),
            config_for_tests(),
            FixedClock {
                now: ts(1_700_000_001),
            },
        );

        let outcome = ingestor.poll_once().await;
        assert!(matches!(outcome, PollOutcome::Terminal(_)));
    }

    #[tokio::test]
    async fn api_failures_backoff() {
        let client = MockEsiClient {
            character_id: CharacterId(42),
            responses: Mutex::new(VecDeque::from(vec![Err(EsiError::message(
                "esi unavailable",
            ))])),
        };
        let sink = Arc::new(RecordingSink::default());

        let mut ingestor = LocationIngestor::with_clock(
            client,
            SharedRecordingSink(Arc::clone(&sink)),
            config_for_tests(),
            FixedClock {
                now: ts(1_700_000_001),
            },
        );

        assert!(matches!(
            ingestor.poll_once().await,
            PollOutcome::ApiFailure(_)
        ));
        assert_eq!(ingestor.api_consecutive_failures(), 1);
        assert_eq!(ingestor.next_api_failure_delay(), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn records_poll_metrics_after_success() {
        let client = MockEsiClient {
            character_id: CharacterId(42),
            responses: Mutex::new(VecDeque::from(vec![Ok(location(30000142, None))])),
        };
        let sink = Arc::new(RecordingSink::default());
        let mut ingestor = LocationIngestor::with_clock(
            client,
            SharedRecordingSink(Arc::clone(&sink)),
            config_for_tests(),
            FixedClock {
                now: ts(1_700_000_100),
            },
        );

        assert!(matches!(ingestor.poll_once().await, PollOutcome::Success));
        let metrics = ingestor.metrics();
        assert_eq!(metrics.last_success_at, Some(ts(1_700_000_100)));
        assert!(metrics.last_poll_latency.is_some());
    }

    #[tokio::test]
    async fn exits_cleanly_when_shutdown_signal_is_set() {
        let client = MockEsiClient {
            character_id: CharacterId(42),
            responses: Mutex::new(VecDeque::new()),
        };
        let sink = Arc::new(RecordingSink::default());
        let mut ingestor = LocationIngestor::with_clock(
            client,
            SharedRecordingSink(Arc::clone(&sink)),
            config_for_tests(),
            FixedClock {
                now: ts(1_700_000_200),
            },
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(true);
        drop(shutdown_tx);

        ingestor
            .run_until_shutdown(shutdown_rx)
            .await
            .expect("shutdown path should succeed");
    }

    fn ts(epoch_secs: i64) -> Timestamp {
        Timestamp::from_epoch_secs(epoch_secs).expect("valid epoch seconds")
    }
}
