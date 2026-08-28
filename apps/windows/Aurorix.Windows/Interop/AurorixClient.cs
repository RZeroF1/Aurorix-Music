using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;

namespace Aurorix.Windows.Interop;

/// <summary>
/// A transport completion projected from the bounded Rust facade envelope.
/// Response bytes are owned by this managed value.
/// </summary>
public sealed record AurorixCompletion(int Status, int Outcome, byte[] Response)
{
    public bool IsSuccess => Status == AurorixNativeMethods.StatusOk;

    public bool IsCancelledBeforeCommit =>
        Outcome == AurorixNativeMethods.OutcomeCancelledBeforeCommit;

    public bool IsCancelledOutcomeUnknown =>
        Outcome == AurorixNativeMethods.OutcomeCancelledOutcomeUnknown;
}

/// <summary>
/// One copied event from a Rust subscription. A sequence gap requires the
/// caller to request a fresh snapshot through the facade.
/// </summary>
public sealed record AurorixEvent(ulong Sequence, byte[] Payload, bool RequiresResync);

/// <summary>
/// Thin C# facade over the x64 Rust C ABI. XAML and ViewModels consume this
/// class rather than importing native symbols directly.
/// </summary>
public sealed class AurorixClient : IDisposable
{
    private readonly AurorixClientSafeHandle _handle;
    private readonly DispatcherQueue? _dispatcher;
    private int _disposed;

    private AurorixClient(AurorixClientSafeHandle handle, DispatcherQueue? dispatcher)
    {
        _handle = handle;
        _dispatcher = dispatcher;
    }

    public static AurorixClient Create(
        string dataDirectory,
        TimeSpan? shutdownTimeout = null,
        DispatcherQueue? dispatcher = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(dataDirectory);
        var encodedPath = System.Text.Encoding.UTF8.GetBytes(dataDirectory);
        var pinnedPath = GCHandle.Alloc(encodedPath, GCHandleType.Pinned);
        try
        {
            var config = new AurorixNativeMethods.ClientConfig
            {
                DataDirectory = new AurorixNativeMethods.ByteSlice(
                    encodedPath.Length == 0 ? IntPtr.Zero : pinnedPath.AddrOfPinnedObject(),
                    (ulong)encodedPath.Length),
                ShutdownTimeoutMilliseconds = ToTimeoutMilliseconds(shutdownTimeout),
            };
            var nativeHandle = AurorixNativeMethods.aurorix_client_create_v1(in config, out var error);
            if (nativeHandle == IntPtr.Zero)
            {
                throw new AurorixClientException(error.Code, CopyAndFree(error.Message));
            }
            return new AurorixClient(
                new AurorixClientSafeHandle(nativeHandle),
                dispatcher ?? DispatcherQueue.GetForCurrentThread());
        }
        finally
        {
            pinnedPath.Free();
        }
    }

    public Task<AurorixCompletion> CommandAsync(
        ReadOnlyMemory<byte> request,
        CancellationToken cancellationToken = default) =>
        SendAsync(request, query: false, cancellationToken);

    public Task<AurorixCompletion> QueryAsync(
        ReadOnlyMemory<byte> request,
        CancellationToken cancellationToken = default) =>
        SendAsync(request, query: true, cancellationToken);

    public AurorixSubscription Subscribe(
        ReadOnlyMemory<byte> request,
        Action<AurorixEvent> onEvent)
    {
        ObjectDisposedException.ThrowIf(_disposed != 0, this);
        ArgumentNullException.ThrowIfNull(onEvent);
        var state = new SubscriptionState(_dispatcher, onEvent);
        var requestBytes = request.ToArray();
        var pinnedRequest = GCHandle.Alloc(requestBytes, GCHandleType.Pinned);
        state.Context = GCHandle.ToIntPtr(GCHandle.Alloc(state));
        try
        {
            var requestSlice = new AurorixNativeMethods.ByteSlice(
                requestBytes.Length == 0 ? IntPtr.Zero : pinnedRequest.AddrOfPinnedObject(),
                (ulong)requestBytes.Length);
            var result = WithClientHandle(client =>
            {
                var status = AurorixNativeMethods.aurorix_client_subscribe_v1(
                    client,
                    requestSlice,
                    state.Callback,
                    state.Context,
                    out var nativeSubscription,
                    out var observedSequence);
                return (status, nativeSubscription, observedSequence);
            });
            var status = result.status;
            var nativeSubscription = result.nativeSubscription;
            var observedSequence = result.observedSequence;
            if (status != AurorixNativeMethods.StatusOk || nativeSubscription == IntPtr.Zero)
            {
                state.FreeContext();
                throw new AurorixClientException(status, "subscription could not be created");
            }
            state.Handle = new AurorixSubscriptionSafeHandle(nativeSubscription);
            state.Activate(observedSequence);
            return new AurorixSubscription(state);
        }
        catch
        {
            state.FreeContext();
            throw;
        }
        finally
        {
            pinnedRequest.Free();
        }
    }

