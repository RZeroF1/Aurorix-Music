using System.Collections.ObjectModel;
using System.Globalization;
using System.Text.RegularExpressions;

namespace Aurorix.Platform.Windows.Extensions;

/// <summary>
/// Version of the declarative contribution schema understood by the host.
/// A host accepts the same major and an extension minor no newer than its own.
/// </summary>
public readonly record struct ExtensionSchemaVersion(int Major, int Minor)
{
    public static ExtensionSchemaVersion Current => new(1, 0);

    public bool IsValid => Major > 0 && Minor >= 0;

    public bool IsCompatibleWith(ExtensionSchemaVersion host) =>
        IsValid && host.IsValid && Major == host.Major && Minor <= host.Minor;

    public override string ToString() => $"{Major}.{Minor}";
}

/// <summary>
/// Stable capability names exposed by this host. Capabilities are declarations,
/// not handles; no database, Core, credential, process, or realtime-audio
/// object is represented by this contract.
/// </summary>
public static class ExtensionCapabilityNames
{
    public const string ThemeTokenOverrides = "theme.tokens";
    public const string ActionSlots = "actions.invoke";
    public const string PlaceholderPages = "pages.placeholder";

    private static readonly IReadOnlySet<string> Known = new HashSet<string>(
        new[] { ThemeTokenOverrides, ActionSlots, PlaceholderPages },
        StringComparer.Ordinal);

    public static bool IsKnown(string? name) =>
        name is not null && Known.Contains(name);
}

/// <summary>
/// A capability requested by an extension manifest.
/// </summary>
public sealed record ExtensionCapabilityDeclaration
{
    public ExtensionCapabilityDeclaration(
        string name,
        ExtensionSchemaVersion? version = null)
    {
        Name = name ?? string.Empty;
        Version = version ?? ExtensionSchemaVersion.Current;
    }

    public string Name { get; }

    public ExtensionSchemaVersion Version { get; }
}

/// <summary>
/// The reserved host-rendered slots. Slot IDs are data and are never used as
/// XAML names or visual-tree paths.
/// </summary>
public static class ExtensionActionSlots
{
    public const string NavigationPrimary = "navigation.primary";
    public const string HomeQuickEntries = "home.quick_entries";
    public const string HomeContentSections = "home.content_sections";
    public const string PlayerTransport = "player.transport";
    public const string PlayerQueue = "player.queue";
    public const string PlayerOutput = "player.output";
    public const string PlayerLyrics = "player.lyrics";
    public const string PlayerCustomActions = "player.custom_actions";
    public const string PlayerSecondaryPanel = "player.secondary_panel";
    public const string LibraryToolbar = "library.toolbar";
    public const string DetailActions = "detail.actions";
    public const string SettingsSections = "settings.sections";

    private static readonly IReadOnlySet<string> Known = new HashSet<string>(
        new[]
        {
            NavigationPrimary,
            HomeQuickEntries,
            HomeContentSections,
            PlayerTransport,
            PlayerQueue,
            PlayerOutput,
            PlayerLyrics,
            PlayerCustomActions,
            PlayerSecondaryPanel,
            LibraryToolbar,
            DetailActions,
            SettingsSections
        },
        StringComparer.Ordinal);

    public static IReadOnlyList<string> All { get; } = Array.AsReadOnly(new[]
    {
        NavigationPrimary,
        HomeQuickEntries,
        HomeContentSections,
        PlayerTransport,
        PlayerQueue,
        PlayerOutput,
        PlayerLyrics,
        PlayerCustomActions,
        PlayerSecondaryPanel,
        LibraryToolbar,
        DetailActions,
        SettingsSections
    });

    public static bool IsKnown(string? slotId) =>
        slotId is not null && Known.Contains(slotId);
}

public enum ExtensionIconKind
{
    NamedSymbol,
    Glyph,
}

/// <summary>
/// Bounded icon metadata for a host-rendered control. It is not a URI, XAML
/// resource reference, or executable asset locator.
/// </summary>
public sealed record ExtensionIcon
{
    public ExtensionIcon(ExtensionIconKind kind, string value)
    {
        Kind = kind;
        Value = value ?? string.Empty;
    }

    public ExtensionIconKind Kind { get; }

    public string Value { get; }

    public static ExtensionIcon Named(string value) =>
        new(ExtensionIconKind.NamedSymbol, value);

    public static ExtensionIcon Glyph(string value) =>
        new(ExtensionIconKind.Glyph, value);
}

