using System.Collections.Concurrent;
using System.Collections.ObjectModel;

namespace Aurorix.Platform.Windows;

public interface IWindowsLibraryGateway
{
    ValueTask<WindowsLocatorProbeResult> ProbeAsync(
        WindowsMediaLocator locator,
        CancellationToken cancellationToken = default);

    WindowsScanSession StartScan(
        WindowsScanRequest request,
        IProgress<WindowsScanEvent>? progress = null,
        CancellationToken cancellationToken = default);

    bool TryCancelScan(Guid scanId);

    ValueTask<WindowsRelinkResult> FindRelinkCandidatesAsync(
        WindowsRelinkRequest request,
        CancellationToken cancellationToken = default);
}

public sealed class WindowsLibraryGateway : IWindowsLibraryGateway, IDisposable
{
    private readonly IWindowsFileSystem _fileSystem;
    private readonly IWindowsFileIdentityProvider _identityProvider;
    private readonly IWindowsQuickHashProvider _quickHashProvider;
    private readonly ConcurrentDictionary<Guid, WindowsScanSession> _scans = new();

    public WindowsLibraryGateway(
        IWindowsFileSystem? fileSystem = null,
        IWindowsFileIdentityProvider? identityProvider = null,
        IWindowsQuickHashProvider? quickHashProvider = null)
    {
        _fileSystem = fileSystem ?? new SystemWindowsFileSystem();
        _identityProvider = identityProvider ?? new WindowsFileIdentityProvider();
        _quickHashProvider = quickHashProvider ?? new WindowsQuickHashProvider();
    }

    public ValueTask<WindowsLocatorProbeResult> ProbeAsync(
        WindowsMediaLocator locator,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(locator);
        cancellationToken.ThrowIfCancellationRequested();

        try
        {
            var info = _fileSystem.GetInfo(locator.NormalizedPath);
            if (info.Kind != locator.Kind)
            {
                return ValueTask.FromResult(new WindowsLocatorProbeResult(
                    locator,
                    WindowsResourceState.Unsupported,
                    null,
                    WindowsGatewayErrorCode.TypeMismatch));
            }

            if (info.IsReparsePoint)
            {
                return ValueTask.FromResult(new WindowsLocatorProbeResult(
                    locator,
                    WindowsResourceState.Unsupported,
                    null,
                    WindowsGatewayErrorCode.ReparsePointSkipped));
            }

            var identity = _identityProvider.TryGetIdentity(locator.NormalizedPath, locator.Kind);
            var metadata = new WindowsFileMetadata(
                info.SizeBytes,
                info.LastWriteTimeUtc,
                identity,
                info.IsReparsePoint);
            var resolvedLocator = identity is { } value ? locator.WithIdentity(value) : locator;
            return ValueTask.FromResult(new WindowsLocatorProbeResult(
                resolvedLocator,
                WindowsResourceState.Available,
                metadata));
        }
        catch (Exception exception) when (IsExpectedFileSystemException(exception))
        {
            var (state, errorCode) = Classify(exception);
            return ValueTask.FromResult(new WindowsLocatorProbeResult(locator, state, null, errorCode));
        }
    }

    public WindowsScanSession StartScan(
        WindowsScanRequest request,
        IProgress<WindowsScanEvent>? progress = null,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(request);

        var scanId = Guid.NewGuid();
        var linkedCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        var session = new WindowsScanSession(scanId, linkedCancellation);
        if (!_scans.TryAdd(scanId, session))
        {
            linkedCancellation.Dispose();
            throw new InvalidOperationException("Could not allocate a unique scan session.");
        }

        var task = Task.Run(
            () => WindowsScanEngine.Scan(
                scanId,
                request,
                _fileSystem,
                _identityProvider,
                progress,
                linkedCancellation.Token),
            CancellationToken.None);
        session.Attach(task);
        _ = task.ContinueWith(
            _ =>
            {
                _scans.TryRemove(scanId, out var removedSession);
                linkedCancellation.Dispose();
            },
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);

        return session;
    }

    public bool TryCancelScan(Guid scanId) =>
        _scans.TryGetValue(scanId, out var session) && session.Cancel();

