using System.Collections.ObjectModel;

namespace Aurorix.Platform.Windows;

/// <summary>
/// Thin picker adapter contract. A WinUI host supplies the native picker
/// implementation; raw paths are transient here and are normalized before
/// returning from this gateway.
/// </summary>
public interface IWindowsPickerAdapter
{
    ValueTask<WindowsRawPickerSelection> PickFolderAsync(CancellationToken cancellationToken = default);

    ValueTask<WindowsRawPickerSelection> PickFilesAsync(CancellationToken cancellationToken = default);
}

public interface IWindowsFilePickerGateway
{
    ValueTask<WindowsPickerResult> PickFolderAsync(CancellationToken cancellationToken = default);

    ValueTask<WindowsPickerResult> PickFilesAsync(CancellationToken cancellationToken = default);
}

public sealed class WindowsFilePickerGateway : IWindowsFilePickerGateway
{
    private readonly IWindowsPickerAdapter _adapter;

    public WindowsFilePickerGateway(IWindowsPickerAdapter adapter)
    {
        _adapter = adapter ?? throw new ArgumentNullException(nameof(adapter));
    }

    public ValueTask<WindowsPickerResult> PickFolderAsync(CancellationToken cancellationToken = default) =>
        PickAsync(WindowsLocatorKind.Folder, adapter => adapter.PickFolderAsync(cancellationToken), cancellationToken);

    public ValueTask<WindowsPickerResult> PickFilesAsync(CancellationToken cancellationToken = default) =>
        PickAsync(WindowsLocatorKind.File, adapter => adapter.PickFilesAsync(cancellationToken), cancellationToken);

    private async ValueTask<WindowsPickerResult> PickAsync(
        WindowsLocatorKind expectedKind,
        Func<IWindowsPickerAdapter, ValueTask<WindowsRawPickerSelection>> pick,
        CancellationToken cancellationToken)
    {
        var selection = await pick(_adapter).ConfigureAwait(false);
        cancellationToken.ThrowIfCancellationRequested();

        if (selection.Outcome == WindowsPickerOutcome.Cancelled)
        {
            return WindowsPickerResult.Cancelled();
        }

        if (selection.Outcome != WindowsPickerOutcome.Selected)
        {
            return new WindowsPickerResult(
                WindowsPickerOutcome.InvalidSelection,
                Array.Empty<WindowsMediaLocator>(),
                [new WindowsPickerIssue(null, WindowsGatewayErrorCode.InvalidSelection)]);
        }

        if (selection.Paths.Count == 0)
        {
            return new WindowsPickerResult(
                WindowsPickerOutcome.InvalidSelection,
                Array.Empty<WindowsMediaLocator>(),
                [new WindowsPickerIssue(null, WindowsGatewayErrorCode.InvalidSelection)]);
        }

        var locators = new List<WindowsMediaLocator>(selection.Paths.Count);
        var issues = new List<WindowsPickerIssue>();
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        foreach (var path in selection.Paths)
        {
            cancellationToken.ThrowIfCancellationRequested();

            if (!WindowsPathNormalizer.TryNormalizePath(path, out var normalizedPath, out var errorCode))
            {
                issues.Add(new WindowsPickerIssue(null, errorCode));
                continue;
            }

            if (!seen.Add(normalizedPath))
            {
                continue;
            }

            locators.Add(WindowsMediaLocator.FromNormalizedPath(normalizedPath, expectedKind));
        }

        var outcome = locators.Count > 0 && issues.Count == 0
            ? WindowsPickerOutcome.Selected
            : WindowsPickerOutcome.InvalidSelection;

        return new WindowsPickerResult(
            outcome,
            new ReadOnlyCollection<WindowsMediaLocator>(locators),
            new ReadOnlyCollection<WindowsPickerIssue>(issues));
    }
}
