using System.Collections.ObjectModel;

namespace Aurorix.Platform.Windows.Extensions;

/// <summary>
/// Host-rendered page model. This is deliberately a placeholder contract; it
/// has no page factory, XAML, assembly, process, or Core object.
/// </summary>
public sealed record ExtensionPlaceholderPage
{
    public ExtensionPlaceholderPage(
        string extensionId,
        ExtensionPageContribution contribution,
        IReadOnlyDictionary<string, string> parameters)
    {
        ExtensionId = extensionId;
        ContributionId = contribution.ContributionId;
        RouteId = contribution.RouteId;
        ParentSlotId = contribution.ParentSlotId;
        Title = contribution.Title;
        Icon = contribution.Icon;
        Parameters = new ReadOnlyDictionary<string, string>(
            new Dictionary<string, string>(parameters, StringComparer.Ordinal));
        LifecycleScope = contribution.LifecycleScope;
        BackBehavior = contribution.BackBehavior;
        DeepLinkBehavior = contribution.DeepLinkBehavior;
        Message = "This extension page is provided as a host-rendered placeholder.";
    }

    public string ExtensionId { get; }

    public string ContributionId { get; }

    public string RouteId { get; }

    public string ParentSlotId { get; }

    public string Title { get; }

    public ExtensionIcon Icon { get; }

    public IReadOnlyDictionary<string, string> Parameters { get; }

    public ExtensionPageLifecycleScope LifecycleScope { get; }

    public ExtensionPageBackBehavior BackBehavior { get; }

    public ExtensionPageDeepLinkBehavior DeepLinkBehavior { get; }

    public string Message { get; }
}
