//! Public-safe transport primitives and the narrow Gate 3 C facade.
//!
//! The C ABI deliberately exposes only bounded transport bytes and opaque
//! lifetime handles. Core domain structs, database rows, runtime leases,
//! realtime state, and platform handles never cross this boundary.

use std::{
    cell::Cell,
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

thread_local! {
    static IN_FFI_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

use aurorix_runtime::{CoreHost, CoreHostConfig};

pub mod transport;

pub use transport::{
    ExtensionRequest, FfiError, FfiRequest, FfiResponse, MAX_EVENT_BYTES, MAX_MESSAGE_BYTES,
    RequestBody, ResponseBody, SCHEMA_MAJOR, TransportError,
};

/// Successful operation status.
pub const AURORIX_STATUS_OK: i32 = 0;
/// A caller supplied pointer, length, or request was invalid.
pub const AURORIX_STATUS_INVALID_ARGUMENT: i32 = 1;
/// The supplied opaque handle is null or no longer usable.
pub const AURORIX_STATUS_INVALID_HANDLE: i32 = 2;
/// The request used an unsupported transport schema.
pub const AURORIX_STATUS_INCOMPATIBLE_VERSION: i32 = 3;
/// The Core host is shutting down or has stopped.
pub const AURORIX_STATUS_SHUTDOWN: i32 = 4;
/// The operation cancellation request was already made.
pub const AURORIX_STATUS_ALREADY_CANCELLED: i32 = 5;
/// Cancellation happened before a non-durable operation committed.
pub const AURORIX_STATUS_CANCELLED: i32 = 6;
/// A callback was invoked from a foreign thread and must return promptly.
pub const AURORIX_STATUS_CALLBACK_REJECTED: i32 = 7;
/// A Rust panic was contained at the ABI boundary.
pub const AURORIX_STATUS_PANIC: i32 = 8;
/// Shutdown could not drain callbacks within its configured timeout.
pub const AURORIX_STATUS_SHUTDOWN_INCOMPLETE: i32 = 9;
/// A release/cancel call was attempted reentrantly from its own callback.
pub const AURORIX_STATUS_REENTRANT_RELEASE: i32 = 10;

/// The accepted operation completed its transport request.
pub const AURORIX_OUTCOME_COMPLETED: i32 = 0;
/// The operation was cancelled before its non-durable work took effect.
pub const AURORIX_OUTCOME_CANCELLED_BEFORE_COMMIT: i32 = 1;
/// A durable command's final outcome would require reconciliation by key.
pub const AURORIX_OUTCOME_CANCELLED_OUTCOME_UNKNOWN: i32 = 2;

/// Maximum path/configuration byte length accepted by the bootstrap ABI.
pub const MAX_CONFIG_BYTES: usize = 32 * 1024;

/// A borrowed byte range. The pointer is read only for the duration of the
/// invoking ABI call and is never retained by Rust.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AurorixByteSliceV1 {
    pub ptr: *const u8,
    pub len: u64,
}

/// A Rust-owned byte range returned to a foreign caller. It must be released
/// exactly once with [`aurorix_buffer_free_v1`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AurorixBufferV1 {
    pub ptr: *mut u8,
    pub len: u64,
}

/// Configuration for one process-scoped Core client.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AurorixClientConfigV1 {
    pub data_dir: AurorixByteSliceV1,
    pub shutdown_timeout_ms: u32,
}

/// A bounded diagnostic returned by a create call.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AurorixErrorV1 {
    pub code: i32,
    pub message: AurorixBufferV1,
}

/// Completion callback. Response bytes are borrowed until this callback
/// returns and must be copied by the foreign caller. The callback is never
/// invoked on the realtime or database-write path.
pub type AurorixCompletionV1 = Option<
    extern "C" fn(context: *mut c_void, status: i32, outcome: i32, response: AurorixByteSliceV1),
>;

/// Event callback. Event bytes are borrowed until this callback returns.
pub type AurorixEventSinkV1 =
    Option<extern "C" fn(context: *mut c_void, event_sequence: u64, event: AurorixByteSliceV1)>;

