namespace Aurorix.Platform.Windows;

/// <summary>
/// Normalizes paths at the Windows adapter boundary without resolving links.
/// Link resolution is intentionally left to the file-system probe.
/// </summary>
public static class WindowsPathNormalizer
{
    public static string NormalizePath(string path)
    {
        if (!TryNormalizePath(path, out var normalizedPath, out var errorCode))
        {
            throw new ArgumentException($"The selected Windows path is invalid ({errorCode}).", nameof(path));
        }

        return normalizedPath;
    }

    public static bool TryNormalizePath(
        string? path,
        out string normalizedPath,
        out WindowsGatewayErrorCode errorCode)
    {
        normalizedPath = string.Empty;
        errorCode = WindowsGatewayErrorCode.None;

        if (string.IsNullOrWhiteSpace(path))
        {
            errorCode = WindowsGatewayErrorCode.EmptyPath;
            return false;
        }

        if (path.IndexOf('\0') >= 0)
        {
            errorCode = WindowsGatewayErrorCode.InvalidPath;
            return false;
        }

        if (path.IndexOfAny(['*', '?']) >= 0)
        {
            errorCode = WindowsGatewayErrorCode.WildcardPath;
            return false;
        }

        // Do not resolve a relative or drive-relative picker value against
        // the process working directory. A locator must be fully qualified
        // before it enters the platform boundary.
        if (!Path.IsPathFullyQualified(path))
        {
            errorCode = WindowsGatewayErrorCode.PathNotFullyQualified;
            return false;
        }

        try
        {
            var fullPath = Path.GetFullPath(path);
            if (!Path.IsPathFullyQualified(fullPath))
            {
                errorCode = WindowsGatewayErrorCode.PathNotFullyQualified;
                return false;
            }

            fullPath = fullPath.Replace(Path.AltDirectorySeparatorChar, Path.DirectorySeparatorChar);
            var root = Path.GetPathRoot(fullPath);
            if (root is null)
            {
                errorCode = WindowsGatewayErrorCode.PathNotFullyQualified;
                return false;
            }

            var trimmed = fullPath.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            var trimmedRoot = root.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            normalizedPath = string.Equals(trimmed, trimmedRoot, StringComparison.OrdinalIgnoreCase)
                ? root
                : trimmed;

            return true;
        }
        catch (ArgumentException)
        {
            errorCode = WindowsGatewayErrorCode.InvalidPath;
            return false;
        }
        catch (NotSupportedException)
        {
            errorCode = WindowsGatewayErrorCode.InvalidPath;
            return false;
        }
        catch (PathTooLongException)
        {
            errorCode = WindowsGatewayErrorCode.InvalidPath;
            return false;
        }
    }

    public static bool AreEquivalent(string left, string right)
    {
        ArgumentNullException.ThrowIfNull(left);
        ArgumentNullException.ThrowIfNull(right);

        return string.Equals(
            NormalizePath(left),
            NormalizePath(right),
            StringComparison.OrdinalIgnoreCase);
    }
}
