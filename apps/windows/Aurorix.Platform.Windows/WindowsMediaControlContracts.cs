using System.Collections.ObjectModel;

namespace Aurorix.Platform.Windows;

/// <summary>
/// Platform-neutral names for the states exposed by Core's playback session.
/// This is a boundary DTO, not a second playback state machine.
/// </summary>
public enum WindowsPlaybackState
{
    Empty,
    Loading,
    Buffering,
    Playing,
    Paused,
    Stopped,
    Ended,
    Failed,
    Unavailable,
}

/// <summary>
/// Reasons that Core may mark a presentation timeline discontinuous.
/// </summary>
public enum WindowsClockDiscontinuityReason
{
    Seek,
    Pause,
    Resume,
    Stop,
    SourceTransition,
    OutputRestart,
    UnderrunRecovery,
    PlaybackRateChanged,
    SampleRateChanged,
}

/// <summary>
/// The public values needed to mirror Core's PresentationClock.
/// Position is always the Core media position; output latency is metadata and
/// is deliberately not added to it.
/// </summary>
public sealed record WindowsPresentationClock
{
    public WindowsPresentationClock(
        ulong clockEpoch,
        ulong renderedFrames,
        ulong mediaPositionUs,
        uint playbackRateMillionths,
        uint outputSampleRateHz,
        ulong estimatedOutputLatencyFrames,
        bool isDiscontinuous = false,
        WindowsClockDiscontinuityReason? discontinuityReason = null)
    {
        if (playbackRateMillionths == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(playbackRateMillionths));
        }

        if (!isDiscontinuous && discontinuityReason is not null)
        {
            throw new ArgumentException(
                "A discontinuity reason requires isDiscontinuous to be true.",
                nameof(discontinuityReason));
        }

        if (outputSampleRateHz == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(outputSampleRateHz));
        }

        EnsureTimeSpanRepresentable(mediaPositionUs, nameof(mediaPositionUs));

        ClockEpoch = clockEpoch;
        RenderedFrames = renderedFrames;
        MediaPositionUs = mediaPositionUs;
        PlaybackRateMillionths = playbackRateMillionths;
        OutputSampleRateHz = outputSampleRateHz;
        EstimatedOutputLatencyFrames = estimatedOutputLatencyFrames;
        IsDiscontinuous = isDiscontinuous;
        DiscontinuityReason = discontinuityReason;
    }

    public ulong ClockEpoch { get; }

    public ulong RenderedFrames { get; }

    public ulong MediaPositionUs { get; }

    public uint PlaybackRateMillionths { get; }

    public uint OutputSampleRateHz { get; }

    public ulong EstimatedOutputLatencyFrames { get; }

    public bool IsDiscontinuous { get; }

    public WindowsClockDiscontinuityReason? DiscontinuityReason { get; }

    public double PlaybackRate => PlaybackRateMillionths / 1_000_000d;

    public TimeSpan Position => FromMicroseconds(MediaPositionUs);

    internal static bool IsAtLeast(WindowsPresentationClock candidate, WindowsPresentationClock current)
    {
        if (candidate.ClockEpoch != current.ClockEpoch)
        {
            return candidate.ClockEpoch > current.ClockEpoch;
        }

        if (candidate.RenderedFrames != current.RenderedFrames)
        {
            return candidate.RenderedFrames > current.RenderedFrames;
        }

        return candidate.MediaPositionUs >= current.MediaPositionUs;
    }

    internal static TimeSpan FromMicroseconds(ulong microseconds)
    {
        EnsureTimeSpanRepresentable(microseconds, nameof(microseconds));
        return TimeSpan.FromTicks(checked((long)microseconds * 10));
    }

    internal static ulong ToMicroseconds(TimeSpan value, string parameterName)
    {
        if (value < TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(parameterName);
        }

        if (value.Ticks % 10 != 0)
        {
            throw new ArgumentException(
                "The value must have whole-microsecond precision.",
                parameterName);
        }

        return checked((ulong)(value.Ticks / 10));
    }

    private static void EnsureTimeSpanRepresentable(ulong microseconds, string parameterName)
    {
        var maximumMicroseconds = (ulong)(TimeSpan.MaxValue.Ticks / 10);
        if (microseconds > maximumMicroseconds)
        {
            throw new ArgumentOutOfRangeException(parameterName);
        }
    }
}

