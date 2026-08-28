using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Aurorix.Windows.Themes;

/// <summary>
/// Stable semantic resource names shared by the host and XAML. A token is
/// intentionally named for its meaning rather than for a particular control
/// or material implementation.
/// </summary>
public static class ThemeTokenKeys
{
    public const string CanvasColor = "Aurorix.Theme.Color.Canvas";
    public const string SurfaceColor = "Aurorix.Theme.Color.Surface";
    public const string SurfaceSubtleColor = "Aurorix.Theme.Color.SurfaceSubtle";
    public const string InkColor = "Aurorix.Theme.Color.Ink";
    public const string MutedInkColor = "Aurorix.Theme.Color.MutedInk";
    public const string BorderColor = "Aurorix.Theme.Color.Border";
    public const string AccentColor = "Aurorix.Theme.Color.Accent";
    public const string OnAccentColor = "Aurorix.Theme.Color.OnAccent";
    public const string FocusColor = "Aurorix.Theme.Color.Focus";
    public const string SelectionColor = "Aurorix.Theme.Color.Selection";
    public const string DisabledColor = "Aurorix.Theme.Color.Disabled";

    public const string FontFamily = "Aurorix.Theme.Typography.FontFamily";
    public const string BodyFontSize = "Aurorix.Theme.Typography.BodySize";
    public const string CaptionFontSize = "Aurorix.Theme.Typography.CaptionSize";
    public const string HeadingFontSize = "Aurorix.Theme.Typography.HeadingSize";
    public const string TitleFontSize = "Aurorix.Theme.Typography.TitleSize";
    public const string BodyFontWeight = "Aurorix.Theme.Typography.BodyWeight";

    public const string SpacingXs = "Aurorix.Theme.Spacing.Xs";
    public const string SpacingSm = "Aurorix.Theme.Spacing.Sm";
    public const string SpacingMd = "Aurorix.Theme.Spacing.Md";
    public const string SpacingLg = "Aurorix.Theme.Spacing.Lg";
    public const string SpacingXl = "Aurorix.Theme.Spacing.Xl";

    public const string ControlCornerRadius = "Aurorix.Theme.Shape.ControlCornerRadius";
    public const string CardCornerRadius = "Aurorix.Theme.Shape.CardCornerRadius";
    public const string PanelCornerRadius = "Aurorix.Theme.Shape.PanelCornerRadius";

    public const string PanelElevation = "Aurorix.Theme.Elevation.Panel";
    public const string SurfaceOpacity = "Aurorix.Theme.Opacity.Surface";
    public const string BorderOpacity = "Aurorix.Theme.Opacity.Border";
    public const string DisabledOpacity = "Aurorix.Theme.Opacity.Disabled";
    public const string SurfaceBlur = "Aurorix.Theme.Blur.Surface";

    public const string MaterialSurface = "Aurorix.Theme.Material.Surface";
    public const string MotionEnabled = "Aurorix.Theme.Motion.Enabled";
    public const string MotionFastDuration = "Aurorix.Theme.Motion.FastDurationMs";
    public const string MotionNormalDuration = "Aurorix.Theme.Motion.NormalDurationMs";
    public const string MotionSlowDuration = "Aurorix.Theme.Motion.SlowDurationMs";
    public const string FocusRingThickness = "Aurorix.Theme.Focus.RingThickness";
    public const string AccessibilityTransparencyAllowed = "Aurorix.Theme.Accessibility.TransparencyAllowed";
    public const string AccessibilityFocusVisible = "Aurorix.Theme.Accessibility.FocusVisible";

    public static readonly IReadOnlyList<string> All = new ReadOnlyCollection<string>(
    [
        CanvasColor,
        SurfaceColor,
        SurfaceSubtleColor,
        InkColor,
        MutedInkColor,
        BorderColor,
        AccentColor,
        OnAccentColor,
        FocusColor,
        SelectionColor,
        DisabledColor,
        FontFamily,
        BodyFontSize,
        CaptionFontSize,
        HeadingFontSize,
        TitleFontSize,
        BodyFontWeight,
        SpacingXs,
        SpacingSm,
        SpacingMd,
        SpacingLg,
        SpacingXl,
        ControlCornerRadius,
        CardCornerRadius,
        PanelCornerRadius,
        PanelElevation,
        SurfaceOpacity,
        BorderOpacity,
        DisabledOpacity,
        SurfaceBlur,
        MaterialSurface,
        MotionEnabled,
        MotionFastDuration,
        MotionNormalDuration,
        MotionSlowDuration,
        FocusRingThickness,
        AccessibilityTransparencyAllowed,
        AccessibilityFocusVisible,
    ]);

