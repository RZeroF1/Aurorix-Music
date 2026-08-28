#if WINDOWS_PLATFORM_CONTRACT_TESTS

using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace Aurorix.Platform.Windows.Extensions;

/// <summary>
/// Dependency-free probes for the public extension host boundary. They are
/// compiled by the temporary Windows contract runner, not shipped as runtime
/// behavior.
/// </summary>
public static class ExtensionContractTests
{
    public static async Task RunAllAsync()
    {
        BuiltInContributionRendersActionAndPlaceholder();
        RejectsUnsupportedSchemaAndCapabilities();
        RejectsInvalidOrderingAndDuplicateRoutes();
        OrdersActionsDeterministically();
        DisabledContributionFailsClosed();
        RejectsInvalidPageParameters();
        await DispatchesOnlyValidatedEnabledActions();
    }

    private static void BuiltInContributionRendersActionAndPlaceholder()
    {
        var registry = new ExtensionContributionRegistry();

        True(registry.RegisterBuiltInTestContribution(out var registration));
        True(registration.Accepted);

        var actions = registry.GetActions(ExtensionActionSlots.HomeQuickEntries);
        Equal(1, actions.Count);
        Equal(BuiltInTestContribution.ExtensionId, actions[0].ExtensionId);
        Equal(BuiltInTestContribution.ActionId, actions[0].ContributionId);
        Equal("Open the built-in extension test page", actions[0].AccessibilityText);

        var pages = registry.GetPages(ExtensionActionSlots.HomeContentSections);
        Equal(1, pages.Count);
        Equal(BuiltInTestContribution.PageRouteId, pages[0].RouteId);

        var page = registry.RenderPlaceholderPage(
            BuiltInTestContribution.PageRouteId,
            new Dictionary<string, string> { ["mode"] = "contract" });
        True(page is not null);
        Equal("contract", page!.Parameters["mode"]);
        Equal(
            "This extension page is provided as a host-rendered placeholder.",
            page.Message);

        Equal(1, registry.GetThemeTokenOverrides().Count);
    }

    private static void RejectsUnsupportedSchemaAndCapabilities()
    {
        var registry = new ExtensionContributionRegistry();
        var unsupportedSchema = new ExtensionManifest(
            "com.aurorix.schema-test",
            "Schema test",
            new ExtensionSchemaVersion(2, 0),
            Array.Empty<ExtensionCapabilityDeclaration>(),
            Array.Empty<IExtensionContribution>());

        True(!registry.TryRegister(unsupportedSchema, out var schemaResult));
        Equal(ExtensionRegistrationRejectionReason.UnsupportedSchema, schemaResult.RejectionReason);

        var unknownCapability = new ExtensionManifest(
            "com.aurorix.capability-test",
            "Capability test",
            ExtensionSchemaVersion.Current,
            new[] { new ExtensionCapabilityDeclaration("filesystem.write") },
            Array.Empty<IExtensionContribution>());

        True(!registry.TryRegister(unknownCapability, out var capabilityResult));
        Equal(
            ExtensionRegistrationRejectionReason.UnknownCapability,
            capabilityResult.RejectionReason);

        var unsupportedCapabilityVersion = new ExtensionManifest(
            "com.aurorix.capability-version-test",
            "Capability version test",
            ExtensionSchemaVersion.Current,
            new[]
            {
                new ExtensionCapabilityDeclaration(
                    ExtensionCapabilityNames.ActionSlots,
                    new ExtensionSchemaVersion(2, 0))
            },
            Array.Empty<IExtensionContribution>());

        True(!registry.TryRegister(
            unsupportedCapabilityVersion,
            out var capabilityVersionResult));
        Equal(
            ExtensionRegistrationRejectionReason.CapabilityVersion,
            capabilityVersionResult.RejectionReason);

        var undeclaredActionCapability = Action(
            "undeclared",
            ExtensionActionSlots.HomeQuickEntries,
            orderingHint: 0,
            requiredCapability: ExtensionCapabilityNames.ActionSlots);
        var missingDeclaration = Manifest(
            "com.aurorix.missing-capability",
            Array.Empty<ExtensionCapabilityDeclaration>(),
            undeclaredActionCapability);

        True(!registry.TryRegister(missingDeclaration, out var declarationResult));
        Equal(
            ExtensionRegistrationRejectionReason.CapabilityNotDeclared,
            declarationResult.RejectionReason);
    }