public enum ExtensionVisibilityPredicateKind
{
    Always,
    RouteEquals,
    HostCapability,
}

/// <summary>
/// Declarative visibility only. Delegate/code predicates are intentionally not
/// part of the extension boundary.
/// </summary>
public sealed record ExtensionVisibilityPredicate
{
    public ExtensionVisibilityPredicate(
        ExtensionVisibilityPredicateKind kind,
        string? value = null)
    {
        Kind = kind;
        Value = string.IsNullOrWhiteSpace(value) ? null : value.Trim();
    }

    public ExtensionVisibilityPredicateKind Kind { get; }

    public string? Value { get; }

    public static ExtensionVisibilityPredicate Always { get; } =
        new(ExtensionVisibilityPredicateKind.Always);

    internal bool IsValid => Kind switch
    {
        ExtensionVisibilityPredicateKind.Always => Value is null,
        ExtensionVisibilityPredicateKind.RouteEquals =>
            ExtensionContractValidation.IsBoundedIdentifier(Value, allowColon: true),
        ExtensionVisibilityPredicateKind.HostCapability =>
            ExtensionCapabilityNames.IsKnown(Value),
        _ => false
    };

    internal bool Matches(ExtensionHostContext context) => Kind switch
    {
        ExtensionVisibilityPredicateKind.Always => true,
        ExtensionVisibilityPredicateKind.RouteEquals =>
            string.Equals(context.RouteId, Value, StringComparison.Ordinal),
        ExtensionVisibilityPredicateKind.HostCapability =>
            Value is not null && context.AvailableCapabilities.Contains(Value),
        _ => false
    };
}

public enum ExtensionThemeTokenValueKind
{
    Color,
    Number,
    Boolean,
    Text,
    Material,
}

/// <summary>
/// A bounded, declarative theme value. Material values require an explicit
/// fallback so accessibility and platform capability can fail closed.
/// </summary>
public sealed record ExtensionThemeTokenValue
{
    public ExtensionThemeTokenValue(
        ExtensionThemeTokenValueKind kind,
        string value,
        string? fallbackValue = null)
    {
        Kind = kind;
        Value = value ?? string.Empty;
        FallbackValue = fallbackValue;
    }

    public ExtensionThemeTokenValueKind Kind { get; }

    public string Value { get; }

    public string? FallbackValue { get; }

    public static ExtensionThemeTokenValue Color(string value) =>
        new(ExtensionThemeTokenValueKind.Color, value);

    public static ExtensionThemeTokenValue Number(string value) =>
        new(ExtensionThemeTokenValueKind.Number, value);

    public static ExtensionThemeTokenValue Boolean(bool value) =>
        new(ExtensionThemeTokenValueKind.Boolean, value ? "true" : "false");

    public static ExtensionThemeTokenValue Text(string value) =>
        new(ExtensionThemeTokenValueKind.Text, value);

    public static ExtensionThemeTokenValue Material(
        string value,
        string fallbackValue) =>
        new(ExtensionThemeTokenValueKind.Material, value, fallbackValue);
}

/// <summary>
/// Semantic token groups that an extension may override. The host owns the
/// token vocabulary; an extension cannot introduce a resource dictionary key.
/// </summary>
public static class ExtensionThemeTokenGroups
{
    public const string Color = "color";
    public const string Typography = "typography";
    public const string Spacing = "spacing";
    public const string Shape = "shape";
    public const string Elevation = "elevation";
    public const string Material = "material";
    public const string Opacity = "opacity";
    public const string Blur = "blur";
    public const string Accent = "accent";
    public const string Motion = "motion";
    public const string Focus = "focus";
    public const string Accessibility = "accessibility";

    private static readonly IReadOnlySet<string> Known = new HashSet<string>(
        new[]
        {
            Color,
            Typography,
            Spacing,
            Shape,
            Elevation,
            Material,
            Opacity,
            Blur,
            Accent,
            Motion,
            Focus,
            Accessibility
        },
        StringComparer.Ordinal);

    public static bool IsKnown(string? group) =>
        group is not null && Known.Contains(group);
}

public sealed record ExtensionThemeTokenOverride : IExtensionContribution
{
    public ExtensionThemeTokenOverride(
        string tokenId,
        ExtensionThemeTokenValue value,
        int orderingHint = 0)
    {
        TokenId = tokenId ?? string.Empty;
        Value = value ?? new ExtensionThemeTokenValue(
            ExtensionThemeTokenValueKind.Text,
            string.Empty);
        OrderingHint = orderingHint;
    }