    public static string BrushForColor(string colorKey)
    {
        ArgumentNullException.ThrowIfNull(colorKey);

        const string colorPrefix = "Aurorix.Theme.Color.";
        return colorKey.StartsWith(colorPrefix, StringComparison.Ordinal)
            ? "Aurorix.Theme.Brush." + colorKey[colorPrefix.Length..]
            : colorKey;
    }
}

public enum ThemeTokenValueKind
{
    Color,
    Number,
    Boolean,
    Material,
    String,
}

public enum ThemeMaterial
{
    Solid,
    Mica,
    Acrylic,
    Glass,
}

public enum ThemeSystemVariant
{
    Light,
    Dark,
}

public enum ThemeRegistryEntryKind
{
    BuiltIn,
    Extension,
}

public enum ThemeTokenIssueKind
{
    UnknownToken,
    WrongType,
    InvalidValue,
    ClampedValue,
}

public enum ThemeTokenDiagnosticKind
{
    InvalidToken,
    ClampedToken,
    UnknownTheme,
    RegistryRejected,
    MaterialFallback,
    AccessibilityFallback,
    PowerFallback,
    ReducedMotionFallback,
}

/// <summary>
/// Declarative token value. It contains no XAML, code, URI, package path, or
/// executable payload, which keeps extension input bounded to the host schema.
/// </summary>
public sealed record ThemeTokenValue
{
    [JsonConstructor]
    public ThemeTokenValue(string key, ThemeTokenValueKind kind, string value)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(key);
        ArgumentNullException.ThrowIfNull(value);
        Key = key.Trim();
        Kind = kind;
        Value = value.Trim();
    }

    public string Key { get; }

    public ThemeTokenValueKind Kind { get; }

    public string Value { get; }

    public static ThemeTokenValue Color(string key, string value) =>
        new(key, ThemeTokenValueKind.Color, value);

    public static ThemeTokenValue Number(string key, double value) =>
        new(key, ThemeTokenValueKind.Number, value.ToString("R", CultureInfo.InvariantCulture));

    public static ThemeTokenValue Boolean(string key, bool value) =>
        new(key, ThemeTokenValueKind.Boolean, value ? "true" : "false");

    public static ThemeTokenValue Material(string key, ThemeMaterial value) =>
        new(key, ThemeTokenValueKind.Material, value.ToString().ToLowerInvariant());

    public static ThemeTokenValue String(string key, string value) =>
        new(key, ThemeTokenValueKind.String, value);
}

public sealed record ThemeTokenIssue(
    string Key,
    ThemeTokenIssueKind Kind,
    string Message,
    ThemeTokenValue? NormalizedValue = null);

public sealed record ThemeTokenDiagnostic(
    string Layer,
    string Key,
    ThemeTokenDiagnosticKind Kind,
    string Message);

public sealed record ThemeEnvironment
{
    public ThemeEnvironment(
        ThemeSystemVariant systemVariant = ThemeSystemVariant.Light,
        bool highContrast = false,
        bool reducedMotion = false,
        bool powerSaver = false,
        bool supportsMica = true,
        bool supportsAcrylic = true,
        bool supportsGlass = false)
    {
        SystemVariant = systemVariant;
        HighContrast = highContrast;
        ReducedMotion = reducedMotion;
        PowerSaver = powerSaver;
        SupportsMica = supportsMica;
        SupportsAcrylic = supportsAcrylic;
        SupportsGlass = supportsGlass;
    }

    public ThemeSystemVariant SystemVariant { get; }

    public bool HighContrast { get; }

    public bool ReducedMotion { get; }

    public bool PowerSaver { get; }

    public bool SupportsMica { get; }

    public bool SupportsAcrylic { get; }

    public bool SupportsGlass { get; }
}

public sealed record ThemeFallbackState(
    bool MaterialFallbackApplied,
    bool AccessibilityFallbackApplied,
    bool PowerFallbackApplied,
    bool ReducedMotionFallbackApplied,
    ThemeMaterial EffectiveMaterial);

