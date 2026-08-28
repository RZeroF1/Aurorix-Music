namespace Aurorix.Platform.Windows.Extensions;

/// <summary>
/// In-memory host registry for declarative extension contributions. Registration
/// is all-or-nothing and every projection is derived from enabled declarations.
/// </summary>
public sealed class ExtensionContributionRegistry
{
    public const int MinimumOrderingHint = -10_000;
    public const int MaximumOrderingHint = 10_000;

    private readonly object _gate = new();
    private readonly Dictionary<string, ExtensionSchemaVersion> _hostCapabilities;
    private readonly Dictionary<string, RegisteredManifest> _manifests = new(
        StringComparer.Ordinal);
    private readonly ExtensionSchemaVersion _hostSchema;
    private readonly IExtensionActionRouter? _actionRouter;

    public ExtensionContributionRegistry()
        : this(ExtensionSchemaVersion.Current)
    {
    }

    public ExtensionContributionRegistry(
        ExtensionSchemaVersion hostSchema,
        IEnumerable<ExtensionCapabilityDeclaration>? supportedCapabilities = null,
        IExtensionActionRouter? actionRouter = null)
    {
        if (!hostSchema.IsValid)
        {
            throw new ArgumentOutOfRangeException(nameof(hostSchema));
        }

        _hostSchema = hostSchema;
        _actionRouter = actionRouter;
        _hostCapabilities = CreateHostCapabilities(supportedCapabilities);
    }

    public ExtensionSchemaVersion HostSchema => _hostSchema;