    public string TokenId { get; }

    public ExtensionThemeTokenValue Value { get; }

    public string ContributionId => $"theme:{TokenId}";

    public int OrderingHint { get; }

    public string RequiredCapability => ExtensionCapabilityNames.ThemeTokenOverrides;
}

public enum ExtensionPageParameterType
{
    String,
    Integer,
    Boolean,
}

public sealed record ExtensionPageParameter
{
    public ExtensionPageParameter(
        string name,
        ExtensionPageParameterType type,
        bool isRequired = false)
    {
        Name = name ?? string.Empty;
        Type = type;
        IsRequired = isRequired;
    }

    public string Name { get; }

    public ExtensionPageParameterType Type { get; }

    public bool IsRequired { get; }
}

/// <summary>
/// A small parameter schema for a page deep link. Values remain text at this
/// host boundary and are validated against the declared primitive type.
/// </summary>
public sealed record ExtensionPageParameterSchema
{
    public ExtensionPageParameterSchema(
        IEnumerable<ExtensionPageParameter>? parameters = null)
    {
        Parameters = new ReadOnlyCollection<ExtensionPageParameter>(
            (parameters ?? Array.Empty<ExtensionPageParameter>()).ToArray());
    }

    public IReadOnlyList<ExtensionPageParameter> Parameters { get; }

    public static ExtensionPageParameterSchema Empty { get; } = new();

    internal bool IsValid(out string error)
    {
        if (Parameters.Count > 16)
        {
            error = "The page parameter schema is too large.";
            return false;
        }

        var names = new HashSet<string>(StringComparer.Ordinal);
        foreach (var parameter in Parameters)
        {
            if (parameter is null ||
                !ExtensionContractValidation.IsBoundedIdentifier(parameter.Name) ||
                !Enum.IsDefined(parameter.Type) ||
                !names.Add(parameter.Name))
            {
                error = "The page parameter schema is invalid.";
                return false;
            }
        }

        error = string.Empty;
        return true;
    }

    internal bool TryValidateValues(
        IReadOnlyDictionary<string, string> values,
        out string error)
    {
        if (!IsValid(out error))
        {
            return false;
        }

        var declared = new HashSet<string>(
            Parameters.Select(static parameter => parameter.Name),
            StringComparer.Ordinal);

        foreach (var pair in values)
        {
            if (!declared.Contains(pair.Key) ||
                pair.Key.Length > 64 ||
                pair.Value is null ||
                pair.Value.Length > 512 ||
                pair.Value.Any(char.IsControl))
            {
                error = "Page parameters contain an undeclared or invalid value.";
                return false;
            }

            var parameter = Parameters.First(item =>
                string.Equals(item.Name, pair.Key, StringComparison.Ordinal));
            if (!IsValueOfType(pair.Value, parameter.Type))
            {
                error = $"Page parameter '{pair.Key}' has the wrong type.";
                return false;
            }
        }

        foreach (var parameter in Parameters.Where(static item => item.IsRequired))
        {
            if (!values.ContainsKey(parameter.Name))
            {
                error = $"Required page parameter '{parameter.Name}' is missing.";
                return false;
            }
        }

        error = string.Empty;
        return true;
    }

    private static bool IsValueOfType(
        string value,
        ExtensionPageParameterType type) => type switch
        {
            ExtensionPageParameterType.String => value.Length > 0,
            ExtensionPageParameterType.Integer =>
                int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out _),
            ExtensionPageParameterType.Boolean =>
                bool.TryParse(value, out _),
            _ => false
        };
}

public enum ExtensionPageLifecycleScope
{
    Navigation,
    Session,
}

public enum ExtensionPageBackBehavior
{
    HostNavigation,
    ReturnToParent,
}

public enum ExtensionPageDeepLinkBehavior
{
    HostRouteOnly,
    Disabled,
}

public interface IExtensionContribution
{
    string ContributionId { get; }

    int OrderingHint { get; }

    string RequiredCapability { get; }
}