public sealed record ThemeResolutionResult
{
    public ThemeResolutionResult(
        IReadOnlyDictionary<string, ThemeTokenValue> tokens,
        IReadOnlyList<ThemeTokenDiagnostic> diagnostics,
        ThemeFallbackState fallbackState,
        string activeThemeId,
        bool registryAccepted)
    {
        Tokens = new ReadOnlyDictionary<string, ThemeTokenValue>(
            new Dictionary<string, ThemeTokenValue>(tokens, StringComparer.Ordinal));
        Diagnostics = new ReadOnlyCollection<ThemeTokenDiagnostic>(diagnostics.ToArray());
        FallbackState = fallbackState;
        ActiveThemeId = activeThemeId;
        RegistryAccepted = registryAccepted;
    }

    public IReadOnlyDictionary<string, ThemeTokenValue> Tokens { get; }

    public IReadOnlyList<ThemeTokenDiagnostic> Diagnostics { get; }

    public ThemeFallbackState FallbackState { get; }

    public string ActiveThemeId { get; }

    public bool RegistryAccepted { get; }

    public ThemeTokenValue this[string key] => Tokens[key];

    public bool TryGet(string key, out ThemeTokenValue value) => Tokens.TryGetValue(key, out value!);
}

public sealed record ThemeBuiltInTheme
{
    public ThemeBuiltInTheme(
        string id,
        string displayName,
        string version,
        IReadOnlyDictionary<string, ThemeTokenValue> tokens)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(id);
        ArgumentException.ThrowIfNullOrWhiteSpace(displayName);
        ArgumentException.ThrowIfNullOrWhiteSpace(version);
        ArgumentNullException.ThrowIfNull(tokens);
        Id = id.Trim();
        DisplayName = displayName.Trim();
        Version = version.Trim();
        Tokens = new ReadOnlyDictionary<string, ThemeTokenValue>(
            new Dictionary<string, ThemeTokenValue>(tokens, StringComparer.Ordinal));
    }

    public string Id { get; }

    public string DisplayName { get; }

    public string Version { get; }

    public IReadOnlyDictionary<string, ThemeTokenValue> Tokens { get; }
}

/// <summary>
/// Versioned local-only registry contract. Entries carry declarative token
/// values only; package discovery/loading is deliberately outside Gate 3.
/// </summary>
public sealed record ThemeRegistryDocument
{
    public ThemeRegistryDocument()
    {
    }

    public ThemeRegistryDocument(
        int schemaMajor,
        int schemaMinor,
        string activeThemeId,
        IReadOnlyList<ThemeRegistryEntry>? themes = null,
        IReadOnlyDictionary<string, ThemeTokenValue>? localOverrides = null)
    {
        SchemaMajor = schemaMajor;
        SchemaMinor = schemaMinor;
        ActiveThemeId = string.IsNullOrWhiteSpace(activeThemeId) ? BuiltInThemeCatalog.SystemId : activeThemeId.Trim();
        Themes = themes ?? Array.Empty<ThemeRegistryEntry>();
        LocalOverrides = localOverrides ?? new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal);
    }

    public int SchemaMajor { get; init; } = ThemeRegistryContract.CurrentSchemaMajor;

    public int SchemaMinor { get; init; } = ThemeRegistryContract.CurrentSchemaMinor;

    public string ActiveThemeId { get; init; } = BuiltInThemeCatalog.SystemId;

    public IReadOnlyList<ThemeRegistryEntry> Themes { get; init; } = Array.Empty<ThemeRegistryEntry>();

    public IReadOnlyDictionary<string, ThemeTokenValue> LocalOverrides { get; init; } =
        new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal);
}

public sealed record ThemeRegistryEntry
{
    public ThemeRegistryEntry()
    {
    }

    public ThemeRegistryEntry(
        string id,
        string displayName,
        string version,
        ThemeRegistryEntryKind kind,
        bool enabled = true,
        int order = 0,
        IReadOnlyDictionary<string, ThemeTokenValue>? tokens = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(id);
        ArgumentException.ThrowIfNullOrWhiteSpace(displayName);
        ArgumentException.ThrowIfNullOrWhiteSpace(version);
        Id = id.Trim();
        DisplayName = displayName.Trim();
        Version = version.Trim();
        Kind = kind;
        Enabled = enabled;
        Order = order;
        Tokens = tokens ?? new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal);
    }

    public string Id { get; init; } = string.Empty;

    public string DisplayName { get; init; } = string.Empty;

    public string Version { get; init; } = string.Empty;

    public ThemeRegistryEntryKind Kind { get; init; }

    public bool Enabled { get; init; } = true;

    public int Order { get; init; }

    public IReadOnlyDictionary<string, ThemeTokenValue> Tokens { get; init; } =
        new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal);
}