/// <summary>
/// Metadata supplied by the Core/library projection for the current item.
/// Paths, URLs, credentials, and provider handles are intentionally absent.
/// </summary>
public sealed record WindowsMediaMetadata
{
    public WindowsMediaMetadata(
        string title,
        string? artist = null,
        string? album = null,
        string? albumArtist = null,
        string? genre = null,
        uint? trackNumber = null,
        uint? trackCount = null,
        ulong? durationUs = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(title);

        if (trackNumber == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(trackNumber));
        }

        if (trackCount == 0)
        {
            throw new ArgumentOutOfRangeException(nameof(trackCount));
        }

        if (trackNumber is { } number && trackCount is { } count && number > count)
        {
            throw new ArgumentException(
                "trackNumber cannot be greater than trackCount.",
                nameof(trackNumber));
        }

        if (durationUs is { } duration)
        {
            WindowsPresentationClock.FromMicroseconds(duration);
        }

        Title = title.Trim();
        Artist = NormalizeOptional(artist);
        Album = NormalizeOptional(album);
        AlbumArtist = NormalizeOptional(albumArtist);
        Genre = NormalizeOptional(genre);
        TrackNumber = trackNumber;
        TrackCount = trackCount;
        DurationUs = durationUs;
    }

    public string Title { get; }

    public string? Artist { get; }

    public string? Album { get; }

    public string? AlbumArtist { get; }

    public string? Genre { get; }

    public uint? TrackNumber { get; }

    public uint? TrackCount { get; }

    public ulong? DurationUs { get; }

    public TimeSpan? Duration => DurationUs is { } value
        ? WindowsPresentationClock.FromMicroseconds(value)
        : null;

    private static string? NormalizeOptional(string? value) =>
        string.IsNullOrWhiteSpace(value) ? null : value.Trim();
}

/// <summary>
/// Snapshot mirror consumed by this platform boundary. The Core host/FFI
/// adapter supplies the values; this project does not own playback state.
/// </summary>
public sealed record WindowsPlaybackSnapshot
{
    public WindowsPlaybackSnapshot(
        WindowsPlaybackState state,
        string? currentItemId,
        WindowsPresentationClock clock,
        ulong stateVersion,
        ulong bufferGeneration,
        ulong? pendingRequestId = null,
        WindowsMediaMetadata? metadata = null)
    {
        ArgumentNullException.ThrowIfNull(clock);

        CurrentItemId = NormalizeItemId(currentItemId);
        if (CurrentItemId is null && metadata is not null)
        {
            throw new ArgumentException(
                "Metadata cannot be present without a current item.",
                nameof(metadata));
        }

        if (metadata?.DurationUs is { } duration && duration < clock.MediaPositionUs)
        {
            throw new ArgumentException(
                "Metadata duration cannot be before the Core clock position.",
                nameof(metadata));
        }

        State = state;
        Clock = clock;
        StateVersion = stateVersion;
        BufferGeneration = bufferGeneration;
        PendingRequestId = pendingRequestId;
        Metadata = metadata;
    }

    public WindowsPlaybackState State { get; }

    public string? CurrentItemId { get; }

    public WindowsPresentationClock Clock { get; }

    public ulong StateVersion { get; }

    public ulong BufferGeneration { get; }

    public ulong? PendingRequestId { get; }

    public WindowsMediaMetadata? Metadata { get; }

    internal WindowsPlaybackSnapshot WithClock(WindowsPresentationClock clock) =>
        new(
            State,
            CurrentItemId,
            clock,
            StateVersion,
            BufferGeneration,
            PendingRequestId,
            Metadata);

    private static string? NormalizeItemId(string? itemId)
    {
        if (string.IsNullOrWhiteSpace(itemId))
        {
            return null;
        }

        return itemId.Trim();
    }
}

/// <summary>
/// A sampled Core clock. Sequence is assigned by the sampling source and is
/// used only to reject out-of-order observations.
/// </summary>
public sealed record WindowsPlaybackProgressSample
{
    public WindowsPlaybackProgressSample(
        ulong sequence,
        ulong stateVersion,
        ulong bufferGeneration,
        string? currentItemId,
        WindowsPresentationClock clock)
    {
        ArgumentNullException.ThrowIfNull(clock);

        Sequence = sequence;
        StateVersion = stateVersion;
        BufferGeneration = bufferGeneration;
        CurrentItemId = string.IsNullOrWhiteSpace(currentItemId) ? null : currentItemId.Trim();
        Clock = clock;
    }