/// Opaque process client handle.
#[repr(C)]
pub struct AurorixClientHandle {
    inner: Arc<ClientInner>,
}

/// Opaque asynchronous operation handle.
#[repr(C)]
pub struct AurorixOperationHandle {
    inner: Arc<OperationState>,
}

/// Opaque event subscription handle.
#[repr(C)]
pub struct AurorixSubscriptionHandle {
    inner: Arc<SubscriptionState>,
}

struct ClientInner {
    host: CoreHost,
    operations: Mutex<Vec<Weak<OperationState>>>,
    subscriptions: Mutex<Vec<Weak<SubscriptionState>>>,
    event_sequence: AtomicU64,
    shutdown: AtomicBool,
    admission: Mutex<()>,
}

struct Callback {
    function: AurorixCompletionV1,
    context: usize,
}

struct EventCallback {
    function: AurorixEventSinkV1,
    context: usize,
}

struct OperationState {
    client: Arc<ClientInner>,
    callback: Callback,
    cancelled: AtomicBool,
    completed: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

struct SubscriptionState {
    client: Arc<ClientInner>,
    callback: EventCallback,
    cancelled: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ClientInner {
    fn register_operation(&self, operation: &Arc<OperationState>) {
        let mut operations = lock_recover(&self.operations);
        operations.retain(|candidate| candidate.strong_count() != 0);
        operations.push(Arc::downgrade(operation));
    }

    fn register_subscription(&self, subscription: &Arc<SubscriptionState>) {
        let mut subscriptions = lock_recover(&self.subscriptions);
        subscriptions.retain(|candidate| candidate.strong_count() != 0);
        subscriptions.push(Arc::downgrade(subscription));
    }

    fn shutdown(&self) -> i32 {
        let (operations, subscriptions) = {
            let _admission = lock_recover(&self.admission);
            if self.shutdown.swap(true, Ordering::AcqRel) {
                (Vec::new(), Vec::new())
            } else {
                let operations = lock_recover(&self.operations)
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>();
                let subscriptions = lock_recover(&self.subscriptions)
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>();
                (operations, subscriptions)
            }
        };
        for operation in operations {
            operation.cancel_and_join();
        }
        for subscription in subscriptions {
            subscription.cancel_and_join();
        }

        match self.host.shutdown() {
            outcome if outcome.is_complete() => AURORIX_STATUS_OK,
            _ => AURORIX_STATUS_SHUTDOWN_INCOMPLETE,
        }
    }

    fn next_event_sequence(&self) -> u64 {
        self.event_sequence.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn callback_enter(&self) -> Option<aurorix_runtime::CallbackGuard> {
        self.host.callback_fence().try_enter()
    }
}

impl OperationState {
    fn cancel(&self) -> i32 {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            AURORIX_STATUS_ALREADY_CANCELLED
        } else {
            AURORIX_STATUS_OK
        }
    }

    fn cancel_and_join(&self) {
        self.cancelled.store(true, Ordering::Release);
        join_worker(&self.worker);
    }

    fn complete(&self, status: i32, outcome: i32, response: &[u8]) {
        if self.completed.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(function) = self.callback.function else {
            return;
        };
        let Some(_guard) = self.client.callback_enter() else {
            return;
        };
        let callback = self.callback.context as *mut c_void;
        let response = AurorixByteSliceV1 {
            ptr: response.as_ptr(),
            len: response.len() as u64,
        };
        // The foreign callback is part of the explicitly audited C ABI
        // boundary. Its byte slice is borrowed and the guard fences teardown.
        IN_FFI_CALLBACK.with(|active| {
            let previous = active.replace(true);
            function(callback, status, outcome, response);
            active.set(previous);
        });
    }
}

impl SubscriptionState {
    fn cancel(&self) -> i32 {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            AURORIX_STATUS_ALREADY_CANCELLED
        } else {
            AURORIX_STATUS_OK
        }
    }

    fn cancel_and_join(&self) {
        self.cancelled.store(true, Ordering::Release);
        join_worker(&self.worker);
    }