    private async Task<AurorixCompletion> SendAsync(
        ReadOnlyMemory<byte> request,
        bool query,
        CancellationToken cancellationToken)
    {
        ObjectDisposedException.ThrowIf(_disposed != 0, this);
        cancellationToken.ThrowIfCancellationRequested();
        var state = new OperationState(_dispatcher);
        var requestBytes = request.ToArray();
        var pinnedRequest = GCHandle.Alloc(requestBytes, GCHandleType.Pinned);
        var operationContext = GCHandle.Alloc(state);
        state.Context = GCHandle.ToIntPtr(operationContext);
        AurorixOperationSafeHandle? operationHandle = null;
        CancellationTokenRegistration cancellation = default;
        try
        {
            var requestSlice = new AurorixNativeMethods.ByteSlice(
                requestBytes.Length == 0 ? IntPtr.Zero : pinnedRequest.AddrOfPinnedObject(),
                (ulong)requestBytes.Length);
            var result = WithClientHandle(client =>
            {
                var status = query
                    ? AurorixNativeMethods.aurorix_client_query_v1(
                        client, requestSlice, state.Callback, state.Context, out var nativeOperation)
                    : AurorixNativeMethods.aurorix_client_command_v1(
                        client, requestSlice, state.Callback, state.Context, out nativeOperation);
                return (status, nativeOperation);
            });
            var status = result.status;
            var nativeOperation = result.nativeOperation;
            if (status != AurorixNativeMethods.StatusOk || nativeOperation == IntPtr.Zero)
            {
                state.FreeContext();
                throw new AurorixClientException(status, "operation could not be started");
            }
            operationHandle = new AurorixOperationSafeHandle(nativeOperation);
            state.Handle = operationHandle;
            cancellation = cancellationToken.Register(
                () =>
                {
                    var handle = operationHandle;
                    if (handle is not null && !handle.IsInvalid)
                    {
                        _ = AurorixNativeMethods.aurorix_operation_cancel_v1(handle.DangerousGetHandle());
                    }
                });
            return await state.Completion.Task.ConfigureAwait(false);
        }
        finally
        {
            cancellation.Dispose();
            operationHandle?.Dispose();
            state.FreeContext();
            pinnedRequest.Free();
        }
    }

    private static uint ToTimeoutMilliseconds(TimeSpan? timeout)
    {
        if (timeout is null || timeout.Value <= TimeSpan.Zero)
        {
            return 0;
        }
        var milliseconds = timeout.Value.TotalMilliseconds;
        return milliseconds >= uint.MaxValue ? uint.MaxValue : (uint)milliseconds;
    }

    private T WithClientHandle<T>(Func<IntPtr, T> operation)
    {
        var addedReference = false;
        try
        {
            _handle.DangerousAddRef(ref addedReference);
            ObjectDisposedException.ThrowIf(_disposed != 0, this);
            return operation(_handle.DangerousGetHandle());
        }
        finally
        {
            if (addedReference)
            {
                _handle.DangerousRelease();
            }
        }
    }

    internal static byte[] CopyAndFree(AurorixNativeMethods.Buffer buffer)
    {
        try
        {
            if (buffer.Pointer == IntPtr.Zero || buffer.Length == 0)
            {
                return Array.Empty<byte>();
            }
            if (buffer.Length > int.MaxValue)
            {
                throw new AurorixClientException(
                    AurorixNativeMethods.StatusInvalidArgument,
                    "native buffer exceeds the managed limit");
            }
            var result = new byte[(int)buffer.Length];
            Marshal.Copy(buffer.Pointer, result, 0, result.Length);
            return result;
        }
        finally
        {
            AurorixNativeMethods.aurorix_buffer_free_v1(buffer);
        }
    }