public sealed record ThemeRegistryValidationResult(
    bool IsValid,
    IReadOnlyList<string> Errors,
    IReadOnlyList<string> Warnings);

public static class ThemeRegistryContract
{
    public const int CurrentSchemaMajor = 1;
    public const int CurrentSchemaMinor = 0;

    private static readonly JsonSerializerOptions JsonOptions = CreateJsonOptions();

    public static ThemeRegistryDocument CreateDefault() => new(
        CurrentSchemaMajor,
        CurrentSchemaMinor,
        BuiltInThemeCatalog.SystemId,
        BuiltInThemeCatalog.All.Values
            .Select(theme => new ThemeRegistryEntry(
                theme.Id,
                theme.DisplayName,
                theme.Version,
                ThemeRegistryEntryKind.BuiltIn))
            .ToArray());

    public static ThemeRegistryValidationResult Validate(ThemeRegistryDocument? document)
    {
        var errors = new List<string>();
        var warnings = new List<string>();
        if (document is null)
        {
            return new(false, ["Registry document is required."], warnings);
        }

        if (document.SchemaMajor != CurrentSchemaMajor)
        {
            errors.Add($"Unsupported registry schema major '{document.SchemaMajor}'.");
            return new(false, errors, warnings);
        }

        if (document.SchemaMinor > CurrentSchemaMinor)
        {
            warnings.Add($"Registry schema minor '{document.SchemaMinor}' is newer; unknown fields are ignored.");
        }

        if (string.IsNullOrWhiteSpace(document.ActiveThemeId))
        {
            errors.Add("ActiveThemeId is required.");
        }

        var ids = new HashSet<string>(StringComparer.Ordinal);
        foreach (var entry in document.Themes ?? Array.Empty<ThemeRegistryEntry>())
        {
            if (entry is null || string.IsNullOrWhiteSpace(entry.Id))
            {
                errors.Add("Theme entries require an id.");
                continue;
            }

            if (!ids.Add(entry.Id))
            {
                errors.Add($"Duplicate theme entry '{entry.Id}'.");
            }

            if (string.IsNullOrWhiteSpace(entry.Version))
            {
                errors.Add($"Theme '{entry.Id}' requires a version.");
            }

            if (entry.Kind is not (ThemeRegistryEntryKind.BuiltIn or ThemeRegistryEntryKind.Extension))
            {
                errors.Add($"Theme '{entry.Id}' has an unsupported registry entry kind.");
            }

            if (entry.Order is < -1000 or > 1000)
            {
                errors.Add($"Theme '{entry.Id}' has an out-of-range ordering value.");
            }

            ValidateTokens(entry.Id, entry.Tokens, errors, warnings);
        }

        ValidateTokens("local override", document.LocalOverrides, errors, warnings);
        return new(errors.Count == 0, errors, warnings);
    }

    public static string Serialize(ThemeRegistryDocument document)
    {
        ArgumentNullException.ThrowIfNull(document);
        var validation = Validate(document);
        if (!validation.IsValid)
        {
            throw new ArgumentException(
                "The theme registry contract is invalid: " + string.Join(" ", validation.Errors),
                nameof(document));
        }

        return JsonSerializer.Serialize(document, JsonOptions);
    }

    public static bool TryDeserialize(
        string json,
        out ThemeRegistryDocument? document,
        out ThemeRegistryValidationResult validation)
    {
        document = null;
        if (string.IsNullOrWhiteSpace(json))
        {
            validation = new(false, ["Registry JSON is required."], []);
            return false;
        }

        try
        {
            document = JsonSerializer.Deserialize<ThemeRegistryDocument>(json, JsonOptions);
            validation = Validate(document);
            return validation.IsValid;
        }
        catch (JsonException exception)
        {
            validation = new(false, [$"Registry JSON is invalid: {exception.Message}"], []);
            return false;
        }
        catch (ArgumentException exception)
        {
            validation = new(false, [$"Registry JSON contains an invalid value: {exception.Message}"], []);
            return false;
        }
    }