public sealed record ExtensionActionContribution : IExtensionContribution
{
    public ExtensionActionContribution(
        string contributionId,
        string slotId,
        string label,
        ExtensionIcon icon,
        string commandNamespace,
        string requiredCapability = ExtensionCapabilityNames.ActionSlots,
        int orderingHint = 0,
        ExtensionVisibilityPredicate? visibility = null,
        string? accessibilityText = null)
    {
        ContributionId = contributionId ?? string.Empty;
        SlotId = slotId ?? string.Empty;
        Label = label ?? string.Empty;
        Icon = icon ?? new ExtensionIcon(ExtensionIconKind.NamedSymbol, string.Empty);
        CommandNamespace = commandNamespace ?? string.Empty;
        RequiredCapability = requiredCapability ?? string.Empty;
        OrderingHint = orderingHint;
        Visibility = visibility ?? ExtensionVisibilityPredicate.Always;
        AccessibilityText = accessibilityText;
    }

    public ExtensionActionContribution(
        string contributionId,
        string slotId,
        string label,
        string iconValue,
        string commandNamespace,
        string requiredCapability = ExtensionCapabilityNames.ActionSlots,
        int orderingHint = 0,
        ExtensionVisibilityPredicate? visibility = null,
        string? accessibilityText = null)
        : this(
            contributionId,
            slotId,
            label,
            ExtensionIcon.Named(iconValue),
            commandNamespace,
            requiredCapability,
            orderingHint,
            visibility,
            accessibilityText)
    {
    }

    public string ContributionId { get; }

    public string SlotId { get; }

    public string Label { get; }

    public ExtensionIcon Icon { get; }

    public string CommandNamespace { get; }

    public string RequiredCapability { get; }

    public int OrderingHint { get; }

    public ExtensionVisibilityPredicate Visibility { get; }

    public string? AccessibilityText { get; }
}

public sealed record ExtensionPageContribution : IExtensionContribution
{
    public ExtensionPageContribution(
        string contributionId,
        string routeId,
        string parentSlotId,
        string title,
        ExtensionIcon icon,
        ExtensionPageParameterSchema? parameterSchema = null,
        ExtensionPageLifecycleScope lifecycleScope = ExtensionPageLifecycleScope.Navigation,
        ExtensionSchemaVersion? minimumHostSchema = null,
        ExtensionPageBackBehavior backBehavior = ExtensionPageBackBehavior.HostNavigation,
        ExtensionPageDeepLinkBehavior deepLinkBehavior = ExtensionPageDeepLinkBehavior.HostRouteOnly,
        string requiredCapability = ExtensionCapabilityNames.PlaceholderPages,
        int orderingHint = 0)
    {
        ContributionId = contributionId ?? string.Empty;
        RouteId = routeId ?? string.Empty;
        ParentSlotId = parentSlotId ?? string.Empty;
        Title = title ?? string.Empty;
        Icon = icon ?? new ExtensionIcon(ExtensionIconKind.NamedSymbol, string.Empty);
        ParameterSchema = parameterSchema ?? ExtensionPageParameterSchema.Empty;
        LifecycleScope = lifecycleScope;
        MinimumHostSchema = minimumHostSchema ?? ExtensionSchemaVersion.Current;
        BackBehavior = backBehavior;
        DeepLinkBehavior = deepLinkBehavior;
        RequiredCapability = requiredCapability ?? string.Empty;
        OrderingHint = orderingHint;
    }

    public ExtensionPageContribution(
        string contributionId,
        string routeId,
        string parentSlotId,
        string title,
        string iconValue,
        ExtensionPageParameterSchema? parameterSchema = null,
        ExtensionPageLifecycleScope lifecycleScope = ExtensionPageLifecycleScope.Navigation,
        ExtensionSchemaVersion? minimumHostSchema = null,
        ExtensionPageBackBehavior backBehavior = ExtensionPageBackBehavior.HostNavigation,
        ExtensionPageDeepLinkBehavior deepLinkBehavior = ExtensionPageDeepLinkBehavior.HostRouteOnly,
        string requiredCapability = ExtensionCapabilityNames.PlaceholderPages,
        int orderingHint = 0)
        : this(
            contributionId,
            routeId,
            parentSlotId,
            title,
            ExtensionIcon.Named(iconValue),
            parameterSchema,
            lifecycleScope,
            minimumHostSchema,
            backBehavior,
            deepLinkBehavior,
            requiredCapability,
            orderingHint)
    {
    }

    public string ContributionId { get; }

    public string RouteId { get; }

    public string ParentSlotId { get; }

    public string Title { get; }

    public ExtensionIcon Icon { get; }

    public ExtensionPageParameterSchema ParameterSchema { get; }

    public ExtensionPageLifecycleScope LifecycleScope { get; }

