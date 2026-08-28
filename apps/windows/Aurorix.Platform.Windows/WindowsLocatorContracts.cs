using System.Collections.ObjectModel;

namespace Aurorix.Platform.Windows;

/// <summary>
/// The two Windows resource kinds that can be selected as a local-library root.
/// </summary>
public enum WindowsLocatorKind
{
    File,
    Folder,
}

public enum WindowsResourceState
{
    Available,
    Missing,
    PermissionDenied,
    Unsupported,
    Error,
}

public enum WindowsGatewayErrorCode
{
    None,
    EmptyPath,
    InvalidPath,
    PathNotFullyQualified,
    WildcardPath,
    NotFound,
    AccessDenied,
    TypeMismatch,
    ReparsePointSkipped,
    IoError,
    NoRoots,
    InvalidSelection,
    Cancelled,
}

/// <summary>
/// Stable Windows identity observed from a file or directory handle. The
/// handle itself is deliberately not retained in this value.
/// </summary>
public readonly record struct WindowsFileIdentity(uint VolumeSerialNumber, ulong FileIndex)
{
    public override string ToString() => $"{VolumeSerialNumber:x8}:{FileIndex:x16}";
}

/// <summary>
/// A persistent Windows locator. It is a platform value, not a runtime file
/// handle or lease. It has no serializer and must not be copied into FFI,
/// Sync, or durable playback-intent messages by this project.
/// </summary>
public sealed record WindowsMediaLocator
{
    private WindowsMediaLocator(string normalizedPath, WindowsLocatorKind kind, WindowsFileIdentity? identity)
    {
        NormalizedPath = normalizedPath;
        Kind = kind;
        Identity = identity;
    }

    public string NormalizedPath { get; }

    public WindowsLocatorKind Kind { get; }

    public WindowsFileIdentity? Identity { get; }

    public static WindowsMediaLocator Create(string path, WindowsLocatorKind kind)
    {
        ArgumentNullException.ThrowIfNull(path);
        return FromNormalizedPath(WindowsPathNormalizer.NormalizePath(path), kind);
    }

    internal static WindowsMediaLocator FromNormalizedPath(
        string normalizedPath,
        WindowsLocatorKind kind,
        WindowsFileIdentity? identity = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(normalizedPath);
        return new WindowsMediaLocator(normalizedPath, kind, identity);
    }

    public WindowsMediaLocator WithIdentity(WindowsFileIdentity identity) =>
        new(NormalizedPath, Kind, identity);

    // Avoid accidentally printing a local path into logs or exception text.
    public override string ToString() => Identity is { } value
        ? $"{Kind} ({value})"
        : Kind.ToString();
}

public sealed record WindowsFileMetadata(
    long SizeBytes,
    DateTimeOffset LastWriteTimeUtc,
    WindowsFileIdentity? Identity,
    bool IsReparsePoint = false)
{
    public WindowsFileMetadata(long sizeBytes, DateTimeOffset lastWriteTimeUtc, WindowsFileIdentity? identity)
        : this(sizeBytes, lastWriteTimeUtc, identity, false)
    {
    }
}

public sealed record WindowsLocatorProbeResult(
    WindowsMediaLocator Locator,
    WindowsResourceState State,
    WindowsFileMetadata? Metadata,
    WindowsGatewayErrorCode ErrorCode = WindowsGatewayErrorCode.None);

public enum WindowsPickerOutcome
{
    Selected,
    Cancelled,
    InvalidSelection,
}

public sealed record WindowsRawPickerSelection(
    WindowsPickerOutcome Outcome,
    IReadOnlyList<string> Paths)
{
    public static WindowsRawPickerSelection Cancelled() =>
        new(WindowsPickerOutcome.Cancelled, Array.Empty<string>());
}

public sealed record WindowsPickerIssue(
    string? DisplayName,
    WindowsGatewayErrorCode ErrorCode);

public sealed record WindowsPickerResult(
    WindowsPickerOutcome Outcome,
    IReadOnlyList<WindowsMediaLocator> Locators,
    IReadOnlyList<WindowsPickerIssue> Issues)
{
    public static WindowsPickerResult Cancelled() =>
        new(WindowsPickerOutcome.Cancelled, Array.Empty<WindowsMediaLocator>(), Array.Empty<WindowsPickerIssue>());
}