    fn emit(&self, sequence: u64, event: &[u8]) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let Some(function) = self.callback.function else {
            return;
        };
        let Some(_guard) = self.client.callback_enter() else {
            return;
        };
        let callback = self.callback.context as *mut c_void;
        let event = AurorixByteSliceV1 {
            ptr: event.as_ptr(),
            len: event.len() as u64,
        };
        IN_FFI_CALLBACK.with(|active| {
            let previous = active.replace(true);
            function(callback, sequence, event);
            active.set(previous);
        });
    }
}

fn join_worker(worker: &Mutex<Option<JoinHandle<()>>>) {
    let Some(worker) = lock_recover(worker).take() else {
        return;
    };
    let _ = worker.join();
}

fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn bytes_from_slice(slice: AurorixByteSliceV1, max: usize) -> Result<Vec<u8>, i32> {
    let length = usize::try_from(slice.len).map_err(|_| AURORIX_STATUS_INVALID_ARGUMENT)?;
    if length > max {
        return Err(AURORIX_STATUS_INVALID_ARGUMENT);
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    if slice.ptr.is_null() {
        return Err(AURORIX_STATUS_INVALID_ARGUMENT);
    }
    // Copy at the ABI edge so no borrowed foreign memory can outlive this
    // call, even if a future parser implementation retains an input view.
    Ok(unsafe { std::slice::from_raw_parts(slice.ptr, length) }.to_vec())
}

fn owned_buffer(bytes: Vec<u8>) -> AurorixBufferV1 {
    let boxed = bytes.into_boxed_slice();
    let len = boxed.len() as u64;
    let ptr = if len == 0 {
        std::ptr::null_mut()
    } else {
        Box::into_raw(boxed).cast::<u8>()
    };
    AurorixBufferV1 { ptr, len }
}

fn set_error(error: *mut AurorixErrorV1, code: i32, message: &str) {
    if error.is_null() {
        return;
    }
    unsafe {
        (*error).code = code;
        (*error).message = owned_buffer(message.as_bytes().to_vec());
    }
}

fn clear_error(error: *mut AurorixErrorV1) {
    if error.is_null() {
        return;
    }
    unsafe {
        (*error).code = AURORIX_STATUS_OK;
        (*error).message = AurorixBufferV1 {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
    }
}

fn parse_request(request: AurorixByteSliceV1) -> Result<FfiRequest, i32> {
    let bytes = bytes_from_slice(request, MAX_MESSAGE_BYTES)?;
    FfiRequest::decode(&bytes).map_err(|error| match error {
        TransportError::UnsupportedSchema { .. } => AURORIX_STATUS_INCOMPATIBLE_VERSION,
        _ => AURORIX_STATUS_INVALID_ARGUMENT,
    })
}

fn response_for(request: &FfiRequest) -> Vec<u8> {
    let response = match request.body() {
        RequestBody::Ping => FfiResponse::new(request.request_id(), ResponseBody::Pong),
        RequestBody::Extension(_) => FfiResponse::new(
            request.request_id(),
            ResponseBody::Error(
                FfiError::new(
                    "unsupported_operation",
                    "bootstrap extension is not executable",
                    false,
                )
                .expect("static error is valid"),
            ),
        ),
    };
    response
        .expect("validated bootstrap response is valid")
        .encode()
        .expect("response encodes")
}

fn create_client_inner(
    config: *const AurorixClientConfigV1,
) -> Result<Box<AurorixClientHandle>, (i32, &'static str)> {
    if config.is_null() {
        return Err((AURORIX_STATUS_INVALID_ARGUMENT, "config is null"));
    }
    let config = unsafe { *config };
    let data_dir = bytes_from_slice(config.data_dir, MAX_CONFIG_BYTES)
        .map_err(|_| (AURORIX_STATUS_INVALID_ARGUMENT, "data_dir is invalid"))?;
    let data_dir = std::str::from_utf8(&data_dir)
        .map_err(|_| (AURORIX_STATUS_INVALID_ARGUMENT, "data_dir must be UTF-8"))?;
    if data_dir.trim().is_empty() {
        return Err((AURORIX_STATUS_INVALID_ARGUMENT, "data_dir is empty"));
    }
    let timeout = if config.shutdown_timeout_ms == 0 {
        Duration::from_secs(5)
    } else {
        Duration::from_millis(u64::from(config.shutdown_timeout_ms))
    };
    let host = CoreHost::start(
        CoreHostConfig::new(PathBuf::from(data_dir)).with_shutdown_timeout(timeout),
    )
    .map_err(|_| (AURORIX_STATUS_SHUTDOWN, "CoreHost could not start"))?;
    let client = Arc::new(ClientInner {
        host,
        operations: Mutex::new(Vec::new()),
        subscriptions: Mutex::new(Vec::new()),
        event_sequence: AtomicU64::new(0),
        shutdown: AtomicBool::new(false),
        admission: Mutex::new(()),
    });
    Ok(Box::new(AurorixClientHandle { inner: client }))
}

fn spawn_operation(
    client: &Arc<ClientInner>,
    request: FfiRequest,
    callback: AurorixCompletionV1,
    context: *mut c_void,
) -> AurorixOperationHandle {
    let state = Arc::new(OperationState {
        client: Arc::clone(client),
        callback: Callback {
            function: callback,
            context: context as usize,
        },
        cancelled: AtomicBool::new(false),
        completed: AtomicBool::new(false),
        worker: Mutex::new(None),
    });
    // Store the worker after the Arc exists so the thread can hold its own
    // state reference without exposing a raw pointer to foreign code.
    let worker_state = Arc::clone(&state);
    let worker = thread::spawn(move || {
        thread::yield_now();
        if worker_state.cancelled.load(Ordering::Acquire) {
            worker_state.complete(
                AURORIX_STATUS_CANCELLED,
                AURORIX_OUTCOME_CANCELLED_BEFORE_COMMIT,
                &[],
            );
            return;
        }
        let response = response_for(&request);
        if worker_state.cancelled.load(Ordering::Acquire) {
            worker_state.complete(
                AURORIX_STATUS_CANCELLED,
                AURORIX_OUTCOME_CANCELLED_BEFORE_COMMIT,
                &[],
            );
        } else {
            worker_state.complete(AURORIX_STATUS_OK, AURORIX_OUTCOME_COMPLETED, &response);
        }
    });
    *lock_recover(&state.worker) = Some(worker);
    client.register_operation(&state);
    let _ = state.client.host.state();
    AurorixOperationHandle { inner: state }
}

fn spawn_subscription(
    client: &Arc<ClientInner>,
    callback: AurorixEventSinkV1,
    context: *mut c_void,
    observed_sequence: u64,
) -> AurorixSubscriptionHandle {
    let state = Arc::new(SubscriptionState {
        client: Arc::clone(client),
        callback: EventCallback {
            function: callback,
            context: context as usize,
        },
        cancelled: AtomicBool::new(false),
        worker: Mutex::new(None),
    });
    let worker_state = Arc::clone(&state);
    let worker = thread::spawn(move || {
        thread::yield_now();
        if !worker_state.cancelled.load(Ordering::Acquire) {
            let sequence = worker_state.client.next_event_sequence();
            if sequence > observed_sequence {
                worker_state.emit(sequence, &[]);
            }
        }
    });
    *lock_recover(&state.worker) = Some(worker);
    client.register_subscription(&state);
    AurorixSubscriptionHandle { inner: state }
}

/// Creates one process-scoped client and its `CoreHost`.
///
/// # Safety
///
/// `config` must be null or point to a readable `AurorixClientConfigV1`, and
/// `error` must be null or point to writable caller-owned storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_client_create_v1(
    config: *const AurorixClientConfigV1,
    error: *mut AurorixErrorV1,
) -> *mut AurorixClientHandle {
    clear_error(error);
    match panic::catch_unwind(AssertUnwindSafe(|| create_client_inner(config))) {
        Ok(Ok(client)) => Box::into_raw(client),
        Ok(Err((code, message))) => {
            set_error(error, code, message);
            std::ptr::null_mut()
        }
        Err(_) => {
            set_error(error, AURORIX_STATUS_PANIC, "CoreHost creation panicked");
            std::ptr::null_mut()
        }
    }
}

/// Starts a bounded asynchronous command over the bootstrap transport.
///
/// # Safety
///
/// `client` and `operation` must be valid pointers for the duration of this
/// call. `request` must point to readable bytes for its declared length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_client_command_v1(
    client: *mut AurorixClientHandle,
    request: AurorixByteSliceV1,
    callback: AurorixCompletionV1,
    context: *mut c_void,
    operation: *mut *mut AurorixOperationHandle,
) -> i32 {
    start_operation(client, request, callback, context, operation)
}