    public ExtensionSchemaVersion MinimumHostSchema { get; }

    public ExtensionPageBackBehavior BackBehavior { get; }

    public ExtensionPageDeepLinkBehavior DeepLinkBehavior { get; }

    public string RequiredCapability { get; }

    public int OrderingHint { get; }
}

/// <summary>
/// Manifest data accepted by the registry. It contains no activation target;
/// contributions are rendered by the host from this declaration only.
/// </summary>
public sealed record ExtensionManifest
{
    public ExtensionManifest(
        string extensionId,
        string displayName,
        ExtensionSchemaVersion schemaVersion,
        IEnumerable<ExtensionCapabilityDeclaration> capabilities,
        IEnumerable<IExtensionContribution> contributions,
        bool isEnabled = true)
    {
        ExtensionId = extensionId ?? string.Empty;
        DisplayName = displayName ?? string.Empty;
        SchemaVersion = schemaVersion;
        Capabilities = new ReadOnlyCollection<ExtensionCapabilityDeclaration>(
            (capabilities ?? Array.Empty<ExtensionCapabilityDeclaration>()).ToArray());
        Contributions = new ReadOnlyCollection<IExtensionContribution>(
            (contributions ?? Array.Empty<IExtensionContribution>()).ToArray());
        IsEnabled = isEnabled;
    }

    public string ExtensionId { get; }

    public string DisplayName { get; }

    public ExtensionSchemaVersion SchemaVersion { get; }

    public IReadOnlyList<ExtensionCapabilityDeclaration> Capabilities { get; }

    public IReadOnlyList<IExtensionContribution> Contributions { get; }

    public bool IsEnabled { get; }
}

public sealed record ExtensionHostContext
{
    public ExtensionHostContext(
        string? routeId = null,
        IEnumerable<string>? availableCapabilities = null)
    {
        RouteId = string.IsNullOrWhiteSpace(routeId) ? null : routeId.Trim();
        AvailableCapabilities = new HashSet<string>(
            (availableCapabilities ?? Array.Empty<string>())
                .Where(static value => !string.IsNullOrWhiteSpace(value))
                .Select(static value => value.Trim()),
            StringComparer.Ordinal);
    }

    public string? RouteId { get; }

    public IReadOnlySet<string> AvailableCapabilities { get; }

    public static ExtensionHostContext Empty { get; } = new();
}

/// <summary>
/// Host-rendered action projection. The host owns the actual button/menu
/// control and routes invocation through a bounded facade.
/// </summary>
public sealed record ExtensionActionRenderModel
{
    public ExtensionActionRenderModel(
        string extensionId,
        ExtensionActionContribution contribution)
    {
        ExtensionId = extensionId;
        ContributionId = contribution.ContributionId;
        SlotId = contribution.SlotId;
        Label = contribution.Label;
        Icon = contribution.Icon;
        CommandNamespace = contribution.CommandNamespace;
        AccessibilityText = contribution.AccessibilityText ?? contribution.Label;
        OrderingHint = contribution.OrderingHint;
    }

    public string ExtensionId { get; }

    public string ContributionId { get; }

    public string SlotId { get; }

    public string Label { get; }

    public ExtensionIcon Icon { get; }

    public string CommandNamespace { get; }

    public string AccessibilityText { get; }

    public int OrderingHint { get; }
}

public sealed record ExtensionActionInvocation(
    ulong RequestId,
    string ExtensionId,
    string ContributionId,
    string CommandNamespace);

public enum ExtensionActionDispatchStatus
{
    Dispatched,
    Disabled,
    Unknown,
    Unavailable,
    Failed,
    Canceled,
}

public sealed record ExtensionActionDispatchResult(
    ExtensionActionDispatchStatus Status,
    ExtensionActionInvocation Invocation);

public interface IExtensionActionRouter
{
    ValueTask<ExtensionActionDispatchResult> DispatchAsync(
        ExtensionActionInvocation invocation,
        CancellationToken cancellationToken = default);
}

public enum ExtensionRegistrationRejectionReason
{
    None,
    NullManifest,
    InvalidManifest,
    UnsupportedSchema,
    UnknownCapability,
    CapabilityVersion,
    CapabilityNotDeclared,
    InvalidOrdering,
    DuplicateContribution,
    DuplicatePageRoute,
    InvalidThemeToken,
    InvalidAction,
    InvalidPage,
}