    private static void ValidateTokens(
        string layer,
        IReadOnlyDictionary<string, ThemeTokenValue>? tokens,
        ICollection<string> errors,
        ICollection<string> warnings)
    {
        if (tokens is null)
        {
            return;
        }

        foreach (var pair in tokens)
        {
            var token = pair.Value;
            if (token is null)
            {
                errors.Add($"{layer} token '{pair.Key}' is null.");
                continue;
            }

            if (!string.Equals(pair.Key, token.Key, StringComparison.Ordinal))
            {
                errors.Add($"{layer} token dictionary key '{pair.Key}' does not match token key '{token.Key}'.");
                continue;
            }

            if (ThemeTokenSchema.TryNormalize(token, out _, out var issue))
            {
                if (issue?.Kind == ThemeTokenIssueKind.ClampedValue)
                {
                    warnings.Add($"{layer} token '{token.Key}' will be clamped.");
                }

                continue;
            }

            errors.Add($"{layer} token '{token.Key}' is invalid: {issue?.Message ?? "unknown error"}");
        }
    }

    private static JsonSerializerOptions CreateJsonOptions() => new(JsonSerializerDefaults.Web)
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };
}

public static class ThemeTokenSchema
{
    private sealed record TokenSpec(ThemeTokenValueKind Kind, double? Minimum = null, double? Maximum = null);

    private static readonly IReadOnlyDictionary<string, TokenSpec> Specs =
        new ReadOnlyDictionary<string, TokenSpec>(new Dictionary<string, TokenSpec>(StringComparer.Ordinal)
        {
            [ThemeTokenKeys.CanvasColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.SurfaceColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.SurfaceSubtleColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.InkColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.MutedInkColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.BorderColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.AccentColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.OnAccentColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.FocusColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.SelectionColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.DisabledColor] = new(ThemeTokenValueKind.Color),
            [ThemeTokenKeys.FontFamily] = new(ThemeTokenValueKind.String),
            [ThemeTokenKeys.BodyFontSize] = new(ThemeTokenValueKind.Number, 8, 72),
            [ThemeTokenKeys.CaptionFontSize] = new(ThemeTokenValueKind.Number, 8, 48),
            [ThemeTokenKeys.HeadingFontSize] = new(ThemeTokenValueKind.Number, 10, 72),
            [ThemeTokenKeys.TitleFontSize] = new(ThemeTokenValueKind.Number, 12, 96),
            [ThemeTokenKeys.BodyFontWeight] = new(ThemeTokenValueKind.String),
            [ThemeTokenKeys.SpacingXs] = new(ThemeTokenValueKind.Number, 0, 64),
            [ThemeTokenKeys.SpacingSm] = new(ThemeTokenValueKind.Number, 0, 64),
            [ThemeTokenKeys.SpacingMd] = new(ThemeTokenValueKind.Number, 0, 96),
            [ThemeTokenKeys.SpacingLg] = new(ThemeTokenValueKind.Number, 0, 128),
            [ThemeTokenKeys.SpacingXl] = new(ThemeTokenValueKind.Number, 0, 192),
            [ThemeTokenKeys.ControlCornerRadius] = new(ThemeTokenValueKind.Number, 0, 32),
            [ThemeTokenKeys.CardCornerRadius] = new(ThemeTokenValueKind.Number, 0, 32),
            [ThemeTokenKeys.PanelCornerRadius] = new(ThemeTokenValueKind.Number, 0, 32),
            [ThemeTokenKeys.PanelElevation] = new(ThemeTokenValueKind.Number, 0, 64),
            [ThemeTokenKeys.SurfaceOpacity] = new(ThemeTokenValueKind.Number, 0, 1),
            [ThemeTokenKeys.BorderOpacity] = new(ThemeTokenValueKind.Number, 0, 1),
            [ThemeTokenKeys.DisabledOpacity] = new(ThemeTokenValueKind.Number, 0, 1),
            [ThemeTokenKeys.SurfaceBlur] = new(ThemeTokenValueKind.Number, 0, 96),
            [ThemeTokenKeys.MaterialSurface] = new(ThemeTokenValueKind.Material),
            [ThemeTokenKeys.MotionEnabled] = new(ThemeTokenValueKind.Boolean),
            [ThemeTokenKeys.MotionFastDuration] = new(ThemeTokenValueKind.Number, 0, 2000),
            [ThemeTokenKeys.MotionNormalDuration] = new(ThemeTokenValueKind.Number, 0, 4000),
            [ThemeTokenKeys.MotionSlowDuration] = new(ThemeTokenValueKind.Number, 0, 8000),
            [ThemeTokenKeys.FocusRingThickness] = new(ThemeTokenValueKind.Number, 1, 8),
            [ThemeTokenKeys.AccessibilityTransparencyAllowed] = new(ThemeTokenValueKind.Boolean),
            [ThemeTokenKeys.AccessibilityFocusVisible] = new(ThemeTokenValueKind.Boolean),
        });