    private static void RejectsInvalidOrderingAndDuplicateRoutes()
    {
        var registry = new ExtensionContributionRegistry();
        var invalidOrdering = Manifest(
            "com.aurorix.ordering-test",
            Capabilities(ExtensionCapabilityNames.ActionSlots),
            Action(
                "too-late",
                ExtensionActionSlots.HomeQuickEntries,
                orderingHint: ExtensionContributionRegistry.MaximumOrderingHint + 1));

        True(!registry.TryRegister(invalidOrdering, out var orderingResult));
        Equal(
            ExtensionRegistrationRejectionReason.InvalidOrdering,
            orderingResult.RejectionReason);

        var invalidToken = Manifest(
            "com.aurorix.token-group-test",
            Capabilities(ExtensionCapabilityNames.ThemeTokenOverrides),
            new ExtensionThemeTokenOverride(
                "unreserved.value",
                ExtensionThemeTokenValue.Text("not accepted")));
        True(!registry.TryRegister(invalidToken, out var tokenResult));
        Equal(
            ExtensionRegistrationRejectionReason.InvalidThemeToken,
            tokenResult.RejectionReason);

        var first = Manifest(
            "com.aurorix.route-one",
            Capabilities(ExtensionCapabilityNames.PlaceholderPages),
            Page("first", "extension:shared-route"));
        var second = Manifest(
            "com.aurorix.route-two",
            Capabilities(ExtensionCapabilityNames.PlaceholderPages),
            Page("second", "extension:shared-route"));

        True(registry.TryRegister(first, out _));
        True(!registry.TryRegister(second, out var routeResult));
        Equal(
            ExtensionRegistrationRejectionReason.DuplicatePageRoute,
            routeResult.RejectionReason);
    }

    private static void OrdersActionsDeterministically()
    {
        var registry = new ExtensionContributionRegistry();
        var capabilities = Capabilities(ExtensionCapabilityNames.ActionSlots);
        True(registry.TryRegister(
            Manifest(
                "com.aurorix.order-b",
                capabilities,
                Action("b", ExtensionActionSlots.HomeQuickEntries, orderingHint: 10)),
            out _));
        True(registry.TryRegister(
            Manifest(
                "com.aurorix.order-a",
                capabilities,
                Action("a", ExtensionActionSlots.HomeQuickEntries, orderingHint: 10)),
            out _));
        True(registry.TryRegister(
            Manifest(
                "com.aurorix.order-first",
                capabilities,
                Action("first", ExtensionActionSlots.HomeQuickEntries, orderingHint: -10)),
            out _));

        var actions = registry.GetActions(ExtensionActionSlots.HomeQuickEntries);
        Equal(3, actions.Count);
        Equal("first", actions[0].ContributionId);
        Equal("a", actions[1].ContributionId);
        Equal("b", actions[2].ContributionId);
    }

