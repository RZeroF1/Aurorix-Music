//! Platform-neutral composition root for the Aurorix Core.
//!
//! [`CoreHost`] owns one local database and one playback session for the
//! process. It deliberately composes the existing boundaries instead of
//! adding a second reducer, queue, clock, migration set, or audio engine.
//! Platform adapters and FFI clients are consumers of this host and are not
//! implemented here.

use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use aurorix_playback::{
    command::PlaybackCommand,
    pipeline::DEFAULT_OUTPUT_SAMPLE_RATE_HZ,
    session::{
        PlaybackSession, PlaybackSnapshot, SessionError, SessionUpdate, WorkerEvent, WorkerUpdate,
    },
};
use aurorix_storage::{
    LOCAL_MIGRATIONS,
    database::{Database, DatabaseError},
    migration::MigrationError,
};

/// The filename used for the local Core database below the configured data
/// directory.
pub const DEFAULT_DATABASE_FILE_NAME: &str = "aurorix.sqlite3";

/// The default upper bound used by [`CoreHost::shutdown`].
pub const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

static HOST_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Configuration for one process-scoped [`CoreHost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreHostConfig {
    data_dir: PathBuf,
    output_sample_rate: u32,
    shutdown_timeout: Duration,
}

impl CoreHostConfig {
    /// Creates a configuration using the default output sample rate and
    /// shutdown timeout.
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            output_sample_rate: DEFAULT_OUTPUT_SAMPLE_RATE_HZ,
            shutdown_timeout: DEFAULT_SHUTDOWN_TIMEOUT,
        }
    }

    /// Returns the local data directory, without opening or normalizing it.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Returns the path of the database this host will open.
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(DEFAULT_DATABASE_FILE_NAME)
    }

    /// Sets the initial playback output sample rate.
    #[must_use]
    pub fn with_output_sample_rate(mut self, output_sample_rate: u32) -> Self {
        self.output_sample_rate = output_sample_rate;
        self
    }

    /// Sets the timeout used by [`CoreHost::shutdown`].
    #[must_use]
    pub fn with_shutdown_timeout(mut self, shutdown_timeout: Duration) -> Self {
        self.shutdown_timeout = shutdown_timeout;
        self
    }

    /// Returns the configured output sample rate.
    #[must_use]
    pub const fn output_sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    /// Returns the configured shutdown timeout.
    #[must_use]
    pub const fn shutdown_timeout(&self) -> Duration {
        self.shutdown_timeout
    }
}

impl Default for CoreHostConfig {
    fn default() -> Self {
        Self::new(PathBuf::from("aurorix-data"))
    }
}

/// A failure while constructing a [`CoreHost`].
#[derive(Debug)]
pub enum CoreHostStartError {
    /// A process-scoped host is already alive or has not yet drained its
    /// callback fence.
    AlreadyRunning,
    /// The supplied configuration cannot satisfy the host invariants.
    InvalidConfiguration(&'static str),
    /// The configured local data directory could not be created.
    CreateDataDirectory { source: std::io::Error },
    /// The local database failed its existing startup capability checks.
    Database { source: DatabaseError },
    /// The existing application migrations could not be applied.
    Migration { source: MigrationError },
    /// The existing playback clock could not be created for the requested
    /// output sample rate.
    Playback { source: SessionError },
}

impl fmt::Display for CoreHostStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => formatter.write_str("a CoreHost is already running"),
            Self::InvalidConfiguration(field) => {
                write!(formatter, "CoreHost configuration is invalid: {field}")
            }
            Self::CreateDataDirectory { .. } => {
                formatter.write_str("CoreHost data directory could not be created")
            }
            Self::Database { .. } => formatter.write_str("CoreHost database could not be opened"),
            Self::Migration { .. } => formatter.write_str("CoreHost database migration failed"),
            Self::Playback { .. } => {
                formatter.write_str("CoreHost playback session could not start")
            }
        }
    }
}

impl Error for CoreHostStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateDataDirectory { source } => Some(source),
            Self::Database { source } => Some(source),
            Self::Migration { source } => Some(source),
            Self::Playback { source } => Some(source),
            Self::AlreadyRunning | Self::InvalidConfiguration(_) => None,
        }
    }
}

/// A failure while accessing an already-created host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreHostAccessError {
    /// The host is rejecting work because shutdown has begun.
    ShuttingDown,
    /// The host completed shutdown and released its services.
    Stopped,
    /// The requested playback transition failed its checked invariants.
    Playback(SessionError),
    /// An internal service lock was poisoned after a panic.
    StatePoisoned,
}

impl fmt::Display for CoreHostAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShuttingDown => formatter.write_str("CoreHost is shutting down"),
            Self::Stopped => formatter.write_str("CoreHost has stopped"),
            Self::Playback(error) => write!(formatter, "CoreHost playback access failed: {error}"),
            Self::StatePoisoned => formatter.write_str("CoreHost state lock is poisoned"),
        }
    }
}

impl Error for CoreHostAccessError {}

/// The externally observable lifecycle state of one [`CoreHost`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreHostState {
    /// New commands and callback entries are accepted.
    Running,
    /// New work is rejected while cancellation and callback fencing drain.
    ShuttingDown,
    /// Services have been released and no work is accepted.
    Stopped,
}

/// The result of a bounded shutdown attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// Callbacks drained and Core services were released.
    Complete,
    /// Callback execution was still in flight when the timeout elapsed.
    Incomplete { active_callbacks: usize },
}

impl ShutdownOutcome {
    /// Returns whether the shutdown completed and resources were released.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }

    /// Returns the number of callbacks still in flight, if shutdown timed out.
    #[must_use]
    pub const fn active_callbacks(self) -> usize {
        match self {
            Self::Complete => 0,
            Self::Incomplete { active_callbacks } => active_callbacks,
        }
    }
}

/// The outcome of requesting cooperative operation cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationOutcome {
    /// This call changed the operation from active to cancelled.
    Requested,
    /// The operation had already been cancelled; no second signal was sent.
    AlreadyRequested,
}

/// A host-owned cooperative cancellation token for non-realtime operations.
///
/// Cancellation only requests that a worker stop waiting and exit at its next
/// safe point. It does not roll back durable work and does not touch the audio
/// realtime callback.
#[derive(Debug, Clone)]
pub struct OperationHandle {
    state: Arc<OperationState>,
}

impl OperationHandle {
    /// Requests cancellation. The operation remains valid for reconciliation
    /// by its caller after this method returns.
    #[must_use]
    pub fn cancel(&self) -> CancellationOutcome {
        if self.state.cancelled.swap(true, Ordering::AcqRel) {
            CancellationOutcome::AlreadyRequested
        } else {
            CancellationOutcome::Requested
        }
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug)]
struct OperationState {
    cancelled: AtomicBool,
}

impl OperationState {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }
}

#[derive(Debug, Default)]
struct OperationRegistry {
    operations: Mutex<Vec<Weak<OperationState>>>,
}

impl OperationRegistry {
    fn register(&self) -> OperationHandle {
        let state = Arc::new(OperationState::new());
        let mut operations = lock_or_recover(&self.operations);
        operations.retain(|operation| operation.strong_count() != 0);
        operations.push(Arc::downgrade(&state));
        OperationHandle { state }
    }

    fn cancel_all(&self) {
        let operations = lock_or_recover(&self.operations);
        for operation in operations.iter().filter_map(Weak::upgrade) {
            operation.cancelled.store(true, Ordering::Release);
        }
    }
}

#[derive(Debug)]
struct CallbackState {
    inner: Mutex<CallbackInner>,
    drained: Condvar,
}

#[derive(Debug)]
struct CallbackInner {
    accepting: bool,
    active: usize,
}

impl CallbackState {
    fn new() -> Self {
        Self {
            inner: Mutex::new(CallbackInner {
                accepting: true,
                active: 0,
            }),
            drained: Condvar::new(),
        }
    }

    fn try_enter(self: &Arc<Self>) -> Option<CallbackGuard> {
        let mut inner = lock_or_recover(&self.inner);
        if !inner.accepting {
            return None;
        }
        inner.active = inner.active.saturating_add(1);
        Some(CallbackGuard {
            state: Arc::clone(self),
        })
    }

    fn close(&self, timeout: Duration) -> ShutdownOutcome {
        let mut inner = lock_or_recover(&self.inner);
        inner.accepting = false;
        if inner.active == 0 {
            return ShutdownOutcome::Complete;
        }
        if timeout.is_zero() {
            return ShutdownOutcome::Incomplete {
                active_callbacks: inner.active,
            };
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return ShutdownOutcome::Incomplete {
                    active_callbacks: inner.active,
                };
            }
            let (next_inner, wait_result) = self
                .drained
                .wait_timeout(inner, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            inner = next_inner;
            if inner.active == 0 {
                return ShutdownOutcome::Complete;
            }
            if wait_result.timed_out() {
                return ShutdownOutcome::Incomplete {
                    active_callbacks: inner.active,
                };
            }
        }
    }

    fn leave(&self) {
        let mut inner = lock_or_recover(&self.inner);
        inner.active = inner.active.saturating_sub(1);
        if inner.active == 0 {
            self.drained.notify_all();
        }
    }

    fn active(&self) -> usize {
        lock_or_recover(&self.inner).active
    }

    fn is_closed(&self) -> bool {
        !lock_or_recover(&self.inner).accepting
    }
}

/// A permit proving that one callback entered before the host's shutdown
/// fence. Dropping the permit marks that callback as quiescent.
#[must_use]
#[derive(Debug)]
pub struct CallbackGuard {
    state: Arc<CallbackState>,
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        self.state.leave();
    }
}

/// A clonable callback lifetime fence.
///
/// Producers must acquire a guard before invoking a callback. Once
/// [`CallbackFence::close`] returns `Complete`, no new guard can be acquired
/// and no prior callback remains in flight. This type intentionally provides
/// no callback scheduler: dispatching remains the responsibility of the FFI
/// or platform adapter, and realtime threads cannot use this boundary.
#[derive(Debug, Clone)]
pub struct CallbackFence {
    state: Arc<CallbackState>,
}

impl CallbackFence {
    /// Creates an open callback fence.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(CallbackState::new()),
        }
    }

    /// Attempts to enter the callback region.
    #[must_use]
    pub fn try_enter(&self) -> Option<CallbackGuard> {
        self.state.try_enter()
    }

    /// Closes callback admission and waits up to `timeout` for in-flight
    /// callbacks. A timeout leaves the fence closed and reports the remaining
    /// callback count so callers do not mistake it for a safe resource drop.
    #[must_use]
    pub fn close(&self, timeout: Duration) -> ShutdownOutcome {
        self.state.close(timeout)
    }

    /// Returns whether new callback entries are rejected.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    /// Returns the number of callbacks currently holding a guard.
    #[must_use]
    pub fn active_callbacks(&self) -> usize {
        self.state.active()
    }
}

impl Default for CallbackFence {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Running,
    ShuttingDown,
    Stopped,
}

impl From<Lifecycle> for CoreHostState {
    fn from(value: Lifecycle) -> Self {
        match value {
            Lifecycle::Running => Self::Running,
            Lifecycle::ShuttingDown => Self::ShuttingDown,
            Lifecycle::Stopped => Self::Stopped,
        }
    }
}

struct CoreServices {
    database: Database,
    playback: Mutex<PlaybackSession>,
}

/// The unique process-scoped composition root for the Rust Core.
///
/// Construction opens the configured local database, applies the existing
/// complete migration set, and creates one playback session using the shared
/// platform-neutral clock. It does not start an async runtime, platform audio
/// device, FFI server, or UI loop.
pub struct CoreHost {
    config: CoreHostConfig,
    services: Mutex<Option<CoreServices>>,
    lifecycle: Mutex<Lifecycle>,
    operations: OperationRegistry,
    callback_fence: CallbackFence,
}

impl CoreHost {
    /// Starts the single process-scoped host.
    ///
    /// The data directory is created before opening the database. Existing
    /// storage migrations remain the sole authority for schema changes.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostStartError`] when another host is active, the config
    /// is invalid, or an existing Core boundary cannot start.
    pub fn start(config: CoreHostConfig) -> Result<Self, CoreHostStartError> {
        HOST_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| CoreHostStartError::AlreadyRunning)?;

        let result = Self::start_inner(config);
        if result.is_err() {
            HOST_ACTIVE.store(false, Ordering::Release);
        }
        result
    }

    fn start_inner(config: CoreHostConfig) -> Result<Self, CoreHostStartError> {
        if config.data_dir.as_os_str().is_empty() {
            return Err(CoreHostStartError::InvalidConfiguration("data_dir"));
        }
        if config.shutdown_timeout.is_zero() {
            return Err(CoreHostStartError::InvalidConfiguration("shutdown_timeout"));
        }

        fs::create_dir_all(&config.data_dir)
            .map_err(|source| CoreHostStartError::CreateDataDirectory { source })?;
        let mut database = Database::open(config.database_path())
            .map_err(|source| CoreHostStartError::Database { source })?;
        database
            .apply_migrations(LOCAL_MIGRATIONS)
            .map_err(|source| CoreHostStartError::Migration { source })?;
        let playback = PlaybackSession::new(config.output_sample_rate())
            .map_err(|source| CoreHostStartError::Playback { source })?;

        Ok(Self {
            config,
            services: Mutex::new(Some(CoreServices {
                database,
                playback: Mutex::new(playback),
            })),
            lifecycle: Mutex::new(Lifecycle::Running),
            operations: OperationRegistry::default(),
            callback_fence: CallbackFence::new(),
        })
    }

    /// Returns the original startup configuration.
    #[must_use]
    pub const fn config(&self) -> &CoreHostConfig {
        &self.config
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub fn state(&self) -> CoreHostState {
        (*lock_or_recover(&self.lifecycle)).into()
    }

    /// Returns a clone of the host callback fence for an adapter that needs to
    /// establish callback admission and teardown ordering.
    #[must_use]
    pub fn callback_fence(&self) -> CallbackFence {
        self.callback_fence.clone()
    }

    /// Registers a cooperative non-realtime operation with this host.
    ///
    /// The registration is serialized with the transition into shutdown, so
    /// no operation can be admitted after shutdown has begun.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostAccessError::ShuttingDown`] or
    /// [`CoreHostAccessError::Stopped`] when the host no longer accepts work.
    pub fn register_operation(&self) -> Result<OperationHandle, CoreHostAccessError> {
        let lifecycle = lock_or_recover(&self.lifecycle);
        match *lifecycle {
            Lifecycle::Running => Ok(self.operations.register()),
            Lifecycle::ShuttingDown => Err(CoreHostAccessError::ShuttingDown),
            Lifecycle::Stopped => Err(CoreHostAccessError::Stopped),
        }
    }

    /// Runs a bounded read-only callback over the opened database while the
    /// host is running. The database connection remains owned by storage.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostAccessError::ShuttingDown`] or
    /// [`CoreHostAccessError::Stopped`] when the host no longer accepts work.
    pub fn with_database<T>(
        &self,
        operation: impl FnOnce(&Database) -> T,
    ) -> Result<T, CoreHostAccessError> {
        self.ensure_running()?;
        let services = lock_or_recover(&self.services);
        let Some(services) = services.as_ref() else {
            return Err(CoreHostAccessError::Stopped);
        };
        Ok(operation(&services.database))
    }

    /// Returns the current Core-owned playback snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostAccessError::ShuttingDown`] or
    /// [`CoreHostAccessError::Stopped`] when the host no longer accepts work.
    pub fn playback_snapshot(&self) -> Result<PlaybackSnapshot, CoreHostAccessError> {
        self.with_playback(PlaybackSession::snapshot)
    }

    /// Dispatches one existing playback command through the Core session.
    ///
    /// The host does not execute the returned worker intent. Worker scheduling
    /// belongs to the later audio/runtime adapter while the session remains
    /// the authority for command classification and state.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostAccessError::ShuttingDown`],
    /// [`CoreHostAccessError::Stopped`], [`CoreHostAccessError::StatePoisoned`],
    /// or [`CoreHostAccessError::Playback`] when the host cannot accept or
    /// classify the command.
    pub fn dispatch_playback(
        &self,
        command: PlaybackCommand,
    ) -> Result<SessionUpdate, CoreHostAccessError> {
        self.with_playback_mut(|playback| playback.dispatch(command))
    }

    /// Applies one existing non-realtime worker event to the Core session.
    ///
    /// # Errors
    ///
    /// Returns [`CoreHostAccessError::ShuttingDown`],
    /// [`CoreHostAccessError::Stopped`], [`CoreHostAccessError::StatePoisoned`],
    /// or [`CoreHostAccessError::Playback`] when the host cannot accept or
    /// apply the event.
    pub fn handle_worker_event(
        &self,
        event: WorkerEvent,
    ) -> Result<WorkerUpdate, CoreHostAccessError> {
        self.with_playback_mut(|playback| playback.handle_worker_event(event))
    }

    /// Requests a bounded shutdown using the configured timeout.
    #[must_use]
    pub fn shutdown(&self) -> ShutdownOutcome {
        self.shutdown_with_timeout(self.config.shutdown_timeout())
    }

    /// Requests a bounded shutdown using an explicit timeout.
    ///
    /// Shutdown first rejects new work, then cancels registered operations,
    /// closes the callback admission fence, and finally releases composed
    /// services only after the fence drains. A timed-out shutdown remains in
    /// `ShuttingDown`; callers may retry after workers have quiesced.
    #[must_use]
    pub fn shutdown_with_timeout(&self, timeout: Duration) -> ShutdownOutcome {
        {
            let mut lifecycle = lock_or_recover(&self.lifecycle);
            if *lifecycle == Lifecycle::Running {
                *lifecycle = Lifecycle::ShuttingDown;
            }
        }

        self.operations.cancel_all();
        let outcome = self.callback_fence.close(timeout);
        if outcome.is_complete() {
            let mut services = lock_or_recover(&self.services);
            services.take();
            *lock_or_recover(&self.lifecycle) = Lifecycle::Stopped;
            HOST_ACTIVE.store(false, Ordering::Release);
        }
        outcome
    }

    fn ensure_running(&self) -> Result<(), CoreHostAccessError> {
        match *lock_or_recover(&self.lifecycle) {
            Lifecycle::Running => Ok(()),
            Lifecycle::ShuttingDown => Err(CoreHostAccessError::ShuttingDown),
            Lifecycle::Stopped => Err(CoreHostAccessError::Stopped),
        }
    }

    fn with_playback<T>(
        &self,
        operation: impl FnOnce(&PlaybackSession) -> T,
    ) -> Result<T, CoreHostAccessError> {
        self.ensure_running()?;
        let services = lock_or_recover(&self.services);
        let Some(services) = services.as_ref() else {
            return Err(CoreHostAccessError::Stopped);
        };
        let playback = services
            .playback
            .lock()
            .map_err(|_| CoreHostAccessError::StatePoisoned)?;
        Ok(operation(&playback))
    }

    fn with_playback_mut<T>(
        &self,
        operation: impl FnOnce(&mut PlaybackSession) -> Result<T, SessionError>,
    ) -> Result<T, CoreHostAccessError> {
        self.ensure_running()?;
        let services = lock_or_recover(&self.services);
        let Some(services) = services.as_ref() else {
            return Err(CoreHostAccessError::Stopped);
        };
        let mut playback = services
            .playback
            .lock()
            .map_err(|_| CoreHostAccessError::StatePoisoned)?;
        operation(&mut playback).map_err(CoreHostAccessError::Playback)
    }
}