    public static bool TryNormalize(
        ThemeTokenValue source,
        out ThemeTokenValue normalized,
        out ThemeTokenIssue? issue)
    {
        ArgumentNullException.ThrowIfNull(source);
        normalized = source;
        issue = null;

        if (!Specs.TryGetValue(source.Key, out var spec))
        {
            issue = new(source.Key, ThemeTokenIssueKind.UnknownToken, "The token is not in the host schema.");
            return false;
        }

        if (source.Kind != spec.Kind)
        {
            issue = new(
                source.Key,
                ThemeTokenIssueKind.WrongType,
                $"Expected {spec.Kind}, received {source.Kind}.");
            return false;
        }

        switch (source.Kind)
        {
            case ThemeTokenValueKind.Color:
                if (!TryNormalizeColor(source.Value, out var color))
                {
                    issue = new(source.Key, ThemeTokenIssueKind.InvalidValue, "Colors must be #RRGGBB or #AARRGGBB.");
                    return false;
                }

                normalized = new(source.Key, source.Kind, color);
                return true;

            case ThemeTokenValueKind.Number:
                if (!double.TryParse(source.Value, NumberStyles.Float, CultureInfo.InvariantCulture, out var number) ||
                    !double.IsFinite(number))
                {
                    issue = new(source.Key, ThemeTokenIssueKind.InvalidValue, "Numbers must be finite invariant-culture values.");
                    return false;
                }

                var clamped = number;
                if (spec.Minimum is { } minimum && clamped < minimum)
                {
                    clamped = minimum;
                }

                if (spec.Maximum is { } maximum && clamped > maximum)
                {
                    clamped = maximum;
                }

                normalized = new(source.Key, source.Kind, clamped.ToString("R", CultureInfo.InvariantCulture));
                if (clamped != number)
                {
                    issue = new(
                        source.Key,
                        ThemeTokenIssueKind.ClampedValue,
                        $"Value was clamped to [{spec.Minimum}, {spec.Maximum}].",
                        normalized);
                }

                return true;

            case ThemeTokenValueKind.Boolean:
                if (!bool.TryParse(source.Value, out var boolean))
                {
                    issue = new(source.Key, ThemeTokenIssueKind.InvalidValue, "Booleans must be true or false.");
                    return false;
                }

                normalized = new(source.Key, source.Kind, boolean ? "true" : "false");
                return true;

            case ThemeTokenValueKind.Material:
                if (!Enum.TryParse<ThemeMaterial>(source.Value, true, out var material))
                {
                    issue = new(source.Key, ThemeTokenIssueKind.InvalidValue, "Material must be solid, mica, acrylic, or glass.");
                    return false;
                }

                normalized = new(source.Key, source.Kind, material.ToString().ToLowerInvariant());
                return true;

            case ThemeTokenValueKind.String:
                if (source.Value.Length is 0 or > 128 || source.Value.Contains('\0'))
                {
                    issue = new(source.Key, ThemeTokenIssueKind.InvalidValue, "String token length is outside the host limit.");
                    return false;
                }

                normalized = new(source.Key, source.Kind, source.Value);
                return true;

            default:
                issue = new(source.Key, ThemeTokenIssueKind.InvalidValue, "Unknown token value kind.");
                return false;
        }
    }

    private static bool TryNormalizeColor(string value, out string normalized)
    {
        normalized = string.Empty;
        if (value.Length is not (7 or 9) || value[0] != '#')
        {
            return false;
        }

        for (var index = 1; index < value.Length; index++)
        {
            if (!Uri.IsHexDigit(value[index]))
            {
                return false;
            }
        }

        normalized = value.ToUpperInvariant();
        return true;
    }
}