/// Starts a bounded asynchronous query over the bootstrap transport.
///
/// # Safety
///
/// `client` and `operation` must be valid pointers for the duration of this
/// call. `request` must point to readable bytes for its declared length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_client_query_v1(
    client: *mut AurorixClientHandle,
    request: AurorixByteSliceV1,
    callback: AurorixCompletionV1,
    context: *mut c_void,
    operation: *mut *mut AurorixOperationHandle,
) -> i32 {
    start_operation(client, request, callback, context, operation)
}

fn start_operation(
    client: *mut AurorixClientHandle,
    request: AurorixByteSliceV1,
    callback: AurorixCompletionV1,
    context: *mut c_void,
    operation: *mut *mut AurorixOperationHandle,
) -> i32 {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || operation.is_null() {
            return AURORIX_STATUS_INVALID_ARGUMENT;
        }
        unsafe { *operation = std::ptr::null_mut() };
        let client = unsafe { &*client };
        let _admission = lock_recover(&client.inner.admission);
        if client.inner.shutdown.load(Ordering::Acquire) {
            return AURORIX_STATUS_SHUTDOWN;
        }
        let request = match parse_request(request) {
            Ok(request) => request,
            Err(status) => return status,
        };
        let handle = spawn_operation(&client.inner, request, callback, context);
        unsafe { *operation = Box::into_raw(Box::new(handle)) };
        AURORIX_STATUS_OK
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Subscribes to the bootstrap event stream and returns the activation barrier.
///
/// # Safety
///
/// `client`, `subscription`, and `observed_sequence` must be valid pointers
/// for the duration of this call. `request` must point to readable bytes for
/// its declared length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_client_subscribe_v1(
    client: *mut AurorixClientHandle,
    request: AurorixByteSliceV1,
    callback: AurorixEventSinkV1,
    context: *mut c_void,
    subscription: *mut *mut AurorixSubscriptionHandle,
    observed_sequence: *mut u64,
) -> i32 {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() || subscription.is_null() || observed_sequence.is_null() {
            return AURORIX_STATUS_INVALID_ARGUMENT;
        }
        unsafe {
            *subscription = std::ptr::null_mut();
            *observed_sequence = 0;
        }
        let client = unsafe { &*client };
        let _admission = lock_recover(&client.inner.admission);
        if client.inner.shutdown.load(Ordering::Acquire) {
            return AURORIX_STATUS_SHUTDOWN;
        }
        if let Err(status) = parse_request(request) {
            return status;
        }
        let observed = client.inner.event_sequence.load(Ordering::Acquire);
        let handle = spawn_subscription(&client.inner, callback, context, observed);
        unsafe {
            *observed_sequence = observed;
            *subscription = Box::into_raw(Box::new(handle));
        }
        AURORIX_STATUS_OK
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Requests cooperative operation cancellation. It is idempotent.
///
/// # Safety
///
/// `operation` must be a live handle returned by this crate and must not be
/// used concurrently with its release function.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_operation_cancel_v1(
    operation: *mut AurorixOperationHandle,
) -> i32 {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if operation.is_null() {
            return AURORIX_STATUS_INVALID_HANDLE;
        }
        unsafe { (&*operation).inner.cancel() }
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Cancels an event sink and waits for its no-more-callback fence.
///
/// # Safety
///
/// `subscription` must be a live handle returned by this crate and must not
/// be used concurrently with its release function or reentrantly by its sink.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_subscription_cancel_v1(
    subscription: *mut AurorixSubscriptionHandle,
) -> i32 {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if subscription.is_null() {
            return AURORIX_STATUS_INVALID_HANDLE;
        }
        if IN_FFI_CALLBACK.with(Cell::get) {
            return AURORIX_STATUS_REENTRANT_RELEASE;
        }
        let state = unsafe { &*subscription }.inner.clone();
        let status = state.cancel();
        state.cancel_and_join();
        status
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Releases an operation handle after cancellation and callback drain.
///
/// # Safety
///
/// `operation` must be a live handle returned by this crate and must not be
/// used after this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_operation_release_v1(
    operation: *mut AurorixOperationHandle,
) -> i32 {
    if operation.is_null() {
        return AURORIX_STATUS_INVALID_HANDLE;
    }
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if IN_FFI_CALLBACK.with(Cell::get) {
            return AURORIX_STATUS_REENTRANT_RELEASE;
        }
        let handle = unsafe { Box::from_raw(operation) };
        handle.inner.cancel_and_join();
        AURORIX_STATUS_OK
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Releases a subscription handle after its no-more-callback fence.
///
/// # Safety
///
/// `subscription` must be a live handle returned by this crate and must not
/// be used after this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_subscription_release_v1(
    subscription: *mut AurorixSubscriptionHandle,
) -> i32 {
    if subscription.is_null() {
        return AURORIX_STATUS_INVALID_HANDLE;
    }
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if IN_FFI_CALLBACK.with(Cell::get) {
            return AURORIX_STATUS_REENTRANT_RELEASE;
        }
        let handle = unsafe { Box::from_raw(subscription) };
        handle.inner.cancel_and_join();
        AURORIX_STATUS_OK
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Frees a buffer allocated by Rust. Foreign allocators must not free it.
///
/// # Safety
///
/// `buffer` must be a value returned by this crate and must be released at
/// most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_buffer_free_v1(buffer: AurorixBufferV1) {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return;
    }
    if let Ok(length) = usize::try_from(buffer.len) {
        unsafe {
            let slice = std::ptr::slice_from_raw_parts_mut(buffer.ptr, length);
            drop(Box::from_raw(slice));
        }
    }
}

