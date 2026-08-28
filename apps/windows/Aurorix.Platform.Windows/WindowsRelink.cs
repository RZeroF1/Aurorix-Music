using System.Security.Cryptography;

namespace Aurorix.Platform.Windows;

public sealed record WindowsRelinkObservation(
    WindowsMediaLocator Locator,
    WindowsFileMetadata Metadata,
    WindowsRelinkFingerprint? QuickHash);

public interface IWindowsQuickHashProvider
{
    WindowsRelinkFingerprint? TryCompute(WindowsMediaLocator locator);
}

/// <summary>
/// Bounded quick fingerprint used only as a relink candidate input. It hashes
/// the first and last 64 KiB plus the file length, never the entire file.
/// </summary>
public sealed class WindowsQuickHashProvider : IWindowsQuickHashProvider
{
    private const int WindowBytes = 64 * 1024;
    private const string Algorithm = "sha256-window";
    private const string Version = "1";

    public WindowsRelinkFingerprint? TryCompute(WindowsMediaLocator locator)
    {
        ArgumentNullException.ThrowIfNull(locator);
        if (locator.Kind != WindowsLocatorKind.File)
        {
            return null;
        }

        try
        {
            using var stream = new FileStream(
                locator.NormalizedPath,
                FileMode.Open,
                FileAccess.Read,
                FileShare.ReadWrite | FileShare.Delete,
                WindowBytes,
                FileOptions.SequentialScan);
            using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
            AppendLength(hash, stream.Length);

            var head = ReadWindow(stream, 0, Math.Min(WindowBytes, stream.Length));
            hash.AppendData(head);

            if (stream.Length > WindowBytes)
            {
                var tailOffset = stream.Length - WindowBytes;
                var tail = ReadWindow(stream, tailOffset, WindowBytes);
                hash.AppendData(tail);
            }

            return new WindowsRelinkFingerprint(
                Algorithm,
                Version,
                Convert.ToHexString(hash.GetHashAndReset()).ToLowerInvariant());
        }
        catch (IOException)
        {
            return null;
        }
        catch (UnauthorizedAccessException)
        {
            return null;
        }
        catch (ArgumentException)
        {
            return null;
        }
    }

    private static byte[] ReadWindow(FileStream stream, long offset, long requestedLength)
    {
        stream.Position = offset;
        var buffer = new byte[checked((int)requestedLength)];
        var read = 0;
        while (read < buffer.Length)
        {
            var count = stream.Read(buffer, read, buffer.Length - read);
            if (count == 0)
            {
                break;
            }

            read += count;
        }

        return read == buffer.Length ? buffer : buffer[..read];
    }

    private static void AppendLength(IncrementalHash hash, long length)
    {
        Span<byte> bytes = stackalloc byte[sizeof(long)];
        BitConverter.TryWriteBytes(bytes, length);
        hash.AppendData(bytes);
    }
}

/// <summary>
/// Pure relink policy. File identity wins; size and quick hash are accepted
/// only when exactly one candidate matches. This prevents a title or size
/// collision from silently changing the catalog identity.
/// </summary>
public static class WindowsRelinkMatcher
{
    public static IReadOnlyList<WindowsRelinkCandidate> Match(
        WindowsRelinkRequest request,
        IEnumerable<WindowsRelinkObservation> observations)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(observations);

        var allCandidates = observations
            .Where(static observation => observation.Locator.Kind == WindowsLocatorKind.File)
            .OrderBy(static observation => observation.Locator.NormalizedPath, StringComparer.OrdinalIgnoreCase)
            .ToArray();

        if (request.OriginalLocator.Identity is { } originalIdentity)
        {
            var identityMatches = allCandidates
                .Where(candidate => candidate.Metadata.Identity == originalIdentity)
                .Select(candidate => new WindowsRelinkCandidate(
                    candidate.Locator,
                    candidate.Metadata,
                    WindowsRelinkMatchKind.FileIdentity))
                .ToArray();
            if (identityMatches.Length > 0)
            {
                return identityMatches;
            }
        }

        if (request.ExpectedQuickHash is not { } expectedQuickHash)
        {
            return Array.Empty<WindowsRelinkCandidate>();
        }

        var hashMatches = allCandidates
            .Where(candidate => candidate.Metadata.SizeBytes == request.ExpectedSizeBytes)
            .Where(candidate => candidate.QuickHash is { } actual &&
                string.Equals(actual.Algorithm, expectedQuickHash.Algorithm, StringComparison.Ordinal) &&
                string.Equals(actual.Version, expectedQuickHash.Version, StringComparison.Ordinal) &&
                string.Equals(actual.Value, expectedQuickHash.Value, StringComparison.OrdinalIgnoreCase))
            .Select(candidate => new WindowsRelinkCandidate(
                candidate.Locator,
                candidate.Metadata,
                WindowsRelinkMatchKind.SizeAndQuickHash))
            .ToArray();

        // An ambiguous result is retained as candidates by the gateway and
        // must be resolved by the user/Core identity policy.
        return hashMatches;
    }
}