    public async ValueTask<WindowsRelinkResult> FindRelinkCandidatesAsync(
        WindowsRelinkRequest request,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(request);

        var entries = new List<WindowsScanEntry>();
        var issues = new List<WindowsScanIssue>();
        var scanId = Guid.NewGuid();
        var result = await Task.Run(
            () => WindowsScanEngine.Scan(
                scanId,
                new WindowsScanRequest(request.SearchRoots),
                _fileSystem,
                _identityProvider,
                new DelegateProgress<WindowsScanEvent>(scanEvent =>
                {
                    if (scanEvent.Entry is { } entry)
                    {
                        entries.Add(entry);
                    }

                    if (scanEvent.Issue is { } issue)
                    {
                        issues.Add(issue);
                    }
                }),
                cancellationToken),
            CancellationToken.None).ConfigureAwait(false);

        if (result.State == WindowsScanCompletionState.Cancelled)
        {
            return new WindowsRelinkResult(
                WindowsRelinkOutcome.Cancelled,
                Array.Empty<WindowsRelinkCandidate>(),
                new ReadOnlyCollection<WindowsScanIssue>(issues));
        }

        var observations = new List<WindowsRelinkObservation>();
        foreach (var entry in entries)
        {
            cancellationToken.ThrowIfCancellationRequested();
            WindowsRelinkFingerprint? quickHash = null;
            if (request.ExpectedQuickHash is not null &&
                entry.Metadata.SizeBytes == request.ExpectedSizeBytes)
            {
                quickHash = _quickHashProvider.TryCompute(entry.Locator);
            }

            observations.Add(new WindowsRelinkObservation(entry.Locator, entry.Metadata, quickHash));
        }

        var candidates = WindowsRelinkMatcher.Match(request, observations);
        var outcome = candidates.Count switch
        {
            1 => WindowsRelinkOutcome.Found,
            > 1 => WindowsRelinkOutcome.Ambiguous,
            _ when issues.Any(static issue => issue.Kind == WindowsScanIssueKind.PermissionDenied) =>
                WindowsRelinkOutcome.PermissionDenied,
            _ => WindowsRelinkOutcome.NotFound,
        };

        return new WindowsRelinkResult(
            outcome,
            new ReadOnlyCollection<WindowsRelinkCandidate>(candidates.ToArray()),
            new ReadOnlyCollection<WindowsScanIssue>(issues));
    }

    public void Dispose()
    {
        foreach (var session in _scans.Values)
        {
            session.Cancel();
        }

        _scans.Clear();
    }

    private static bool IsExpectedFileSystemException(Exception exception) =>
        exception is UnauthorizedAccessException
            or FileNotFoundException
            or DirectoryNotFoundException
            or IOException
            or ArgumentException
            or NotSupportedException
            or PathTooLongException;

    internal static (WindowsResourceState State, WindowsGatewayErrorCode ErrorCode) Classify(Exception exception) =>
        exception switch
        {
            UnauthorizedAccessException => (WindowsResourceState.PermissionDenied, WindowsGatewayErrorCode.AccessDenied),
            FileNotFoundException or DirectoryNotFoundException =>
                (WindowsResourceState.Missing, WindowsGatewayErrorCode.NotFound),
            ArgumentException or NotSupportedException or PathTooLongException =>
                (WindowsResourceState.Unsupported, WindowsGatewayErrorCode.InvalidPath),
            _ => (WindowsResourceState.Error, WindowsGatewayErrorCode.IoError),
        };

    private sealed class DelegateProgress<T>(Action<T> callback) : IProgress<T>
    {
        public void Report(T value) => callback(value);
    }
}

public sealed class WindowsScanSession
{
    private readonly CancellationTokenSource _cancellation;
    private Task<WindowsScanResult>? _completion;

    internal WindowsScanSession(Guid scanId, CancellationTokenSource cancellation)
    {
        ScanId = scanId;
        _cancellation = cancellation;
    }

    public Guid ScanId { get; }

    public Task<WindowsScanResult> Completion =>
        Volatile.Read(ref _completion) ?? throw new InvalidOperationException("The scan session is not started.");

    public bool IsCancellationRequested => _cancellation.IsCancellationRequested;

    public bool Cancel()
    {
        if (_cancellation.IsCancellationRequested)
        {
            return false;
        }

        try
        {
            _cancellation.Cancel();
            return true;
        }
        catch (ObjectDisposedException)
        {
            return false;
        }
    }

    internal void Attach(Task<WindowsScanResult> completion) =>
        Interlocked.CompareExchange(ref _completion, completion, null);
}