/// Shuts down the Core host, cancels work, and fences callbacks.
///
/// # Safety
///
/// `client` must be a live handle returned by this crate.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_client_shutdown_v1(client: *mut AurorixClientHandle) -> i32 {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if client.is_null() {
            return AURORIX_STATUS_INVALID_HANDLE;
        }
        if IN_FFI_CALLBACK.with(Cell::get) {
            return AURORIX_STATUS_REENTRANT_RELEASE;
        }
        unsafe { (&*client).inner.shutdown() }
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

/// Releases the client and implicitly fences all outstanding work.
///
/// # Safety
///
/// `client` must be a live handle returned by this crate and must not be used
/// after this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aurorix_client_release_v1(client: *mut AurorixClientHandle) -> i32 {
    if client.is_null() {
        return AURORIX_STATUS_INVALID_HANDLE;
    }
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        if IN_FFI_CALLBACK.with(Cell::get) {
            return AURORIX_STATUS_REENTRANT_RELEASE;
        }
        let handle = unsafe { Box::from_raw(client) };
        let status = handle.inner.shutdown();
        if status == AURORIX_STATUS_OK {
            AURORIX_STATUS_OK
        } else {
            status
        }
    }));
    result.unwrap_or(AURORIX_STATUS_PANIC)
}

#[cfg(test)]
mod ffi_smoke_tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Mutex, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use super::*;

    fn test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn test_data_dir() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "aurorix-ffi-c-smoke-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    extern "C" fn completion_callback(
        _context: *mut c_void,
        status: i32,
        outcome: i32,
        response: AurorixByteSliceV1,
    ) {
        assert_eq!(status, AURORIX_STATUS_OK);
        assert_eq!(outcome, AURORIX_OUTCOME_COMPLETED);
        assert!(response.len > 0);
        COMPLETIONS.fetch_add(1, Ordering::AcqRel);
    }

    extern "C" fn event_callback(_context: *mut c_void, sequence: u64, event: AurorixByteSliceV1) {
        assert!(sequence > 0);
        assert_eq!(event.len, 0);
        EVENTS.fetch_add(1, Ordering::AcqRel);
    }

    static COMPLETIONS: AtomicUsize = AtomicUsize::new(0);
    static EVENTS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn c_abi_bootstrap_completes_and_fences_subscription_callbacks() {
        let _guard = test_lock().lock().expect("smoke lock is not poisoned");
        COMPLETIONS.store(0, Ordering::Release);
        EVENTS.store(0, Ordering::Release);
        let data_dir = test_data_dir();
        let path = data_dir.to_string_lossy().as_bytes().to_vec();
        let config = AurorixClientConfigV1 {
            data_dir: AurorixByteSliceV1 {
                ptr: path.as_ptr(),
                len: path.len() as u64,
            },
            shutdown_timeout_ms: 2_000,
        };
        let mut error = AurorixErrorV1 {
            code: -1,
            message: AurorixBufferV1 {
                ptr: std::ptr::null_mut(),
                len: 0,
            },
        };
        let client = unsafe { aurorix_client_create_v1(&raw const config, &raw mut error) };
        assert!(!client.is_null());

        let request = FfiRequest::new("ffi-smoke", RequestBody::Ping)
            .expect("smoke request is valid")
            .encode()
            .expect("smoke request encodes");
        let request = AurorixByteSliceV1 {
            ptr: request.as_ptr(),
            len: request.len() as u64,
        };
        let mut operation = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                aurorix_client_command_v1(
                    client,
                    request,
                    Some(completion_callback),
                    std::ptr::null_mut(),
                    &raw mut operation,
                )
            },
            AURORIX_STATUS_OK
        );
        thread::sleep(Duration::from_millis(25));
        assert_eq!(
            unsafe { aurorix_operation_release_v1(operation) },
            AURORIX_STATUS_OK
        );
        assert_eq!(COMPLETIONS.load(Ordering::Acquire), 1);

        let mut subscription = std::ptr::null_mut();
        let mut observed = u64::MAX;
        assert_eq!(
            unsafe {
                aurorix_client_subscribe_v1(
                    client,
                    request,
                    Some(event_callback),
                    std::ptr::null_mut(),
                    &raw mut subscription,
                    &raw mut observed,
                )
            },
            AURORIX_STATUS_OK
        );
        assert_eq!(observed, 0);
        thread::sleep(Duration::from_millis(25));
        assert_eq!(EVENTS.load(Ordering::Acquire), 1);
        assert_eq!(
            unsafe { aurorix_subscription_cancel_v1(subscription) },
            AURORIX_STATUS_OK
        );
        assert_eq!(
            unsafe { aurorix_subscription_cancel_v1(subscription) },
            AURORIX_STATUS_ALREADY_CANCELLED
        );
        assert_eq!(
            unsafe { aurorix_subscription_release_v1(subscription) },
            AURORIX_STATUS_OK
        );
        assert_eq!(
            unsafe { aurorix_client_shutdown_v1(client) },
            AURORIX_STATUS_OK
        );
        assert_eq!(
            unsafe { aurorix_client_release_v1(client) },
            AURORIX_STATUS_OK
        );
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn invalid_create_is_contained_and_reports_a_bounded_error() {
        let mut error = AurorixErrorV1 {
            code: -1,
            message: AurorixBufferV1 {
                ptr: std::ptr::null_mut(),
                len: 0,
            },
        };
        let client = unsafe { aurorix_client_create_v1(std::ptr::null(), &raw mut error) };
        assert!(client.is_null());
        assert_eq!(error.code, AURORIX_STATUS_INVALID_ARGUMENT);
        let message = AurorixBufferV1 {
            ptr: error.message.ptr,
            len: error.message.len,
        };
        unsafe { aurorix_buffer_free_v1(message) };
    }
}
