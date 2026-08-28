using System.Collections.ObjectModel;

namespace Aurorix.Platform.Windows;

/// <summary>
/// Deterministic sink for contract tests and host integration probes. It has
/// no timing behavior and records only values received at the adapter edge.
/// </summary>
public sealed class FakeWindowsMediaControlSink :
    IWindowsSmTcProjectionSink,
    IWindowsCorePlaybackCommandSink,
    IWindowsMediaControlLifecycleSink
{
    private readonly List<WindowsSmTcProjection> _projections = [];
    private readonly List<WindowsCorePlaybackCommand> _commands = [];
    private readonly List<WindowsMediaControlHostEvent> _lifecycleEvents = [];

    public IReadOnlyList<WindowsSmTcProjection> Projections =>
        new ReadOnlyCollection<WindowsSmTcProjection>(_projections);

    public IReadOnlyList<WindowsCorePlaybackCommand> Commands =>
        new ReadOnlyCollection<WindowsCorePlaybackCommand>(_commands);

    public IReadOnlyList<WindowsMediaControlHostEvent> LifecycleEvents =>
        new ReadOnlyCollection<WindowsMediaControlHostEvent>(_lifecycleEvents);

    public void Apply(WindowsSmTcProjection projection)
    {
        ArgumentNullException.ThrowIfNull(projection);
        _projections.Add(projection);
    }

    public ValueTask DispatchAsync(
        WindowsCorePlaybackCommand command,
        CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        _commands.Add(command);
        return ValueTask.CompletedTask;
    }

    public void Publish(WindowsMediaControlHostEvent @event)
    {
        ArgumentNullException.ThrowIfNull(@event);
        _lifecycleEvents.Add(@event);
    }
}
