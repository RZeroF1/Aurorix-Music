namespace Aurorix.Platform.Windows.Extensions;

/// <summary>
/// A deterministic declaration used to prove the host rendering path. This is
/// data-only and intentionally has no extension assembly or runtime entrypoint.
/// </summary>
public static class BuiltInTestContribution
{
    public const string ExtensionId = "com.aurorix.builtin-test";
    public const string ActionId = "show-test-page";
    public const string PageRouteId = "extension:test-page";

    public static ExtensionManifest CreateManifest()
    {
        return new ExtensionManifest(
            ExtensionId,
            "Built-in extension test",
            ExtensionSchemaVersion.Current,
            new[]
            {
                new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.ThemeTokenOverrides),
                new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.ActionSlots),
                new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.PlaceholderPages)
            },
            new IExtensionContribution[]
            {
                new ExtensionThemeTokenOverride(
                    "accent.extensiontest",
                    ExtensionThemeTokenValue.Color("#FF2F6FED"),
                    orderingHint: 100),
                new ExtensionActionContribution(
                    ActionId,
                    ExtensionActionSlots.HomeQuickEntries,
                    "Extension test",
                    ExtensionIcon.Named("Add"),
                    "com.aurorix.builtin_test.show_page",
                    ExtensionCapabilityNames.ActionSlots,
                    orderingHint: 100,
                    accessibilityText: "Open the built-in extension test page"),
                new ExtensionPageContribution(
                    "test-page",
                    PageRouteId,
                    ExtensionActionSlots.HomeContentSections,
                    "Extension test page",
                    ExtensionIcon.Named("Page"),
                    new ExtensionPageParameterSchema(new[]
                    {
                        new ExtensionPageParameter("mode", ExtensionPageParameterType.String)
                    }),
                    ExtensionPageLifecycleScope.Navigation,
                    ExtensionSchemaVersion.Current,
                    ExtensionPageBackBehavior.HostNavigation,
                    ExtensionPageDeepLinkBehavior.HostRouteOnly,
                    ExtensionCapabilityNames.PlaceholderPages,
                    orderingHint: 100)
            });
    }
}