public static class ThemeSystemDefaults
{
    public static IReadOnlyList<ThemeTokenValue> Create(ThemeSystemVariant variant) =>
        [
            ThemeTokenValue.Color(ThemeTokenKeys.CanvasColor, variant == ThemeSystemVariant.Dark ? "#101719" : "#F4F8F7"),
            ThemeTokenValue.Color(ThemeTokenKeys.SurfaceColor, variant == ThemeSystemVariant.Dark ? "#E51A2427" : "#F2FFFFFF"),
            ThemeTokenValue.Color(ThemeTokenKeys.InkColor, variant == ThemeSystemVariant.Dark ? "#F5FAF9" : "#172026"),
            ThemeTokenValue.Color(ThemeTokenKeys.MutedInkColor, variant == ThemeSystemVariant.Dark ? "#B2C1C0" : "#647078"),
            ThemeTokenValue.Material(ThemeTokenKeys.MaterialSurface, ThemeMaterial.Solid),
            ThemeTokenValue.Boolean(ThemeTokenKeys.MotionEnabled, true),
        ];
}

public static class BuiltInThemeCatalog
{
    public const string SystemId = "aurora.system";
    public const string LightId = "aurora.light";
    public const string DarkId = "aurora.dark";

    public static IReadOnlyDictionary<string, ThemeBuiltInTheme> All { get; } =
        new ReadOnlyDictionary<string, ThemeBuiltInTheme>(new Dictionary<string, ThemeBuiltInTheme>(StringComparer.Ordinal)
        {
            [LightId] = CreateLight(),
            [DarkId] = CreateDark(),
        });

    public static ThemeBuiltInTheme ForSystem(ThemeSystemVariant variant) =>
        variant == ThemeSystemVariant.Dark ? All[DarkId] : All[LightId];

    public static bool TryGet(string id, out ThemeBuiltInTheme theme) => All.TryGetValue(id, out theme!);

    private static ThemeBuiltInTheme CreateLight() => new(
        LightId,
        "Aurorix Light",
        "1.0.0",
        CreateSharedTokens(
            canvas: "#DDF1F3",
            surface: "#F2FFFFFF",
            surfaceSubtle: "#EAF3F1",
            ink: "#172026",
            mutedInk: "#647078",
            border: "#841B2529",
            accent: "#315F58",
            onAccent: "#FFFFFFFF",
            focus: "#147A6D",
            selection: "#5C76B7AA",
            disabled: "#7A879095",
            material: ThemeMaterial.Mica));

    private static ThemeBuiltInTheme CreateDark() => new(
        DarkId,
        "Aurorix Dark",
        "1.0.0",
        CreateSharedTokens(
            canvas: "#101719",
            surface: "#E5222D30",
            surfaceSubtle: "#D92C393B",
            ink: "#F5FAF9",
            mutedInk: "#B2C1C0",
            border: "#806E8582",
            accent: "#8DD6C9",
            onAccent: "#10211E",
            focus: "#A1F2E3",
            selection: "#7564B5A8",
            disabled: "#7A899794",
            material: ThemeMaterial.Mica));

