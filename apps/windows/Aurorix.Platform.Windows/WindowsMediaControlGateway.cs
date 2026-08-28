namespace Aurorix.Platform.Windows;

/// <summary>
/// Maps the Core snapshot into the platform-neutral SMTC projection. A real
/// host translates the result into Windows Runtime types in its own adapter.
/// </summary>
public static class WindowsSmTcProjectionMapper
{
    public static WindowsSmTcProjection Map(WindowsPlaybackSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);

        var status = MapStatus(snapshot);
        var timeline = snapshot.CurrentItemId is null
            ? null
            : new WindowsMediaTimeline(
                snapshot.Clock.Position,
                snapshot.Metadata?.Duration,
                status == WindowsMediaPlaybackStatus.Playing ? snapshot.Clock.PlaybackRate : 0,
                snapshot.Clock.ClockEpoch,
                snapshot.Clock.IsDiscontinuous);

        return new WindowsSmTcProjection(
            snapshot.CurrentItemId,
            snapshot.Metadata,
            status,
            timeline,
            EnabledActions(snapshot, status),
            snapshot.StateVersion,
            snapshot.BufferGeneration,
            snapshot.Clock.ClockEpoch);
    }

    private static WindowsMediaPlaybackStatus MapStatus(WindowsPlaybackSnapshot snapshot)
    {
        if (snapshot.CurrentItemId is null)
        {
            return WindowsMediaPlaybackStatus.Closed;
        }

        return snapshot.State switch
        {
            WindowsPlaybackState.Loading or WindowsPlaybackState.Buffering =>
                WindowsMediaPlaybackStatus.Changing,
            WindowsPlaybackState.Playing => WindowsMediaPlaybackStatus.Playing,
            WindowsPlaybackState.Paused => WindowsMediaPlaybackStatus.Paused,
            WindowsPlaybackState.Empty => WindowsMediaPlaybackStatus.Closed,
            _ => WindowsMediaPlaybackStatus.Stopped,
        };
    }

    private static IEnumerable<WindowsMediaControlActionKind> EnabledActions(
        WindowsPlaybackSnapshot snapshot,
        WindowsMediaPlaybackStatus status)
    {
        if (snapshot.CurrentItemId is null)
        {
            return Array.Empty<WindowsMediaControlActionKind>();
        }

        var actions = new List<WindowsMediaControlActionKind>
        {
            WindowsMediaControlActionKind.Previous,
            WindowsMediaControlActionKind.Next,
        };

        if (status == WindowsMediaPlaybackStatus.Playing)
        {
            actions.Add(WindowsMediaControlActionKind.Pause);
        }
        else if (status is WindowsMediaPlaybackStatus.Changing or
            WindowsMediaPlaybackStatus.Paused or
            WindowsMediaPlaybackStatus.Stopped)
        {
            actions.Add(WindowsMediaControlActionKind.Play);
        }

        if (snapshot.State is not WindowsPlaybackState.Stopped and
            not WindowsPlaybackState.Ended and
            not WindowsPlaybackState.Failed and
            not WindowsPlaybackState.Unavailable)
        {
            actions.Add(WindowsMediaControlActionKind.Stop);
        }

        if (snapshot.Metadata?.Duration is not null)
        {
            actions.Add(WindowsMediaControlActionKind.Seek);
        }

        return actions;
    }
}

/// <summary>
/// Converts the small SMTC action vocabulary into the corresponding Core
/// command vocabulary. The Core-facing host owns the actual Rust/FFI command
/// construction and dispatch policy.
/// </summary>
public static class WindowsCorePlaybackCommandMapper
{
    public static WindowsCorePlaybackCommand Map(
        WindowsMediaControlAction action,
        WindowsPlaybackState? currentState = null)
    {
        ArgumentNullException.ThrowIfNull(action);

        return action.Kind switch
        {
            WindowsMediaControlActionKind.Play when currentState == WindowsPlaybackState.Paused =>
                new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Resume),
            WindowsMediaControlActionKind.Play => new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Play),
            WindowsMediaControlActionKind.Pause => new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Pause),
            WindowsMediaControlActionKind.Stop => new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Stop),
            WindowsMediaControlActionKind.Previous => new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Previous),
            WindowsMediaControlActionKind.Next => new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Next),
            WindowsMediaControlActionKind.Seek => new WindowsCorePlaybackCommand(
                action.RequestId,
                WindowsCorePlaybackCommandAction.Seek,
                WindowsPresentationClock.ToMicroseconds(action.SeekPosition!.Value, nameof(action.SeekPosition))),
            _ => throw new ArgumentOutOfRangeException(nameof(action.Kind)),
        };
    }
}