    public ulong Sequence { get; }

    public ulong StateVersion { get; }

    public ulong BufferGeneration { get; }

    public string? CurrentItemId { get; }

    public WindowsPresentationClock Clock { get; }
}

/// <summary>
/// The timeline shape a real SMTC host can translate to native timeline
/// properties without consulting a second clock.
/// </summary>
public sealed record WindowsMediaTimeline
{
    public WindowsMediaTimeline(
        TimeSpan position,
        TimeSpan? duration,
        double playbackRate,
        ulong clockEpoch,
        bool isDiscontinuous)
    {
        if (position < TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(position));
        }

        if (duration is { } value && (value < TimeSpan.Zero || position > value))
        {
            throw new ArgumentOutOfRangeException(nameof(duration));
        }

        if (playbackRate < 0 || double.IsNaN(playbackRate) || double.IsInfinity(playbackRate))
        {
            throw new ArgumentOutOfRangeException(nameof(playbackRate));
        }

        Position = position;
        Duration = duration;
        PlaybackRate = playbackRate;
        ClockEpoch = clockEpoch;
        IsDiscontinuous = isDiscontinuous;
    }

    public TimeSpan Position { get; }

    public TimeSpan? Duration { get; }

    public TimeSpan MinSeekTime => TimeSpan.Zero;

    public TimeSpan? MaxSeekTime => Duration;

    public double PlaybackRate { get; }

    public ulong ClockEpoch { get; }

    public bool IsDiscontinuous { get; }
}

public enum WindowsMediaPlaybackStatus
{
    Closed,
    Changing,
    Playing,
    Paused,
    Stopped,
}

public enum WindowsMediaControlActionKind
{
    Play,
    Pause,
    Stop,
    Previous,
    Next,
    Seek,
}

/// <summary>
/// An action received from a native SMTC callback. RequestId is supplied by
/// the Core-facing host so this adapter never invents a second command ledger.
/// </summary>
public sealed record WindowsMediaControlAction
{
    public WindowsMediaControlAction(
        ulong requestId,
        WindowsMediaControlActionKind kind,
        TimeSpan? seekPosition = null)
    {
        if (kind == WindowsMediaControlActionKind.Seek)
        {
            if (seekPosition is null || seekPosition < TimeSpan.Zero)
            {
                throw new ArgumentOutOfRangeException(nameof(seekPosition));
            }
        }
        else if (seekPosition is not null)
        {
            throw new ArgumentException(
                "A seek position is only valid for a seek action.",
                nameof(seekPosition));
        }

        RequestId = requestId;
        Kind = kind;
        SeekPosition = seekPosition;
    }

    public ulong RequestId { get; }

    public WindowsMediaControlActionKind Kind { get; }

    public TimeSpan? SeekPosition { get; }
}

public enum WindowsCorePlaybackCommandAction
{
    Play,
    Resume,
    Pause,
    Stop,
    Previous,
    Next,
    Seek,
}

/// <summary>
/// Typed command output for the Core host. A native SMTC action is never
/// applied locally; it is represented once and handed to this sink.
/// </summary>
public sealed record WindowsCorePlaybackCommand
{
    public WindowsCorePlaybackCommand(
        ulong requestId,
        WindowsCorePlaybackCommandAction action,
        ulong? positionUs = null)
    {
        if (action == WindowsCorePlaybackCommandAction.Seek)
        {
            if (positionUs is null)
            {
                throw new ArgumentNullException(nameof(positionUs));
            }

            WindowsPresentationClock.FromMicroseconds(positionUs.Value);
        }
        else if (positionUs is not null)
        {
            throw new ArgumentException(
                "A position is only valid for a seek command.",
                nameof(positionUs));
        }

        RequestId = requestId;
        Action = action;
        PositionUs = positionUs;
    }

    public ulong RequestId { get; }

    public WindowsCorePlaybackCommandAction Action { get; }

    public ulong? PositionUs { get; }
}

public enum WindowsMediaControlActionResult
{
    Dispatched,
    IgnoredStale,
    AwaitingSnapshot,
    Coalesced,
    Applied,
}