    private static void DisabledContributionFailsClosed()
    {
        var registry = new ExtensionContributionRegistry();
        var manifest = new ExtensionManifest(
            BuiltInTestContribution.ExtensionId,
            "Built-in extension test",
            ExtensionSchemaVersion.Current,
            new[]
            {
                new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.ThemeTokenOverrides),
                new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.ActionSlots),
                new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.PlaceholderPages)
            },
            BuiltInTestContribution.CreateManifest().Contributions,
            isEnabled: false);

        True(registry.TryRegister(manifest, out _));
        True(registry.TrySetEnabled(BuiltInTestContribution.ExtensionId, false));
        Equal(0, registry.GetActions(ExtensionActionSlots.HomeQuickEntries).Count);
        Equal(0, registry.GetPages(ExtensionActionSlots.HomeContentSections).Count);
        Equal(0, registry.GetThemeTokenOverrides().Count);
        True(registry.RenderPlaceholderPage(BuiltInTestContribution.PageRouteId) is null);
    }

    private static void RejectsInvalidPageParameters()
    {
        var registry = new ExtensionContributionRegistry();
        True(registry.RegisterBuiltInTestContribution(out _));

        True(registry.RenderPlaceholderPage(
            BuiltInTestContribution.PageRouteId,
            new Dictionary<string, string> { ["unknown"] = "value" }) is null);
        True(registry.RenderPlaceholderPage(
            BuiltInTestContribution.PageRouteId,
            new Dictionary<string, string> { ["mode"] = new string('x', 513) }) is null);
    }

    private static async Task DispatchesOnlyValidatedEnabledActions()
    {
        var router = new RecordingRouter();
        var registry = new ExtensionContributionRegistry(
            ExtensionSchemaVersion.Current,
            actionRouter: router);
        True(registry.RegisterBuiltInTestContribution(out _));

        var dispatched = await registry.DispatchActionAsync(
            41,
            BuiltInTestContribution.ExtensionId,
            BuiltInTestContribution.ActionId);
        Equal(ExtensionActionDispatchStatus.Dispatched, dispatched.Status);
        Equal(1, router.Invocations.Count);

        var forged = await registry.DispatchActionAsync(
            new ExtensionActionInvocation(
                42,
                BuiltInTestContribution.ExtensionId,
                BuiltInTestContribution.ActionId,
                "com.aurorix.forged.command"));
        Equal(ExtensionActionDispatchStatus.Unknown, forged.Status);

        True(registry.TrySetEnabled(BuiltInTestContribution.ExtensionId, false));
        var disabled = await registry.DispatchActionAsync(
            43,
            BuiltInTestContribution.ExtensionId,
            BuiltInTestContribution.ActionId);
        Equal(ExtensionActionDispatchStatus.Disabled, disabled.Status);
        Equal(1, router.Invocations.Count);
    }

    private static ExtensionActionContribution Action(
        string id,
        string slot,
        int orderingHint,
        string requiredCapability = ExtensionCapabilityNames.ActionSlots) =>
        new(
            id,
            slot,
            id,
            ExtensionIcon.Named("Add"),
            $"com.aurorix.test.{id}",
            requiredCapability,
            orderingHint,
            accessibilityText: id);

    private static ExtensionPageContribution Page(string id, string routeId) =>
        new(
            id,
            routeId,
            ExtensionActionSlots.HomeContentSections,
            id,
            ExtensionIcon.Named("Page"));

    private static ExtensionManifest Manifest(
        string extensionId,
        IEnumerable<ExtensionCapabilityDeclaration> capabilities,
        params IExtensionContribution[] contributions) =>
        new(
            extensionId,
            extensionId,
            ExtensionSchemaVersion.Current,
            capabilities,
            contributions);

    private static ExtensionCapabilityDeclaration[] Capabilities(
        params string[] names) => names
            .Select(static name => new ExtensionCapabilityDeclaration(name))
            .ToArray();

    private sealed class RecordingRouter : IExtensionActionRouter
    {
        public List<ExtensionActionInvocation> Invocations { get; } = new();

        public ValueTask<ExtensionActionDispatchResult> DispatchAsync(
            ExtensionActionInvocation invocation,
            CancellationToken cancellationToken = default)
        {
            Invocations.Add(invocation);
            return ValueTask.FromResult(new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Dispatched,
                invocation));
        }
    }

    private static void True(bool condition)
    {
        if (!condition)
        {
            throw new InvalidOperationException("Extension contract assertion failed.");
        }
    }

    private static void Equal<T>(T expected, T actual)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new InvalidOperationException(
                $"Extension contract assertion failed. Expected '{expected}', actual '{actual}'.");
        }
    }
}

#endif