/// <summary>
/// Coalesces Core snapshots and sampled clocks into latest-value projections.
/// It has no queue, timer, worker, wall clock, or local playback position.
/// </summary>
public sealed class WindowsSmTcProjectionGateway
{
    private readonly object _gate = new();
    private readonly IWindowsSmTcProjectionSink _projectionSink;
    private readonly IWindowsCorePlaybackCommandSink _commandSink;
    private readonly IWindowsMediaControlLifecycleSink _lifecycleSink;
    private WindowsPlaybackSnapshot? _latestSnapshot;
    private WindowsSmTcProjection? _lastAppliedProjection;
    private ulong? _lastProgressSequence;
    private ulong? _lastLifecycleSequence;

    public WindowsSmTcProjectionGateway(
        IWindowsSmTcProjectionSink projectionSink,
        IWindowsCorePlaybackCommandSink commandSink,
        IWindowsMediaControlLifecycleSink lifecycleSink)
    {
        _projectionSink = projectionSink ?? throw new ArgumentNullException(nameof(projectionSink));
        _commandSink = commandSink ?? throw new ArgumentNullException(nameof(commandSink));
        _lifecycleSink = lifecycleSink ?? throw new ArgumentNullException(nameof(lifecycleSink));
    }

    public WindowsSmTcProjection? LatestProjection
    {
        get
        {
            lock (_gate)
            {
                return _latestSnapshot is null ? null : WindowsSmTcProjectionMapper.Map(_latestSnapshot);
            }
        }
    }

    public WindowsSmTcProjection? LastAppliedProjection
    {
        get
        {
            lock (_gate)
            {
                return _lastAppliedProjection;
            }
        }
    }

    /// <summary>
    /// Accepts a latest Core snapshot. State, metadata, capability, and
    /// discontinuity changes are applied immediately; position-only changes
    /// remain in the latest-value slot until FlushProgress is requested.
    /// </summary>
    public WindowsMediaControlUpdate AcceptSnapshot(WindowsPlaybackSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);

