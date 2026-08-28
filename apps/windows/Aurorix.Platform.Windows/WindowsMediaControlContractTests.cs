#if WINDOWS_PLATFORM_CONTRACT_TESTS

using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;

namespace Aurorix.Platform.Windows;

/// <summary>
/// Dependency-free contract probes. They are compiled only by the temporary
/// test runner used for this project because no test project exists in the
/// current Windows solution.
/// </summary>
public static class WindowsMediaControlContractTests
{
    public static async Task RunAllAsync()
    {
        MapsCoreSnapshotToSmTcProjection();
        CoalescesAndFlushesCoreProgress();
        RejectsStaleProgressAndBufferSamples();
        await MapsSystemActionsToCoreCommands();
        PreservesOrderedLifecycleEvents();
        await Task.CompletedTask;
    }

    private static void MapsCoreSnapshotToSmTcProjection()
    {
        var snapshot = Snapshot(
            WindowsPlaybackState.Playing,
            stateVersion: 4,
            bufferGeneration: 2,
            positionUs: 1_500_000,
            metadata: new WindowsMediaMetadata(
                "  Song  ",
                artist: " Artist ",
                album: "Album",
                durationUs: 240_000_000));

        var projection = WindowsSmTcProjectionMapper.Map(snapshot);

        Equal(WindowsMediaPlaybackStatus.Playing, projection.PlaybackStatus);
        Equal("Song", projection.Metadata!.Title);
        Equal(TimeSpan.FromSeconds(1.5), projection.Timeline!.Position);
        Equal(TimeSpan.FromMinutes(4), projection.Timeline.Duration);
        Equal(1d, projection.Timeline.PlaybackRate);
        True(projection.EnabledActions.Contains(WindowsMediaControlActionKind.Pause));
        True(!projection.EnabledActions.Contains(WindowsMediaControlActionKind.Play));
        True(projection.EnabledActions.Contains(WindowsMediaControlActionKind.Seek));
    }

    private static void CoalescesAndFlushesCoreProgress()
    {
        var sink = new FakeWindowsMediaControlSink();
        var gateway = new WindowsSmTcProjectionGateway(sink, sink, sink);
        var initial = Snapshot(WindowsPlaybackState.Playing, 1, 1, 0);

        Equal(WindowsMediaControlActionResult.Applied, gateway.AcceptSnapshot(initial).Result);
        Equal(1, sink.Projections.Count);

        Equal(
            WindowsMediaControlActionResult.Coalesced,
            gateway.SampleProgress(Sample(1, 1, 1, 1_000_000)).Result);
        Equal(
            WindowsMediaControlActionResult.Coalesced,
            gateway.SampleProgress(Sample(2, 1, 1, 2_000_000)).Result);
        Equal(1, sink.Projections.Count);

        Equal(WindowsMediaControlActionResult.Applied, gateway.FlushProgress().Result);
        Equal(2, sink.Projections.Count);
        Equal(TimeSpan.FromSeconds(2), sink.Projections[^1].Timeline!.Position);

        Equal(
            WindowsMediaControlActionResult.Coalesced,
            gateway.FlushProgress().Result);
        Equal(2, sink.Projections.Count);
    }

    private static void RejectsStaleProgressAndBufferSamples()
    {
        var sink = new FakeWindowsMediaControlSink();
        var gateway = new WindowsSmTcProjectionGateway(sink, sink, sink);
        gateway.AcceptSnapshot(Snapshot(WindowsPlaybackState.Playing, 3, 7, 1_000_000));

        Equal(
            WindowsMediaControlActionResult.IgnoredStale,
            gateway.SampleProgress(Sample(1, 3, 7, 500_000)).Result);
        Equal(
            WindowsMediaControlActionResult.IgnoredStale,
            gateway.SampleProgress(Sample(1, 3, 7, 1_500_000)).Result);
        Equal(
            WindowsMediaControlActionResult.IgnoredStale,
            gateway.SampleProgress(Sample(2, 3, 6, 1_500_000)).Result);
        Equal(TimeSpan.FromSeconds(1), gateway.LatestProjection!.Timeline!.Position);
    }