    internal static byte[] Copy(AurorixNativeMethods.ByteSlice slice)
    {
        if (slice.Pointer == IntPtr.Zero || slice.Length == 0)
        {
            return Array.Empty<byte>();
        }
        if (slice.Length > int.MaxValue)
        {
            throw new AurorixClientException(
                AurorixNativeMethods.StatusInvalidArgument,
                "native callback exceeds the managed limit");
        }
        var result = new byte[(int)slice.Length];
        Marshal.Copy(slice.Pointer, result, 0, result.Length);
        return result;
    }

    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 0)
        {
            _ = AurorixNativeMethods.aurorix_client_shutdown_v1(_handle.DangerousGetHandle());
            _handle.Dispose();
        }
        GC.SuppressFinalize(this);
    }

    private sealed class OperationState
    {
        internal readonly TaskCompletionSource<AurorixCompletion> Completion =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        internal readonly AurorixNativeMethods.CompletionCallback Callback;
        internal IntPtr Context;
        internal AurorixOperationSafeHandle? Handle;

        internal OperationState(DispatcherQueue? dispatcher)
        {
            Callback = (context, status, outcome, response) =>
            {
                try
                {
                    var state = (OperationState?)GCHandle.FromIntPtr(context).Target;
                    if (state is null)
                    {
                        return;
                    }
                    var completion = new AurorixCompletion(status, outcome, Copy(response));
                    state.Dispatch(dispatcher, () => state.Completion.TrySetResult(completion));
                }
                catch (Exception exception)
                {
                    var state = (OperationState?)GCHandle.FromIntPtr(context).Target;
                    state?.Completion.TrySetException(exception);
                }
            };
        }

        internal void Dispatch(DispatcherQueue? dispatcher, Action action)
        {
            if (dispatcher is null || !dispatcher.TryEnqueue(new DispatcherQueueHandler(action)))
            {
                action();
            }
        }

        internal void FreeContext()
        {
            if (Context != IntPtr.Zero)
            {
                GCHandle.FromIntPtr(Context).Free();
                Context = IntPtr.Zero;
            }
        }
    }

    internal sealed class SubscriptionState
    {
        [ThreadStatic]
        private static bool _inCallback;

        private readonly object _gate = new();
        private readonly List<(ulong Sequence, byte[] Payload)> _pending = new();
        internal readonly AurorixNativeMethods.EventSinkCallback Callback;
        internal readonly DispatcherQueue? Dispatcher;
        internal readonly Action<AurorixEvent> Sink;
        internal IntPtr Context;
        internal AurorixSubscriptionSafeHandle? Handle;
        internal ulong ObservedSequence;
        internal ulong LastSequence;
        internal bool Activated;
        internal bool IsCallbackThread => _inCallback;

        internal SubscriptionState(DispatcherQueue? dispatcher, Action<AurorixEvent> sink)
        {
            Dispatcher = dispatcher;
            Sink = sink;
            Callback = (context, sequence, @event) =>
            {
                _inCallback = true;
                try
                {
                    var state = (SubscriptionState?)GCHandle.FromIntPtr(context).Target;
                    if (state is null)
                    {
                        return;
                    }
                    state.Accept(sequence, Copy(@event));
                }
                catch
                {
                    // A sink exception must not unwind through the native ABI.
                }
                finally
                {
                    _inCallback = false;
                }
            };
        }

        internal void Activate(ulong observedSequence)
        {
            List<(ulong Sequence, byte[] Payload)> pending;
            lock (_gate)
            {
                ObservedSequence = observedSequence;
                Activated = true;
                pending = new List<(ulong Sequence, byte[] Payload)>(_pending);
                _pending.Clear();
            }

            pending.Sort(static (left, right) => left.Sequence.CompareTo(right.Sequence));
            foreach (var item in pending)
            {
                Accept(item.Sequence, item.Payload);
            }
        }

        private void Accept(ulong sequence, byte[] payload)
        {
            AurorixEvent? item = null;
            lock (_gate)
            {
                if (!Activated)
                {
                    _pending.Add((sequence, payload));
                    return;
                }
                if (sequence <= ObservedSequence)
                {
                    return;
                }
                var requiresResync = LastSequence != 0 && sequence != LastSequence + 1;
                LastSequence = sequence;
                item = new AurorixEvent(sequence, payload, requiresResync);
            }

            var delivered = item!;
            if (Dispatcher is null ||
                !Dispatcher.TryEnqueue(new DispatcherQueueHandler(() => Sink(delivered))))
            {
                Sink(delivered);
            }
        }

        internal void FreeContext()
        {
            if (Context != IntPtr.Zero)
            {
                GCHandle.FromIntPtr(Context).Free();
                Context = IntPtr.Zero;
            }
        }
    }
}

public sealed class AurorixSubscription : IDisposable
{
    private readonly AurorixClient.SubscriptionState _state;
    private int _disposed;

    internal AurorixSubscription(AurorixClient.SubscriptionState state)
    {
        _state = state;
    }

    public ulong ObservedSequence => _state.ObservedSequence;

    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }
        if (_state.IsCallbackThread)
        {
            if (_state.Dispatcher is { } dispatcher &&
                dispatcher.TryEnqueue(new DispatcherQueueHandler(DisposeCore)))
            {
                return;
            }
            _ = Task.Run(DisposeCore);
            return;
        }
        DisposeCore();
        GC.SuppressFinalize(this);
    }

    private void DisposeCore()
    {
        var handle = _state.Handle;
        if (handle is not null && !handle.IsInvalid)
        {
            var status = AurorixNativeMethods.aurorix_subscription_cancel_v1(handle.DangerousGetHandle());
            if (status == AurorixNativeMethods.StatusReentrantRelease)
            {
                _ = Task.Run(DisposeCore);
                return;
            }
            handle.Dispose();
        }
        _state.FreeContext();
    }
}

public sealed class AurorixClientException : Exception
{
    public AurorixClientException(int code, string message)
        : base(message)
    {
        Code = code;
    }

    public AurorixClientException(int code, byte[] message)
        : this(code, System.Text.Encoding.UTF8.GetString(message))
    {
    }

    public int Code { get; }
}