        lock (_gate)
        {
            if (_latestSnapshot is { } current && snapshot.StateVersion < current.StateVersion)
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.IgnoredStale);
            }

            if (_latestSnapshot is { } existing && snapshot.StateVersion == existing.StateVersion)
            {
                if (!string.Equals(existing.CurrentItemId, snapshot.CurrentItemId, StringComparison.Ordinal) ||
                    existing.State != snapshot.State ||
                    existing.BufferGeneration != snapshot.BufferGeneration ||
                    !Equals(existing.Metadata, snapshot.Metadata) ||
                    existing.PendingRequestId != snapshot.PendingRequestId)
                {
                    throw new ArgumentException(
                        "A snapshot with the same state version can only advance its Core clock.",
                        nameof(snapshot));
                }

                _latestSnapshot = snapshot.WithClock(
                    WindowsPresentationClock.IsAtLeast(snapshot.Clock, existing.Clock)
                        ? snapshot.Clock
                        : existing.Clock);
            }
            else
            {
                _latestSnapshot = snapshot;
            }

            var projection = WindowsSmTcProjectionMapper.Map(_latestSnapshot);
            if (ShouldApplyImmediately(projection))
            {
                return ApplyLocked(projection);
            }

            return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.Coalesced);
        }
    }

    /// <summary>
    /// Merges one externally sampled Core clock into the latest snapshot.
    /// Samples are ordered by the source sequence and by the Core clock epoch;
    /// no platform timestamp is consulted.
    /// </summary>
    public WindowsMediaControlUpdate SampleProgress(WindowsPlaybackProgressSample sample)
    {
        ArgumentNullException.ThrowIfNull(sample);

        lock (_gate)
        {
            if (_latestSnapshot is null)
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.AwaitingSnapshot);
            }

            if (_lastProgressSequence is { } lastSequence && sample.Sequence <= lastSequence)
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.IgnoredStale);
            }

            // Sequence is an observation watermark. Consume it before the
            // remaining fences so a rejected sample cannot be replayed.
            _lastProgressSequence = sample.Sequence;

            if (sample.StateVersion != _latestSnapshot.StateVersion ||
                sample.BufferGeneration != _latestSnapshot.BufferGeneration ||
                !string.Equals(sample.CurrentItemId, _latestSnapshot.CurrentItemId, StringComparison.Ordinal))
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.IgnoredStale);
            }

            if (!WindowsPresentationClock.IsAtLeast(sample.Clock, _latestSnapshot.Clock))
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.IgnoredStale);
            }

            _latestSnapshot = _latestSnapshot.WithClock(sample.Clock);

            var projection = WindowsSmTcProjectionMapper.Map(_latestSnapshot);
            return ShouldApplyImmediately(projection)
                ? ApplyLocked(projection)
                : new WindowsMediaControlUpdate(WindowsMediaControlActionResult.Coalesced);
        }
    }

    /// <summary>
    /// Applies the newest merged clock sample to the sink. Scheduling this
    /// call belongs to the host/audio integration; it is not a UI state timer.
    /// </summary>
    public WindowsMediaControlUpdate FlushProgress()
    {
        lock (_gate)
        {
            if (_latestSnapshot is null)
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.AwaitingSnapshot);
            }

            var projection = WindowsSmTcProjectionMapper.Map(_latestSnapshot);
            return _lastAppliedProjection is not null &&
                projection.HasSameFullState(_lastAppliedProjection)
                ? new WindowsMediaControlUpdate(WindowsMediaControlActionResult.Coalesced)
                : ApplyLocked(projection);
        }
    }

    /// <summary>
    /// Converts one native SMTC action into exactly one Core command and sends
    /// it directly to the Core-facing sink. The platform gateway never queues
    /// or executes the command itself.
    /// </summary>
    public async ValueTask<WindowsCorePlaybackCommand> HandleActionAsync(
        WindowsMediaControlAction action,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(action);
        cancellationToken.ThrowIfCancellationRequested();

        WindowsPlaybackState? currentState;
        lock (_gate)
        {
            currentState = _latestSnapshot?.State;
        }

        var command = WindowsCorePlaybackCommandMapper.Map(action, currentState);

        await _commandSink.DispatchAsync(command, cancellationToken).ConfigureAwait(false);
        return command;
    }

    /// <summary>
    /// Forwards ordered device/host lifecycle observations without changing
    /// Core playback state or clearing the latest projection.
    /// </summary>
    public WindowsMediaControlUpdate AcceptLifecycleEvent(WindowsMediaControlHostEvent @event)
    {
        ArgumentNullException.ThrowIfNull(@event);

        lock (_gate)
        {
            if (_lastLifecycleSequence is { } lastSequence && @event.Sequence <= lastSequence)
            {
                return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.IgnoredStale);
            }

            _lastLifecycleSequence = @event.Sequence;
            _lifecycleSink.Publish(@event);
            return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.Applied);
        }
    }

    private bool ShouldApplyImmediately(WindowsSmTcProjection projection) =>
        _lastAppliedProjection is null ||
        !_lastAppliedProjection.HasSameNonProgressState(projection) ||
        (_lastAppliedProjection.Timeline?.IsDiscontinuous != projection.Timeline?.IsDiscontinuous &&
            projection.Timeline?.IsDiscontinuous == true);

    private WindowsMediaControlUpdate ApplyLocked(WindowsSmTcProjection projection)
    {
        _projectionSink.Apply(projection);
        _lastAppliedProjection = projection;
        return new WindowsMediaControlUpdate(WindowsMediaControlActionResult.Applied, projection);
    }
}