public sealed record ExtensionRegistrationResult(
    bool Accepted,
    ExtensionRegistrationRejectionReason RejectionReason,
    string Message)
{
    public static ExtensionRegistrationResult Registered { get; } =
        new(true, ExtensionRegistrationRejectionReason.None, "Registered.");

    public static ExtensionRegistrationResult Rejected(
        ExtensionRegistrationRejectionReason reason,
        string message) => new(false, reason, message);
}

public sealed record ExtensionRegistryEntry(
    string ExtensionId,
    string DisplayName,
    ExtensionSchemaVersion SchemaVersion,
    bool IsEnabled,
    int ActionCount,
    int PageCount,
    int ThemeOverrideCount);

internal static class ExtensionContractValidation
{
    private const int MaxIdentifierLength = 128;
    private static readonly Regex Identifier = new(
        "^[a-z0-9][a-z0-9._-]{0,127}$",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);
    private static readonly Regex RouteIdentifier = new(
        "^[a-z0-9][a-z0-9._:-]{0,127}$",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);
    private static readonly Regex TokenIdentifier = new(
        "^[a-z][a-z0-9]*(\\.[a-z][a-z0-9]*){1,3}$",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);
    private static readonly Regex NamespaceIdentifier = new(
        "^[a-z0-9]+(?:[._-][a-z0-9]+)+$",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);

    public static bool IsBoundedIdentifier(
        string? value,
        bool allowColon = false)
    {
        if (string.IsNullOrWhiteSpace(value) || value.Length > MaxIdentifierLength)
        {
            return false;
        }

        return (allowColon ? RouteIdentifier : Identifier).IsMatch(value);
    }

    public static bool IsValidTokenId(string? value)
    {
        if (value is null || value.Length > MaxIdentifierLength || !TokenIdentifier.IsMatch(value))
        {
            return false;
        }

        var group = value[..value.IndexOf('.', StringComparison.Ordinal)];
        return ExtensionThemeTokenGroups.IsKnown(group);
    }

    public static bool IsBoundedText(
        string? value,
        int maximumLength,
        bool allowEmpty = false)
    {
        if (value is null || value.Length > maximumLength ||
            (!allowEmpty && string.IsNullOrWhiteSpace(value)))
        {
            return false;
        }

        return !value.Any(char.IsControl);
    }

    public static bool IsValidIcon(ExtensionIcon? icon)
    {
        if (icon is null || !Enum.IsDefined(icon.Kind) ||
            !IsBoundedText(icon.Value, 128))
        {
            return false;
        }

        return !icon.Value.Contains('/', StringComparison.Ordinal) &&
            !icon.Value.Contains('\\', StringComparison.Ordinal) &&
            !icon.Value.Contains(':', StringComparison.Ordinal);
    }

    public static bool IsBoundedNamespace(string? value) =>
        value is not null &&
        value.Length <= MaxIdentifierLength &&
        NamespaceIdentifier.IsMatch(value);

    public static bool IsValidThemeValue(ExtensionThemeTokenValue? value)
    {
        if (value is null || !Enum.IsDefined(value.Kind) ||
            !IsBoundedText(value.Value, 256))
        {
            return false;
        }

        return value.Kind switch
        {
            ExtensionThemeTokenValueKind.Color => IsColor(value.Value),
            ExtensionThemeTokenValueKind.Number => IsNumber(value.Value),
            ExtensionThemeTokenValueKind.Boolean =>
                bool.TryParse(value.Value, out _),
            ExtensionThemeTokenValueKind.Text =>
                value.FallbackValue is null || IsBoundedText(value.FallbackValue, 256),
            ExtensionThemeTokenValueKind.Material =>
                IsMaterial(value.Value) && IsMaterial(value.FallbackValue),
            _ => false
        };
    }

    private static bool IsColor(string value)
    {
        if ((value.Length != 7 && value.Length != 9) || value[0] != '#')
        {
            return false;
        }

        return value.Skip(1).All(Uri.IsHexDigit);
    }

    private static bool IsNumber(string value) =>
        decimal.TryParse(
            value,
            NumberStyles.Float,
            CultureInfo.InvariantCulture,
            out var parsed) &&
        parsed >= -10000m && parsed <= 10000m;

    private static bool IsMaterial(string? value) =>
        value is "solid"
            or "mica"
            or "mica-alt"
            or "acrylic"
            or "transparent-acrylic"
            or "custom-acrylic"
            or "none"
            or "glass";
}