impl Drop for CoreHost {
    fn drop(&mut self) {
        if !matches!(self.state(), CoreHostState::Stopped) {
            let _ = self.shutdown();
        }
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Serializes tests that intentionally exercise the process-wide singleton.
#[cfg(test)]
fn test_data_dir(name: &str) -> PathBuf {
    static NEXT_ID: std::sync::OnceLock<Mutex<u64>> = std::sync::OnceLock::new();
    let next_id = NEXT_ID.get_or_init(|| Mutex::new(0));
    let mut id = lock_or_recover(next_id);
    *id += 1;
    std::env::temp_dir().join(format!(
        "aurorix-runtime-{name}-{}-{}",
        std::process::id(),
        *id
    ))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier, Mutex, OnceLock},
        thread,
    };

    use super::{
        CallbackFence, CancellationOutcome, CoreHost, CoreHostAccessError, CoreHostConfig,
        CoreHostStartError, CoreHostState, Database, ShutdownOutcome, test_data_dir,
    };

    fn singleton_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn cleanup(path: &std::path::Path) {
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn callback_fence_rejects_new_entries_after_close() {
        let fence = CallbackFence::new();
        let guard = fence.try_enter().expect("callback enters while open");
        assert_eq!(fence.active_callbacks(), 1);
        let outcome = fence.close(std::time::Duration::ZERO);
        assert_eq!(
            outcome,
            ShutdownOutcome::Incomplete {
                active_callbacks: 1
            }
        );
        assert!(fence.is_closed());
        assert!(fence.try_enter().is_none());
        drop(guard);
        assert_eq!(fence.active_callbacks(), 0);
        assert_eq!(
            fence.close(std::time::Duration::from_millis(1)),
            ShutdownOutcome::Complete
        );
    }

    #[test]
    fn callback_fence_waits_for_in_flight_callback_without_post_fence_entry() {
        let fence = CallbackFence::new();
        let guard = fence.try_enter().expect("callback enters");
        let worker_fence = fence.clone();
        let worker = thread::spawn(move || {
            assert!(worker_fence.try_enter().is_none());
        });
        let release = thread::spawn(move || {
            thread::sleep(std::time::Duration::from_millis(10));
            drop(guard);
        });
        let outcome = fence.close(std::time::Duration::from_secs(1));
        worker.join().expect("entry probe joins");
        release.join().expect("callback joins");
        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert_eq!(fence.active_callbacks(), 0);
    }

    #[test]
    fn host_is_singleton_and_releases_after_explicit_shutdown() {
        let _serial = singleton_test_lock().lock().expect("singleton test lock");
        let data_dir = test_data_dir("singleton");
        let host = CoreHost::start(CoreHostConfig::new(&data_dir)).expect("host starts");
        assert_eq!(host.state(), CoreHostState::Running);
        let second = CoreHost::start(CoreHostConfig::new(test_data_dir("second")));
        assert!(matches!(second, Err(CoreHostStartError::AlreadyRunning)));
        assert_eq!(host.shutdown(), ShutdownOutcome::Complete);
        assert_eq!(host.state(), CoreHostState::Stopped);
        let next_dir = test_data_dir("next");
        let next = CoreHost::start(CoreHostConfig::new(&next_dir)).expect("next host starts");
        assert_eq!(next.shutdown(), ShutdownOutcome::Complete);
        cleanup(&data_dir);
        cleanup(&next_dir);
    }

    #[test]
    fn shutdown_cancels_registered_operations_and_rejects_new_work() {
        let _serial = singleton_test_lock().lock().expect("singleton test lock");
        let data_dir = test_data_dir("cancel");
        let host = CoreHost::start(CoreHostConfig::new(&data_dir)).expect("host starts");
        let operation = host.register_operation().expect("operation registers");
        assert_eq!(operation.cancel(), CancellationOutcome::Requested);
        assert_eq!(operation.cancel(), CancellationOutcome::AlreadyRequested);
        let second_operation = host
            .register_operation()
            .expect("second operation registers");
        assert!(!second_operation.is_cancelled());
        assert_eq!(host.shutdown(), ShutdownOutcome::Complete);
        assert!(operation.is_cancelled());
        assert!(second_operation.is_cancelled());
        assert!(matches!(
            host.register_operation(),
            Err(CoreHostAccessError::Stopped)
        ));
        cleanup(&data_dir);
    }

    #[test]
    fn shutdown_timeout_is_retryable_and_does_not_release_services_early() {
        let _serial = singleton_test_lock().lock().expect("singleton test lock");
        let data_dir = test_data_dir("timeout");
        let host = Arc::new(CoreHost::start(CoreHostConfig::new(&data_dir)).expect("host starts"));
        let callback = host.callback_fence().try_enter().expect("callback enters");
        let barrier = Arc::new(Barrier::new(2));
        let release_barrier = Arc::clone(&barrier);
        let release = thread::spawn(move || {
            release_barrier.wait();
            thread::sleep(std::time::Duration::from_millis(20));
            drop(callback);
        });
        barrier.wait();
        let incomplete = host.shutdown_with_timeout(std::time::Duration::from_millis(1));
        assert!(matches!(
            incomplete,
            ShutdownOutcome::Incomplete {
                active_callbacks: 1
            }
        ));
        assert_eq!(host.state(), CoreHostState::ShuttingDown);
        release.join().expect("callback release joins");
        assert_eq!(
            host.shutdown_with_timeout(std::time::Duration::from_secs(1)),
            ShutdownOutcome::Complete
        );
        assert_eq!(host.state(), CoreHostState::Stopped);
        cleanup(&data_dir);
    }

    #[test]
    fn host_composes_existing_database_and_playback_boundaries() {
        let _serial = singleton_test_lock().lock().expect("singleton test lock");
        let data_dir = test_data_dir("composition");
        let host = CoreHost::start(CoreHostConfig::new(&data_dir)).expect("host starts");
        let capabilities = host
            .with_database(Database::capabilities)
            .expect("database is available");
        assert!(capabilities.fts5());
        assert!(capabilities.wal());
        let snapshot = host.playback_snapshot().expect("playback is available");
        assert_eq!(
            snapshot.state(),
            aurorix_playback::session::SessionState::Empty
        );
        assert_eq!(host.shutdown(), ShutdownOutcome::Complete);
        cleanup(&data_dir);
    }
}