public readonly record struct WindowsMediaControlUpdate(
    WindowsMediaControlActionResult Result,
    WindowsSmTcProjection? Projection = null);

public enum WindowsMediaControlHostEventKind
{
    HostStarted,
    HostStopping,
    HostStopped,
    DeviceConnected,
    DeviceDisconnected,
    DeviceChanged,
    ControlsAvailable,
    ControlsUnavailable,
}

/// <summary>
/// Ordered host/device observation from the real native adapter. Sequence and
/// observed time belong to that adapter; this project does not create them.
/// </summary>
public sealed record WindowsMediaControlHostEvent
{
    public WindowsMediaControlHostEvent(
        ulong sequence,
        WindowsMediaControlHostEventKind kind,
        string? deviceId = null,
        DateTimeOffset? observedAt = null)
    {
        if (kind is
            WindowsMediaControlHostEventKind.DeviceConnected or
            WindowsMediaControlHostEventKind.DeviceDisconnected or
            WindowsMediaControlHostEventKind.DeviceChanged)
        {
            ArgumentException.ThrowIfNullOrWhiteSpace(deviceId);
        }

        Sequence = sequence;
        Kind = kind;
        DeviceId = string.IsNullOrWhiteSpace(deviceId) ? null : deviceId.Trim();
        ObservedAt = observedAt;
    }

    public ulong Sequence { get; }

    public WindowsMediaControlHostEventKind Kind { get; }

    public string? DeviceId { get; }

    public DateTimeOffset? ObservedAt { get; }
}

/// <summary>
/// The projection written to SystemMediaTransportControls by a real host.
/// </summary>
public sealed record WindowsSmTcProjection
{
    public WindowsSmTcProjection(
        string? currentItemId,
        WindowsMediaMetadata? metadata,
        WindowsMediaPlaybackStatus playbackStatus,
        WindowsMediaTimeline? timeline,
        IEnumerable<WindowsMediaControlActionKind> enabledActions,
        ulong stateVersion,
        ulong bufferGeneration,
        ulong clockEpoch)
    {
        ArgumentNullException.ThrowIfNull(enabledActions);

        CurrentItemId = string.IsNullOrWhiteSpace(currentItemId) ? null : currentItemId.Trim();
        Metadata = metadata;
        PlaybackStatus = playbackStatus;
        Timeline = timeline;
        EnabledActions = new ReadOnlyCollection<WindowsMediaControlActionKind>(
            enabledActions.Distinct().OrderBy(static action => action).ToArray());
        StateVersion = stateVersion;
        BufferGeneration = bufferGeneration;
        ClockEpoch = clockEpoch;
    }

    public string? CurrentItemId { get; }

    public WindowsMediaMetadata? Metadata { get; }

    public WindowsMediaPlaybackStatus PlaybackStatus { get; }

    public WindowsMediaTimeline? Timeline { get; }

    public IReadOnlyList<WindowsMediaControlActionKind> EnabledActions { get; }

    public ulong StateVersion { get; }

    public ulong BufferGeneration { get; }

    public ulong ClockEpoch { get; }

    internal bool HasSameNonProgressState(WindowsSmTcProjection other) =>
        string.Equals(CurrentItemId, other.CurrentItemId, StringComparison.Ordinal)
        && Equals(Metadata, other.Metadata)
        && PlaybackStatus == other.PlaybackStatus
        && StateVersion == other.StateVersion
        && BufferGeneration == other.BufferGeneration
        && ClockEpoch == other.ClockEpoch
        && EnabledActions.SequenceEqual(other.EnabledActions)
        && Timeline?.Duration == other.Timeline?.Duration
        && Timeline?.PlaybackRate == other.Timeline?.PlaybackRate;

    internal bool HasSameFullState(WindowsSmTcProjection other) =>
        HasSameNonProgressState(other)
        && Timeline?.Position == other.Timeline?.Position
        && Timeline?.IsDiscontinuous == other.Timeline?.IsDiscontinuous;
}

public interface IWindowsSmTcProjectionSink
{
    void Apply(WindowsSmTcProjection projection);
}

public interface IWindowsCorePlaybackCommandSink
{
    ValueTask DispatchAsync(
        WindowsCorePlaybackCommand command,
        CancellationToken cancellationToken = default);
}

public interface IWindowsMediaControlLifecycleSink
{
    void Publish(WindowsMediaControlHostEvent @event);
}