    /// <summary>
    /// Registers or replaces one complete manifest. A rejected manifest leaves
    /// the previous registration untouched.
    /// </summary>
    public bool TryRegister(
        ExtensionManifest? manifest,
        out ExtensionRegistrationResult result)
    {
        lock (_gate)
        {
            if (manifest is null)
            {
                result = ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.NullManifest,
                    "The extension manifest is required.");
                return false;
            }

            result = ValidateManifest(manifest);
            if (!result.Accepted)
            {
                return false;
            }

            _manifests[manifest.ExtensionId] = new RegisteredManifest(manifest);
            result = ExtensionRegistrationResult.Registered;
            return true;
        }
    }

    /// <summary>
    /// Installs the deterministic built-in declaration used by host contract
    /// probes. It does not activate an extension runtime.
    /// </summary>
    public bool RegisterBuiltInTestContribution(
        out ExtensionRegistrationResult result) =>
        TryRegister(BuiltInTestContribution.CreateManifest(), out result);

    public bool TrySetEnabled(string extensionId, bool isEnabled)
    {
        if (!ExtensionContractValidation.IsBoundedIdentifier(extensionId))
        {
            return false;
        }

        lock (_gate)
        {
            if (!_manifests.TryGetValue(extensionId, out var registered))
            {
                return false;
            }

            registered.IsEnabled = isEnabled;
            return true;
        }
    }

    public IReadOnlyList<ExtensionRegistryEntry> GetExtensions()
    {
        lock (_gate)
        {
            return _manifests.Values
                .OrderBy(static manifest => manifest.Manifest.ExtensionId, StringComparer.Ordinal)
                .Select(static manifest => new ExtensionRegistryEntry(
                    manifest.Manifest.ExtensionId,
                    manifest.Manifest.DisplayName,
                    manifest.Manifest.SchemaVersion,
                    manifest.IsEnabled,
                    manifest.Manifest.Contributions.OfType<ExtensionActionContribution>().Count(),
                    manifest.Manifest.Contributions.OfType<ExtensionPageContribution>().Count(),
                    manifest.Manifest.Contributions.OfType<ExtensionThemeTokenOverride>().Count()))
                .ToArray();
        }
    }

    /// <summary>
    /// Returns visible host-rendered action metadata in stable order.
    /// </summary>
    public IReadOnlyList<ExtensionActionRenderModel> GetActions(
        string slotId,
        ExtensionHostContext? context = null)
    {
        if (!ExtensionActionSlots.IsKnown(slotId))
        {
            return Array.Empty<ExtensionActionRenderModel>();
        }

        context ??= ExtensionHostContext.Empty;
        lock (_gate)
        {
            return _manifests.Values
                .Where(static manifest => manifest.IsEnabled)
                .SelectMany(static manifest => manifest.Manifest.Contributions
                    .OfType<ExtensionActionContribution>()
                    .Select(contribution => (manifest.Manifest.ExtensionId, Contribution: contribution)))
                .Where(item => string.Equals(item.Contribution.SlotId, slotId, StringComparison.Ordinal))
                .Where(item => item.Contribution.Visibility.Matches(context))
                .OrderBy(item => item.Contribution.OrderingHint)
                .ThenBy(item => item.ExtensionId, StringComparer.Ordinal)
                .ThenBy(item => item.Contribution.ContributionId, StringComparer.Ordinal)
                .Select(item => new ExtensionActionRenderModel(
                    item.ExtensionId,
                    item.Contribution))
                .ToArray();
        }
    }

    public IReadOnlyList<ExtensionActionRenderModel> GetAllActions(
        ExtensionHostContext? context = null)
    {
        context ??= ExtensionHostContext.Empty;
        lock (_gate)
        {
            return ExtensionActionSlots.All
                .SelectMany(slot => GetActionsWithoutLock(slot, context))
                .ToArray();
        }
    }

    /// <summary>
    /// Returns theme overrides in merge order. Duplicate token IDs are retained
    /// so a later, deterministically ordered extension can override an earlier
    /// one at the host theme layer.
    /// </summary>
    public IReadOnlyList<ExtensionThemeTokenOverride> GetThemeTokenOverrides()
    {
        lock (_gate)
        {
            return _manifests.Values
                .Where(static manifest => manifest.IsEnabled)
                .SelectMany(static manifest => manifest.Manifest.Contributions
                    .OfType<ExtensionThemeTokenOverride>()
                    .Select(contribution => (manifest.Manifest.ExtensionId, Contribution: contribution)))
                .OrderBy(item => item.Contribution.OrderingHint)
                .ThenBy(item => item.ExtensionId, StringComparer.Ordinal)
                .ThenBy(item => item.Contribution.TokenId, StringComparer.Ordinal)
                .Select(static item => item.Contribution)
                .ToArray();
        }
    }

    public IReadOnlyList<ExtensionPageContribution> GetPages(string parentSlotId)
    {
        if (!ExtensionActionSlots.IsKnown(parentSlotId))
        {
            return Array.Empty<ExtensionPageContribution>();
        }

        lock (_gate)
        {
            return _manifests.Values
                .Where(static manifest => manifest.IsEnabled)
                .SelectMany(static manifest => manifest.Manifest.Contributions
                    .OfType<ExtensionPageContribution>()
                    .Select(contribution => (manifest.Manifest.ExtensionId, Contribution: contribution)))
                .Where(item => string.Equals(
                    item.Contribution.ParentSlotId,
                    parentSlotId,
                    StringComparison.Ordinal))
                .OrderBy(item => item.Contribution.OrderingHint)
                .ThenBy(item => item.ExtensionId, StringComparer.Ordinal)
                .ThenBy(item => item.Contribution.RouteId, StringComparer.Ordinal)
                .Select(static item => item.Contribution)
                .ToArray();
        }
    }

    /// <summary>
    /// Creates the only page representation available in Gate 3: a fixed host
    /// placeholder. Disabled pages, disabled deep links, unknown routes, and
    /// invalid parameters all return null.
    /// </summary>
    public ExtensionPlaceholderPage? RenderPlaceholderPage(
        string routeId,
        IReadOnlyDictionary<string, string>? parameters = null)
    {
        if (!ExtensionContractValidation.IsBoundedIdentifier(routeId, allowColon: true))
        {
            return null;
        }

        var suppliedParameters = parameters ?? new Dictionary<string, string>(StringComparer.Ordinal);
        lock (_gate)
        {
            foreach (var registered in _manifests.Values.Where(static manifest => manifest.IsEnabled))
            {
                var page = registered.Manifest.Contributions
                    .OfType<ExtensionPageContribution>()
                    .FirstOrDefault(candidate => string.Equals(
                        candidate.RouteId,
                        routeId,
                        StringComparison.Ordinal));
                if (page is null || page.DeepLinkBehavior == ExtensionPageDeepLinkBehavior.Disabled)
                {
                    continue;
                }

                if (!page.ParameterSchema.TryValidateValues(suppliedParameters, out _))
                {
                    return null;
                }

                return new ExtensionPlaceholderPage(
                    registered.Manifest.ExtensionId,
                    page,
                    suppliedParameters);
            }
        }

        return null;
    }

    public ValueTask<ExtensionActionDispatchResult> DispatchActionAsync(
        ExtensionActionInvocation invocation,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(invocation);
        return DispatchActionCoreAsync(invocation, cancellationToken);
    }

    public ValueTask<ExtensionActionDispatchResult> DispatchActionAsync(
        ulong requestId,
        string extensionId,
        string contributionId,
        CancellationToken cancellationToken = default)
    {
        var action = FindAction(extensionId, contributionId, out var isDisabled);
        var commandNamespace = action?.CommandNamespace ?? string.Empty;
        var invocation = new ExtensionActionInvocation(
            requestId,
            extensionId ?? string.Empty,
            contributionId ?? string.Empty,
            commandNamespace);

        if (isDisabled)
        {
            return ValueTask.FromResult(new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Disabled,
                invocation));
        }

        if (action is null)
        {
            return ValueTask.FromResult(new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Unknown,
                invocation));
        }

        return DispatchActionCoreAsync(invocation, cancellationToken);
    }

    private async ValueTask<ExtensionActionDispatchResult> DispatchActionCoreAsync(
        ExtensionActionInvocation invocation,
        CancellationToken cancellationToken)
    {
        var action = FindAction(
            invocation.ExtensionId,
            invocation.ContributionId,
            out var isDisabled);
        if (isDisabled)
        {
            return new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Disabled,
                invocation);
        }

        if (action is null || !string.Equals(
                action.CommandNamespace,
                invocation.CommandNamespace,
                StringComparison.Ordinal))
        {
            return new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Unknown,
                invocation);
        }

        if (_actionRouter is null)
        {
            return new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Unavailable,
                invocation);
        }

        try
        {
            cancellationToken.ThrowIfCancellationRequested();
            var result = await _actionRouter.DispatchAsync(invocation, cancellationToken)
                .ConfigureAwait(false);
            return result.Status == ExtensionActionDispatchStatus.Dispatched
                ? new ExtensionActionDispatchResult(
                    ExtensionActionDispatchStatus.Dispatched,
                    invocation)
                : new ExtensionActionDispatchResult(
                    ExtensionActionDispatchStatus.Failed,
                    invocation);
        }
        catch (OperationCanceledException)
        {
            return new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Canceled,
                invocation);
        }
        catch (Exception)
        {
            // Do not expose extension exception text across the host boundary.
            return new ExtensionActionDispatchResult(
                ExtensionActionDispatchStatus.Failed,
                invocation);
        }
    }

    private ExtensionActionContribution? FindAction(
        string extensionId,
        string contributionId,
        out bool isDisabled)
    {
        if (string.IsNullOrEmpty(extensionId) || string.IsNullOrEmpty(contributionId))
        {
            isDisabled = false;
            return null;
        }

        lock (_gate)
        {
            if (!_manifests.TryGetValue(extensionId, out var registered))
            {
                isDisabled = false;
                return null;
            }

            var action = registered.Manifest.Contributions
                .OfType<ExtensionActionContribution>()
                .FirstOrDefault(candidate => string.Equals(
                    candidate.ContributionId,
                    contributionId,
                    StringComparison.Ordinal));
            isDisabled = action is not null && !registered.IsEnabled;
            return action;
        }
    }

    private IReadOnlyList<ExtensionActionRenderModel> GetActionsWithoutLock(
        string slotId,
        ExtensionHostContext context)
    {
        return _manifests.Values
            .Where(static manifest => manifest.IsEnabled)
            .SelectMany(static manifest => manifest.Manifest.Contributions
                .OfType<ExtensionActionContribution>()
                .Select(contribution => (manifest.Manifest.ExtensionId, Contribution: contribution)))
            .Where(item => string.Equals(item.Contribution.SlotId, slotId, StringComparison.Ordinal))
            .Where(item => item.Contribution.Visibility.Matches(context))
            .OrderBy(item => item.Contribution.OrderingHint)
            .ThenBy(item => item.ExtensionId, StringComparer.Ordinal)
            .ThenBy(item => item.Contribution.ContributionId, StringComparer.Ordinal)
            .Select(item => new ExtensionActionRenderModel(item.ExtensionId, item.Contribution))
            .ToArray();
    }

    private ExtensionRegistrationResult ValidateManifest(ExtensionManifest manifest)
    {
        if (!ExtensionContractValidation.IsBoundedIdentifier(manifest.ExtensionId) ||
            !ExtensionContractValidation.IsBoundedText(manifest.DisplayName, 80) ||
            !manifest.SchemaVersion.IsValid)
        {
            return ExtensionRegistrationResult.Rejected(
                ExtensionRegistrationRejectionReason.InvalidManifest,
                "The extension manifest metadata is invalid.");
        }

        if (!manifest.SchemaVersion.IsCompatibleWith(_hostSchema))
        {
            return ExtensionRegistrationResult.Rejected(
                ExtensionRegistrationRejectionReason.UnsupportedSchema,
                "The extension schema version is not supported by this host.");
        }

        var declaredCapabilities = new Dictionary<string, ExtensionCapabilityDeclaration>(
            StringComparer.Ordinal);
        foreach (var capability in manifest.Capabilities)
        {
            if (capability is null ||
                !ExtensionCapabilityNames.IsKnown(capability.Name) ||
                !declaredCapabilities.TryAdd(capability.Name, capability))
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.UnknownCapability,
                    "The extension declares an unknown or duplicate capability.");
            }

            if (!_hostCapabilities.TryGetValue(capability.Name, out var hostVersion) ||
                !capability.Version.IsCompatibleWith(hostVersion))
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.CapabilityVersion,
                    "The extension capability version is not supported by this host.");
            }
        }

        var contributionIds = new HashSet<string>(StringComparer.Ordinal);
        foreach (var contribution in manifest.Contributions)
        {
            if (contribution is null ||
                !contributionIds.Add(contribution.ContributionId))
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.DuplicateContribution,
                    "Contribution IDs must be unique within an extension.");
            }

            if (contribution.OrderingHint < MinimumOrderingHint ||
                contribution.OrderingHint > MaximumOrderingHint)
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.InvalidOrdering,
                    "The contribution ordering hint is outside the host bounds.");
            }

            if (!ExtensionCapabilityNames.IsKnown(contribution.RequiredCapability))
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.UnknownCapability,
                    "The contribution requires an unknown capability.");
            }

            if (!declaredCapabilities.ContainsKey(contribution.RequiredCapability))
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.CapabilityNotDeclared,
                    "The contribution capability must be declared by the extension.");
            }

            var contributionResult = contribution switch
            {
                ExtensionThemeTokenOverride theme => ValidateTheme(theme),
                ExtensionActionContribution action => ValidateAction(action),
                ExtensionPageContribution page => ValidatePage(page),
                _ => ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.InvalidManifest,
                    "The contribution type is not supported by this host.")
            };
            if (!contributionResult.Accepted)
            {
                return contributionResult;
            }
        }

        var manifestRoutes = new HashSet<string>(StringComparer.Ordinal);
        foreach (var page in manifest.Contributions.OfType<ExtensionPageContribution>())
        {
            if (!manifestRoutes.Add(page.RouteId))
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.DuplicatePageRoute,
                    "Page route IDs must be unique in an extension.");
            }

            var duplicateRoute = _manifests.Values
                .Where(registered => !string.Equals(
                    registered.Manifest.ExtensionId,
                    manifest.ExtensionId,
                    StringComparison.Ordinal))
                .SelectMany(static registered => registered.Manifest.Contributions
                    .OfType<ExtensionPageContribution>())
                .Any(existing => string.Equals(
                    existing.RouteId,
                    page.RouteId,
                    StringComparison.Ordinal));
            if (duplicateRoute)
            {
                return ExtensionRegistrationResult.Rejected(
                    ExtensionRegistrationRejectionReason.DuplicatePageRoute,
                    "Page route IDs must be unique in the host registry.");
            }
        }

        return ExtensionRegistrationResult.Registered;
    }

    private static ExtensionRegistrationResult ValidateTheme(
        ExtensionThemeTokenOverride theme)
    {
        if (!ExtensionContractValidation.IsValidTokenId(theme.TokenId) ||
            !ExtensionContractValidation.IsValidThemeValue(theme.Value))
        {
            return ExtensionRegistrationResult.Rejected(
                ExtensionRegistrationRejectionReason.InvalidThemeToken,
                "The theme token override is invalid.");
        }

        return ExtensionRegistrationResult.Registered;
    }

    private static ExtensionRegistrationResult ValidateAction(
        ExtensionActionContribution action)
    {
        if (!ExtensionActionSlots.IsKnown(action.SlotId) ||
            !ExtensionContractValidation.IsBoundedIdentifier(action.ContributionId) ||
            !ExtensionContractValidation.IsBoundedText(action.Label, 128) ||
            !ExtensionContractValidation.IsValidIcon(action.Icon) ||
            !ExtensionContractValidation.IsBoundedNamespace(action.CommandNamespace) ||
            !string.Equals(
                action.RequiredCapability,
                ExtensionCapabilityNames.ActionSlots,
                StringComparison.Ordinal) ||
            !action.Visibility.IsValid ||
            (action.AccessibilityText is not null &&
                !ExtensionContractValidation.IsBoundedText(action.AccessibilityText, 256)))
        {
            return ExtensionRegistrationResult.Rejected(
                ExtensionRegistrationRejectionReason.InvalidAction,
                "The action contribution is invalid.");
        }

        return ExtensionRegistrationResult.Registered;
    }

    private ExtensionRegistrationResult ValidatePage(
        ExtensionPageContribution page)
    {
        if (!ExtensionContractValidation.IsBoundedIdentifier(page.ContributionId) ||
            !ExtensionContractValidation.IsBoundedIdentifier(page.RouteId, allowColon: true) ||
            !ExtensionActionSlots.IsKnown(page.ParentSlotId) ||
            !ExtensionContractValidation.IsBoundedText(page.Title, 128) ||
            !ExtensionContractValidation.IsValidIcon(page.Icon) ||
            !string.Equals(
                page.RequiredCapability,
                ExtensionCapabilityNames.PlaceholderPages,
                StringComparison.Ordinal) ||
            !page.MinimumHostSchema.IsCompatibleWith(_hostSchema) ||
            !Enum.IsDefined(page.LifecycleScope) ||
            !Enum.IsDefined(page.BackBehavior) ||
            !Enum.IsDefined(page.DeepLinkBehavior) ||
            !page.ParameterSchema.IsValid(out _))
        {
            return ExtensionRegistrationResult.Rejected(
                ExtensionRegistrationRejectionReason.InvalidPage,
                "The page contribution is invalid.");
        }

        return ExtensionRegistrationResult.Registered;
    }

    private static Dictionary<string, ExtensionSchemaVersion> CreateHostCapabilities(
        IEnumerable<ExtensionCapabilityDeclaration>? capabilities)
    {
        var source = capabilities ?? new[]
        {
            new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.ThemeTokenOverrides),
            new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.ActionSlots),
            new ExtensionCapabilityDeclaration(ExtensionCapabilityNames.PlaceholderPages)
        };
        var result = new Dictionary<string, ExtensionSchemaVersion>(StringComparer.Ordinal);
        foreach (var capability in source)
        {
            if (capability is null ||
                !ExtensionCapabilityNames.IsKnown(capability.Name) ||
                !capability.Version.IsValid ||
                !result.TryAdd(capability.Name, capability.Version))
            {
                throw new ArgumentException(
                    "Host capabilities must be known, valid, and unique.",
                    nameof(capabilities));
            }
        }

        return result;
    }

    private sealed class RegisteredManifest
    {
        public RegisteredManifest(ExtensionManifest manifest)
        {
            Manifest = manifest;
            IsEnabled = manifest.IsEnabled;
        }

        public ExtensionManifest Manifest { get; }

        public bool IsEnabled { get; set; }
    }
}