internal static class WindowsScanEngine
{
    internal static WindowsScanResult Scan(
        Guid scanId,
        WindowsScanRequest request,
        IWindowsFileSystem fileSystem,
        IWindowsFileIdentityProvider identityProvider,
        IProgress<WindowsScanEvent>? progress,
        CancellationToken cancellationToken)
    {
        var issues = new List<WindowsScanIssue>();
        long discoveredCount = 0;
        long missingCount = 0;
        long permissionDeniedCount = 0;
        long unsupportedCount = 0;
        long errorCount = 0;
        long sequence = 0;

        void Emit(WindowsScanEventKind kind, WindowsScanEntry? entry = null, WindowsScanIssue? issue = null, WindowsScanResult? result = null)
        {
            sequence++;
            try
            {
                progress?.Report(new WindowsScanEvent(scanId, sequence, kind, entry, issue, result));
            }
            catch
            {
                // A presentation callback must never fail or stop a scan worker.
            }
        }

        void AddIssue(WindowsScanIssue issue)
        {
            issues.Add(issue);
            switch (issue.Kind)
            {
                case WindowsScanIssueKind.Missing:
                    missingCount++;
                    break;
                case WindowsScanIssueKind.PermissionDenied:
                    permissionDeniedCount++;
                    break;
                case WindowsScanIssueKind.Unsupported:
                    unsupportedCount++;
                    break;
                case WindowsScanIssueKind.Error:
                    errorCount++;
                    break;
            }

            Emit(WindowsScanEventKind.Issue, issue: issue);
        }

        try
        {
            var pending = new Stack<(string Path, WindowsLocatorKind? ExpectedKind)>();
            foreach (var root in request.Roots.Reverse())
            {
                pending.Push((root.NormalizedPath, root.Kind));
            }

            var visited = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            while (pending.Count > 0)
            {
                cancellationToken.ThrowIfCancellationRequested();
                var (path, expectedKind) = pending.Pop();
                if (!visited.Add(path))
                {
                    continue;
                }

                WindowsFileSystemEntryInfo info;
                try
                {
                    info = fileSystem.GetInfo(path);
                }
                catch (Exception exception) when (IsExpectedFileSystemException(exception))
                {
                    var (state, errorCode) = WindowsLibraryGateway.Classify(exception);
                    AddIssue(new WindowsScanIssue(
                        expectedKind is { } kind
                            ? WindowsMediaLocator.FromNormalizedPath(path, kind)
                            : null,
                        ToIssueKind(state),
                        errorCode));
                    continue;
                }

                if (expectedKind is { } selectedKind && info.Kind != selectedKind)
                {
                    AddIssue(new WindowsScanIssue(
                        WindowsMediaLocator.FromNormalizedPath(path, selectedKind),
                        WindowsScanIssueKind.Unsupported,
                        WindowsGatewayErrorCode.TypeMismatch));
                    continue;
                }

                if (info.IsReparsePoint)
                {
                    AddIssue(new WindowsScanIssue(
                        WindowsMediaLocator.FromNormalizedPath(path, expectedKind ?? info.Kind),
                        WindowsScanIssueKind.Unsupported,
                        WindowsGatewayErrorCode.ReparsePointSkipped));
                    continue;
                }

                var locator = WindowsMediaLocator.FromNormalizedPath(path, info.Kind);
                var identity = identityProvider.TryGetIdentity(path, info.Kind);
                var metadata = new WindowsFileMetadata(
                    info.SizeBytes,
                    info.LastWriteTimeUtc,
                    identity,
                    info.IsReparsePoint);
                locator = identity is { } value ? locator.WithIdentity(value) : locator;

                if (info.Kind == WindowsLocatorKind.File)
                {
                    var entry = new WindowsScanEntry(locator, metadata);
                    discoveredCount++;
                    Emit(WindowsScanEventKind.Discovered, entry: entry);
                    continue;
                }

                string[] children;
                try
                {
                    children = fileSystem.EnumerateChildren(path)
                        .Select(WindowsPathNormalizer.NormalizePath)
                        .Distinct(StringComparer.OrdinalIgnoreCase)
                        .OrderByDescending(static child => child, StringComparer.OrdinalIgnoreCase)
                        .ToArray();
                }
                catch (Exception exception) when (IsExpectedFileSystemException(exception))
                {
                    var (state, errorCode) = WindowsLibraryGateway.Classify(exception);
                    var folderLocator = locator;
                    AddIssue(new WindowsScanIssue(folderLocator, ToIssueKind(state), errorCode));
                    continue;
                }

                foreach (var child in children)
                {
                    pending.Push((child, null));
                }
            }

            var completed = new WindowsScanResult(
                scanId,
                WindowsScanCompletionState.Completed,
                discoveredCount,
                missingCount,
                permissionDeniedCount,
                unsupportedCount,
                errorCount,
                new ReadOnlyCollection<WindowsScanIssue>(issues));
            Emit(WindowsScanEventKind.Completed, result: completed);
            return completed;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            var cancelled = new WindowsScanResult(
                scanId,
                WindowsScanCompletionState.Cancelled,
                discoveredCount,
                missingCount,
                permissionDeniedCount,
                unsupportedCount,
                errorCount,
                new ReadOnlyCollection<WindowsScanIssue>(issues));
            Emit(WindowsScanEventKind.Completed, result: cancelled);
            return cancelled;
        }
        catch (Exception exception) when (IsExpectedFileSystemException(exception))
        {
            var (state, errorCode) = WindowsLibraryGateway.Classify(exception);
            AddIssue(new WindowsScanIssue(null, ToIssueKind(state), errorCode));
            var failed = new WindowsScanResult(
                scanId,
                WindowsScanCompletionState.Failed,
                discoveredCount,
                missingCount,
                permissionDeniedCount,
                unsupportedCount,
                errorCount,
                new ReadOnlyCollection<WindowsScanIssue>(issues));
            Emit(WindowsScanEventKind.Completed, result: failed);
            return failed;
        }
    }

    private static bool IsExpectedFileSystemException(Exception exception) =>
        exception is UnauthorizedAccessException
            or FileNotFoundException
            or DirectoryNotFoundException
            or IOException
            or ArgumentException
            or NotSupportedException
            or PathTooLongException;

    private static WindowsScanIssueKind ToIssueKind(WindowsResourceState state) =>
        state switch
        {
            WindowsResourceState.Missing => WindowsScanIssueKind.Missing,
            WindowsResourceState.PermissionDenied => WindowsScanIssueKind.PermissionDenied,
            WindowsResourceState.Unsupported => WindowsScanIssueKind.Unsupported,
            _ => WindowsScanIssueKind.Error,
        };
}