    private static async Task MapsSystemActionsToCoreCommands()
    {
        var sink = new FakeWindowsMediaControlSink();
        var gateway = new WindowsSmTcProjectionGateway(sink, sink, sink);
        gateway.AcceptSnapshot(Snapshot(WindowsPlaybackState.Paused, 8, 2, 10_000_000));

        await gateway.HandleActionAsync(new WindowsMediaControlAction(1, WindowsMediaControlActionKind.Play));
        await gateway.HandleActionAsync(new WindowsMediaControlAction(2, WindowsMediaControlActionKind.Pause));
        await gateway.HandleActionAsync(new WindowsMediaControlAction(3, WindowsMediaControlActionKind.Stop));
        await gateway.HandleActionAsync(new WindowsMediaControlAction(4, WindowsMediaControlActionKind.Previous));
        await gateway.HandleActionAsync(new WindowsMediaControlAction(5, WindowsMediaControlActionKind.Next));
        await gateway.HandleActionAsync(new WindowsMediaControlAction(
            6,
            WindowsMediaControlActionKind.Seek,
            TimeSpan.FromSeconds(12.5)));

        Equal(6, sink.Commands.Count);
        Equal(WindowsCorePlaybackCommandAction.Resume, sink.Commands[0].Action);
        Equal(WindowsCorePlaybackCommandAction.Seek, sink.Commands[^1].Action);
        Equal((ulong)12_500_000, sink.Commands[^1].PositionUs);
    }

    private static void PreservesOrderedLifecycleEvents()
    {
        var sink = new FakeWindowsMediaControlSink();
        var gateway = new WindowsSmTcProjectionGateway(sink, sink, sink);

        Equal(
            WindowsMediaControlActionResult.Applied,
            gateway.AcceptLifecycleEvent(new WindowsMediaControlHostEvent(
                1,
                WindowsMediaControlHostEventKind.HostStarted)).Result);
        Equal(
            WindowsMediaControlActionResult.IgnoredStale,
            gateway.AcceptLifecycleEvent(new WindowsMediaControlHostEvent(
                1,
                WindowsMediaControlHostEventKind.HostStopped)).Result);
        Equal(
            WindowsMediaControlActionResult.Applied,
            gateway.AcceptLifecycleEvent(new WindowsMediaControlHostEvent(
                2,
                WindowsMediaControlHostEventKind.DeviceConnected,
                "default-device")).Result);

        Equal(2, sink.LifecycleEvents.Count);
        Equal(WindowsMediaControlHostEventKind.DeviceConnected, sink.LifecycleEvents[^1].Kind);
    }

    private static WindowsPlaybackSnapshot Snapshot(
        WindowsPlaybackState state,
        ulong stateVersion,
        ulong bufferGeneration,
        ulong positionUs,
        WindowsMediaMetadata? metadata = null) =>
        new(
            state,
            "item-1",
            new WindowsPresentationClock(
                clockEpoch: stateVersion,
                renderedFrames: positionUs,
                mediaPositionUs: positionUs,
                playbackRateMillionths: 1_000_000,
                outputSampleRateHz: 48_000,
                estimatedOutputLatencyFrames: 0),
            stateVersion,
            bufferGeneration,
            metadata: metadata);

    private static WindowsPlaybackProgressSample Sample(
        ulong sequence,
        ulong stateVersion,
        ulong bufferGeneration,
        ulong positionUs) =>
        new(
            sequence,
            stateVersion,
            bufferGeneration,
            "item-1",
            new WindowsPresentationClock(
                clockEpoch: stateVersion,
                renderedFrames: positionUs,
                mediaPositionUs: positionUs,
                playbackRateMillionths: 1_000_000,
                outputSampleRateHz: 48_000,
                estimatedOutputLatencyFrames: 0));

    private static void True(bool condition)
    {
        if (!condition)
        {
            throw new InvalidOperationException("Contract assertion failed.");
        }
    }

    private static void Equal<T>(T expected, T actual)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new InvalidOperationException(
                $"Contract assertion failed. Expected '{expected}', actual '{actual}'.");
        }
    }
}

#endif