public sealed record WindowsScanRequest
{
    public WindowsScanRequest(IEnumerable<WindowsMediaLocator> roots)
    {
        ArgumentNullException.ThrowIfNull(roots);

        var uniqueRoots = roots
            .Where(static root => root is not null)
            .GroupBy(static root => root.NormalizedPath, StringComparer.OrdinalIgnoreCase)
            .Select(static group => group.First())
            .ToArray();

        if (uniqueRoots.Length == 0)
        {
            throw new ArgumentException("At least one user-selected root is required.", nameof(roots));
        }

        Roots = new ReadOnlyCollection<WindowsMediaLocator>(uniqueRoots);
    }

    public IReadOnlyList<WindowsMediaLocator> Roots { get; }
}

public sealed record WindowsScanEntry(
    WindowsMediaLocator Locator,
    WindowsFileMetadata Metadata);

public enum WindowsScanIssueKind
{
    Missing,
    PermissionDenied,
    Unsupported,
    Error,
}

public sealed record WindowsScanIssue(
    WindowsMediaLocator? Locator,
    WindowsScanIssueKind Kind,
    WindowsGatewayErrorCode ErrorCode);

public enum WindowsScanEventKind
{
    Discovered,
    Issue,
    Completed,
}

public sealed record WindowsScanEvent(
    Guid ScanId,
    long Sequence,
    WindowsScanEventKind Kind,
    WindowsScanEntry? Entry = null,
    WindowsScanIssue? Issue = null,
    WindowsScanResult? Result = null);

public enum WindowsScanCompletionState
{
    Completed,
    Cancelled,
    Failed,
}

public sealed record WindowsScanResult(
    Guid ScanId,
    WindowsScanCompletionState State,
    long DiscoveredCount,
    long MissingCount,
    long PermissionDeniedCount,
    long UnsupportedCount,
    long ErrorCount,
    IReadOnlyList<WindowsScanIssue> Issues);

public sealed record WindowsRelinkFingerprint
{
    public WindowsRelinkFingerprint(string algorithm, string version, string value)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(algorithm);
        ArgumentException.ThrowIfNullOrWhiteSpace(version);
        ArgumentException.ThrowIfNullOrWhiteSpace(value);

        Algorithm = algorithm;
        Version = version;
        Value = value;
    }

    public string Algorithm { get; }

    public string Version { get; }

    public string Value { get; }
}

public sealed record WindowsRelinkRequest
{
    public WindowsRelinkRequest(
        WindowsMediaLocator originalLocator,
        long expectedSizeBytes,
        WindowsRelinkFingerprint? expectedQuickHash,
        IEnumerable<WindowsMediaLocator> searchRoots)
    {
        ArgumentNullException.ThrowIfNull(originalLocator);
        ArgumentNullException.ThrowIfNull(searchRoots);

        if (expectedSizeBytes < 0)
        {
            throw new ArgumentOutOfRangeException(nameof(expectedSizeBytes));
        }

        var uniqueRoots = searchRoots
            .Where(static root => root is not null)
            .GroupBy(static root => root.NormalizedPath, StringComparer.OrdinalIgnoreCase)
            .Select(static group => group.First())
            .ToArray();

        if (uniqueRoots.Length == 0)
        {
            throw new ArgumentException("At least one user-selected search root is required.", nameof(searchRoots));
        }

        OriginalLocator = originalLocator;
        ExpectedSizeBytes = expectedSizeBytes;
        ExpectedQuickHash = expectedQuickHash;
        SearchRoots = new ReadOnlyCollection<WindowsMediaLocator>(uniqueRoots);
    }

    public WindowsMediaLocator OriginalLocator { get; }

    public long ExpectedSizeBytes { get; }

    public WindowsRelinkFingerprint? ExpectedQuickHash { get; }

    public IReadOnlyList<WindowsMediaLocator> SearchRoots { get; }
}

public sealed record WindowsRelinkCandidate(
    WindowsMediaLocator Locator,
    WindowsFileMetadata Metadata,
    WindowsRelinkMatchKind MatchKind);

public enum WindowsRelinkMatchKind
{
    FileIdentity,
    SizeAndQuickHash,
}

public enum WindowsRelinkOutcome
{
    Found,
    NotFound,
    Ambiguous,
    PermissionDenied,
    Cancelled,
}

public sealed record WindowsRelinkResult(
    WindowsRelinkOutcome Outcome,
    IReadOnlyList<WindowsRelinkCandidate> Candidates,
    IReadOnlyList<WindowsScanIssue> Issues);