    private static IReadOnlyDictionary<string, ThemeTokenValue> CreateSharedTokens(
        string canvas,
        string surface,
        string surfaceSubtle,
        string ink,
        string mutedInk,
        string border,
        string accent,
        string onAccent,
        string focus,
        string selection,
        string disabled,
        ThemeMaterial material) =>
        new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal)
        {
            [ThemeTokenKeys.CanvasColor] = ThemeTokenValue.Color(ThemeTokenKeys.CanvasColor, canvas),
            [ThemeTokenKeys.SurfaceColor] = ThemeTokenValue.Color(ThemeTokenKeys.SurfaceColor, surface),
            [ThemeTokenKeys.SurfaceSubtleColor] = ThemeTokenValue.Color(ThemeTokenKeys.SurfaceSubtleColor, surfaceSubtle),
            [ThemeTokenKeys.InkColor] = ThemeTokenValue.Color(ThemeTokenKeys.InkColor, ink),
            [ThemeTokenKeys.MutedInkColor] = ThemeTokenValue.Color(ThemeTokenKeys.MutedInkColor, mutedInk),
            [ThemeTokenKeys.BorderColor] = ThemeTokenValue.Color(ThemeTokenKeys.BorderColor, border),
            [ThemeTokenKeys.AccentColor] = ThemeTokenValue.Color(ThemeTokenKeys.AccentColor, accent),
            [ThemeTokenKeys.OnAccentColor] = ThemeTokenValue.Color(ThemeTokenKeys.OnAccentColor, onAccent),
            [ThemeTokenKeys.FocusColor] = ThemeTokenValue.Color(ThemeTokenKeys.FocusColor, focus),
            [ThemeTokenKeys.SelectionColor] = ThemeTokenValue.Color(ThemeTokenKeys.SelectionColor, selection),
            [ThemeTokenKeys.DisabledColor] = ThemeTokenValue.Color(ThemeTokenKeys.DisabledColor, disabled),
            [ThemeTokenKeys.FontFamily] = ThemeTokenValue.String(ThemeTokenKeys.FontFamily, "Segoe UI Variable Text"),
            [ThemeTokenKeys.BodyFontSize] = ThemeTokenValue.Number(ThemeTokenKeys.BodyFontSize, 13),
            [ThemeTokenKeys.CaptionFontSize] = ThemeTokenValue.Number(ThemeTokenKeys.CaptionFontSize, 11),
            [ThemeTokenKeys.HeadingFontSize] = ThemeTokenValue.Number(ThemeTokenKeys.HeadingFontSize, 18),
            [ThemeTokenKeys.TitleFontSize] = ThemeTokenValue.Number(ThemeTokenKeys.TitleFontSize, 30),
            [ThemeTokenKeys.BodyFontWeight] = ThemeTokenValue.String(ThemeTokenKeys.BodyFontWeight, "Normal"),
            [ThemeTokenKeys.SpacingXs] = ThemeTokenValue.Number(ThemeTokenKeys.SpacingXs, 4),
            [ThemeTokenKeys.SpacingSm] = ThemeTokenValue.Number(ThemeTokenKeys.SpacingSm, 8),
            [ThemeTokenKeys.SpacingMd] = ThemeTokenValue.Number(ThemeTokenKeys.SpacingMd, 12),
            [ThemeTokenKeys.SpacingLg] = ThemeTokenValue.Number(ThemeTokenKeys.SpacingLg, 18),
            [ThemeTokenKeys.SpacingXl] = ThemeTokenValue.Number(ThemeTokenKeys.SpacingXl, 28),
            [ThemeTokenKeys.ControlCornerRadius] = ThemeTokenValue.Number(ThemeTokenKeys.ControlCornerRadius, 9),
            [ThemeTokenKeys.CardCornerRadius] = ThemeTokenValue.Number(ThemeTokenKeys.CardCornerRadius, 14),
            [ThemeTokenKeys.PanelCornerRadius] = ThemeTokenValue.Number(ThemeTokenKeys.PanelCornerRadius, 18),
            [ThemeTokenKeys.PanelElevation] = ThemeTokenValue.Number(ThemeTokenKeys.PanelElevation, 8),
            [ThemeTokenKeys.SurfaceOpacity] = ThemeTokenValue.Number(ThemeTokenKeys.SurfaceOpacity, 0.94),
            [ThemeTokenKeys.BorderOpacity] = ThemeTokenValue.Number(ThemeTokenKeys.BorderOpacity, 0.52),
            [ThemeTokenKeys.DisabledOpacity] = ThemeTokenValue.Number(ThemeTokenKeys.DisabledOpacity, 0.45),
            [ThemeTokenKeys.SurfaceBlur] = ThemeTokenValue.Number(ThemeTokenKeys.SurfaceBlur, 18),
            [ThemeTokenKeys.MaterialSurface] = ThemeTokenValue.Material(ThemeTokenKeys.MaterialSurface, material),
            [ThemeTokenKeys.MotionEnabled] = ThemeTokenValue.Boolean(ThemeTokenKeys.MotionEnabled, true),
            [ThemeTokenKeys.MotionFastDuration] = ThemeTokenValue.Number(ThemeTokenKeys.MotionFastDuration, 120),
            [ThemeTokenKeys.MotionNormalDuration] = ThemeTokenValue.Number(ThemeTokenKeys.MotionNormalDuration, 200),
            [ThemeTokenKeys.MotionSlowDuration] = ThemeTokenValue.Number(ThemeTokenKeys.MotionSlowDuration, 320),
            [ThemeTokenKeys.FocusRingThickness] = ThemeTokenValue.Number(ThemeTokenKeys.FocusRingThickness, 2),
            [ThemeTokenKeys.AccessibilityTransparencyAllowed] = ThemeTokenValue.Boolean(ThemeTokenKeys.AccessibilityTransparencyAllowed, true),
            [ThemeTokenKeys.AccessibilityFocusVisible] = ThemeTokenValue.Boolean(ThemeTokenKeys.AccessibilityFocusVisible, true),
        };
}
