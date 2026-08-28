using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.System.Power;
using Windows.UI;
using Windows.UI.ViewManagement;

namespace Aurorix.Windows.Themes;

public interface IThemeEnvironmentSource
{
    ThemeEnvironment Current { get; }

    event EventHandler? Changed;
}

/// <summary>
/// Reads the current Windows appearance/accessibility/power settings and
/// republishes them as host inputs. The source has no theme persistence and
/// does not load packages; it only observes OS state.
/// </summary>
public sealed class WindowsThemeEnvironmentSource : IThemeEnvironmentSource, IDisposable
{
    private readonly UISettings _uiSettings = new();
    private readonly AccessibilitySettings _accessibilitySettings = new();
    private bool _disposed;

    public WindowsThemeEnvironmentSource()
    {
        Current = ReadEnvironment();
        _uiSettings.ColorValuesChanged += OnColorValuesChanged;
        _accessibilitySettings.HighContrastChanged += OnHighContrastChanged;
        PowerManager.EnergySaverStatusChanged += OnEnergySaverStatusChanged;
    }

    public ThemeEnvironment Current { get; private set; }

    public event EventHandler? Changed;

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _uiSettings.ColorValuesChanged -= OnColorValuesChanged;
        _accessibilitySettings.HighContrastChanged -= OnHighContrastChanged;
        PowerManager.EnergySaverStatusChanged -= OnEnergySaverStatusChanged;
        _disposed = true;
    }

    private void OnColorValuesChanged(UISettings sender, object args) => Refresh();

    private void OnHighContrastChanged(AccessibilitySettings sender, object args) => Refresh();

    private void OnEnergySaverStatusChanged(object? sender, object args) => Refresh();

    private void Refresh()
    {
        if (_disposed)
        {
            return;
        }

        var next = ReadEnvironment();
        if (next == Current)
        {
            return;
        }

        Current = next;
        Changed?.Invoke(this, EventArgs.Empty);
    }

    private ThemeEnvironment ReadEnvironment()
    {
        var background = _uiSettings.GetColorValue(UIColorType.Background);
        var luminance = (0.2126 * background.R) + (0.7152 * background.G) + (0.0722 * background.B);
        return new(
            systemVariant: luminance < 128 ? ThemeSystemVariant.Dark : ThemeSystemVariant.Light,
            highContrast: _accessibilitySettings.HighContrast,
            reducedMotion: !_uiSettings.AnimationsEnabled,
            powerSaver: PowerManager.EnergySaverStatus == EnergySaverStatus.On,
            supportsMica: OperatingSystem.IsWindowsVersionAtLeast(10, 0, 22000),
            supportsAcrylic: true,
            supportsGlass: false);
    }
}

/// <summary>
/// Deterministic adapter for focused contract tests and host callers that
/// already own a Windows settings bridge.
/// </summary>
public sealed class MutableThemeEnvironmentSource : IThemeEnvironmentSource
{
    public MutableThemeEnvironmentSource(ThemeEnvironment initial)
    {
        Current = initial ?? throw new ArgumentNullException(nameof(initial));
    }

    public ThemeEnvironment Current { get; private set; }

    public event EventHandler? Changed;

    public void Update(ThemeEnvironment environment)
    {
        Current = environment ?? throw new ArgumentNullException(nameof(environment));
        Changed?.Invoke(this, EventArgs.Empty);
    }
}

public sealed record ThemeTokensChangedEventArgs(
    ThemeResolutionResult Previous,
    ThemeResolutionResult Current);

/// <summary>
/// Owns semantic token resolution and resource projection. Layer precedence is
/// fixed: system defaults, built-in theme, local override, extension, then
/// accessibility/power safety fallback.
/// </summary>
public sealed class ThemeTokenHost : IDisposable
{
    private readonly IThemeEnvironmentSource _environmentSource;
    private ThemeRegistryDocument _registry;
    private ThemeResolutionResult _current;
    private bool _disposed;

    public ThemeTokenHost(
        IThemeEnvironmentSource environmentSource,
        ThemeRegistryDocument? registry = null)
    {
        _environmentSource = environmentSource ?? throw new ArgumentNullException(nameof(environmentSource));
        _registry = registry ?? ThemeRegistryContract.CreateDefault();
        _current = Resolve(_environmentSource.Current, _registry);
        _environmentSource.Changed += OnEnvironmentChanged;
    }

    public ThemeTokenHost(
        ThemeEnvironment environment,
        ThemeRegistryDocument? registry = null)
        : this(new MutableThemeEnvironmentSource(environment), registry)
    {
    }

    public ThemeResolutionResult Current => _current;

    public ThemeRegistryDocument Registry => _registry;

    public event EventHandler<ThemeTokensChangedEventArgs>? TokensChanged;

    public bool TryUpdateRegistry(ThemeRegistryDocument registry, out ThemeRegistryValidationResult validation)
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
        validation = ThemeRegistryContract.Validate(registry);
        if (!validation.IsValid)
        {
            return false;
        }

        _registry = registry;
        Recompute();
        return true;
    }

    /// <summary>
    /// Applies resolved semantic values to an application ResourceDictionary.
    /// Color brushes are derived by the host so material choices remain
    /// replaceable and existing compatibility keys remain coherent.
    /// </summary>
    public void ApplyTo(ResourceDictionary resources)
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
        ArgumentNullException.ThrowIfNull(resources);

        foreach (var token in _current.Tokens.Values)
        {
            switch (token.Kind)
            {
                case ThemeTokenValueKind.Color:
                    var color = ParseColor(token.Value);
                    resources[token.Key] = color;
                    resources[ThemeTokenKeys.BrushForColor(token.Key)] = new SolidColorBrush(color);
                    ApplyCompatibilityBrush(resources, token.Key, color);
                    break;

                case ThemeTokenValueKind.Number:
                    resources[token.Key] = double.Parse(token.Value, CultureInfo.InvariantCulture);
                    break;

                case ThemeTokenValueKind.Boolean:
                    resources[token.Key] = bool.Parse(token.Value);
                    break;

                case ThemeTokenValueKind.Material:
                case ThemeTokenValueKind.String:
                    resources[token.Key] = token.Value;
                    break;
            }
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _environmentSource.Changed -= OnEnvironmentChanged;
        _disposed = true;
    }

    public static ThemeResolutionResult Resolve(
        ThemeEnvironment environment,
        ThemeRegistryDocument registry)
    {
        ArgumentNullException.ThrowIfNull(environment);
        ArgumentNullException.ThrowIfNull(registry);

        var diagnostics = new List<ThemeTokenDiagnostic>();
        var registryValidation = ThemeRegistryContract.Validate(registry);
        var accepted = registryValidation.IsValid;
        if (!accepted)
        {
            diagnostics.Add(new(
                "registry",
                string.Empty,
                ThemeTokenDiagnosticKind.RegistryRejected,
                string.Join(" ", registryValidation.Errors)));
            registry = ThemeRegistryContract.CreateDefault();
        }

        var builtIn = ResolveBuiltIn(environment, registry.ActiveThemeId, diagnostics);
        var resolved = new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal);
        ApplyLayer("system defaults", ThemeSystemDefaults.Create(environment.SystemVariant), resolved, diagnostics);
        ApplyLayer("built-in", builtIn.Tokens.Values, resolved, diagnostics);
        ApplyLayer("local override", registry.LocalOverrides?.Values ?? [], resolved, diagnostics);

        foreach (var extension in (registry.Themes ?? [])
                     .Where(entry => entry.Kind == ThemeRegistryEntryKind.Extension && entry.Enabled)
                     .OrderBy(entry => entry.Order)
                     .ThenBy(entry => entry.Id, StringComparer.Ordinal))
        {
            ApplyLayer("extension:" + extension.Id, extension.Tokens?.Values ?? [], resolved, diagnostics);
        }

        var fallbackState = ApplyFallbacks(environment, resolved, diagnostics);
        return new(
            resolved,
            diagnostics,
            fallbackState,
            registry.ActiveThemeId,
            accepted);
    }

    public static ThemeResolutionResult Resolve(
        ThemeEnvironment environment,
        ThemeBuiltInTheme builtIn,
        IEnumerable<ThemeTokenValue>? localOverride = null,
        IEnumerable<ThemeTokenValue>? extensionOverride = null)
    {
        ArgumentNullException.ThrowIfNull(environment);
        ArgumentNullException.ThrowIfNull(builtIn);
        var diagnostics = new List<ThemeTokenDiagnostic>();
        var resolved = new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal);
        ApplyLayer("system defaults", ThemeSystemDefaults.Create(environment.SystemVariant), resolved, diagnostics);
        ApplyLayer("built-in", builtIn.Tokens.Values, resolved, diagnostics);
        ApplyLayer("local override", localOverride ?? [], resolved, diagnostics);
        ApplyLayer("extension", extensionOverride ?? [], resolved, diagnostics);
        var fallbackState = ApplyFallbacks(environment, resolved, diagnostics);
        return new(resolved, diagnostics, fallbackState, builtIn.Id, true);
    }

    private static ThemeBuiltInTheme ResolveBuiltIn(
        ThemeEnvironment environment,
        string activeThemeId,
        ICollection<ThemeTokenDiagnostic> diagnostics)
    {
        if (activeThemeId == BuiltInThemeCatalog.SystemId)
        {
            return BuiltInThemeCatalog.ForSystem(environment.SystemVariant);
        }

        if (BuiltInThemeCatalog.TryGet(activeThemeId, out var builtIn))
        {
            return builtIn;
        }

        diagnostics.Add(new(
            "built-in",
            activeThemeId,
            ThemeTokenDiagnosticKind.UnknownTheme,
            "Unknown built-in theme; system-selected built-in was used."));
        return BuiltInThemeCatalog.ForSystem(environment.SystemVariant);
    }

    private static void ApplyLayer(
        string layer,
        IEnumerable<ThemeTokenValue> tokens,
        IDictionary<string, ThemeTokenValue> resolved,
        ICollection<ThemeTokenDiagnostic> diagnostics)
    {
        foreach (var token in tokens)
        {
            if (ThemeTokenSchema.TryNormalize(token, out var normalized, out var issue))
            {
                resolved[normalized.Key] = normalized;
                if (issue?.Kind == ThemeTokenIssueKind.ClampedValue)
                {
                    diagnostics.Add(new(layer, token.Key, ThemeTokenDiagnosticKind.ClampedToken, issue.Message));
                }

                continue;
            }

            diagnostics.Add(new(
                layer,
                token.Key,
                ThemeTokenDiagnosticKind.InvalidToken,
                issue?.Message ?? "Invalid token was ignored."));
        }
    }

    private static ThemeFallbackState ApplyFallbacks(
        ThemeEnvironment environment,
        IDictionary<string, ThemeTokenValue> resolved,
        ICollection<ThemeTokenDiagnostic> diagnostics)
    {
        var accessibilityApplied = false;
        var powerApplied = false;
        var reducedMotionApplied = false;

        if (environment.HighContrast)
        {
            accessibilityApplied = true;
            SetColor(resolved, ThemeTokenKeys.CanvasColor, "#000000");
            SetColor(resolved, ThemeTokenKeys.SurfaceColor, "#000000");
            SetColor(resolved, ThemeTokenKeys.SurfaceSubtleColor, "#000000");
            SetColor(resolved, ThemeTokenKeys.InkColor, "#FFFFFF");
            SetColor(resolved, ThemeTokenKeys.MutedInkColor, "#FFFFFF");
            SetColor(resolved, ThemeTokenKeys.BorderColor, "#FFFFFF");
            SetColor(resolved, ThemeTokenKeys.AccentColor, "#FFFFFF");
            SetColor(resolved, ThemeTokenKeys.OnAccentColor, "#000000");
            SetColor(resolved, ThemeTokenKeys.FocusColor, "#FFFFFF");
            SetColor(resolved, ThemeTokenKeys.SelectionColor, "#FFFFFF");
            SetNumber(resolved, ThemeTokenKeys.SurfaceOpacity, 1);
            SetNumber(resolved, ThemeTokenKeys.BorderOpacity, 1);
            SetBoolean(resolved, ThemeTokenKeys.AccessibilityTransparencyAllowed, false);
            diagnostics.Add(new(
                "accessibility",
                ThemeTokenKeys.InkColor,
                ThemeTokenDiagnosticKind.AccessibilityFallback,
                "High contrast forced opaque, high-contrast semantic colors."));
        }

        if (environment.PowerSaver)
        {
            powerApplied = true;
            SetNumber(resolved, ThemeTokenKeys.SurfaceOpacity, 1);
            SetNumber(resolved, ThemeTokenKeys.SurfaceBlur, 0);
            SetBoolean(resolved, ThemeTokenKeys.AccessibilityTransparencyAllowed, false);
            diagnostics.Add(new(
                "power",
                ThemeTokenKeys.SurfaceBlur,
                ThemeTokenDiagnosticKind.PowerFallback,
                "Power saver disabled blur and transparency-heavy surfaces."));
        }

        if (environment.ReducedMotion || environment.PowerSaver)
        {
            reducedMotionApplied = true;
            SetBoolean(resolved, ThemeTokenKeys.MotionEnabled, false);
            SetNumber(resolved, ThemeTokenKeys.MotionFastDuration, 0);
            SetNumber(resolved, ThemeTokenKeys.MotionNormalDuration, 0);
            SetNumber(resolved, ThemeTokenKeys.MotionSlowDuration, 0);
            diagnostics.Add(new(
                environment.PowerSaver ? "power" : "accessibility",
                ThemeTokenKeys.MotionEnabled,
                environment.PowerSaver
                    ? ThemeTokenDiagnosticKind.PowerFallback
                    : ThemeTokenDiagnosticKind.ReducedMotionFallback,
                "Motion was disabled by the active system safety setting."));
        }

        var requestedMaterial = GetMaterial(resolved, ThemeMaterial.Solid);
        var effectiveMaterial = ThemeMaterial.Solid;
        var materialFallbackApplied = false;
        if (!environment.HighContrast && !environment.PowerSaver)
        {
            effectiveMaterial = SelectMaterial(requestedMaterial, environment);
            materialFallbackApplied = effectiveMaterial != requestedMaterial;
        }
        else
        {
            materialFallbackApplied = requestedMaterial != ThemeMaterial.Solid;
            effectiveMaterial = ThemeMaterial.Solid;
        }

        if (materialFallbackApplied)
        {
            diagnostics.Add(new(
                "fallback",
                ThemeTokenKeys.MaterialSurface,
                ThemeTokenDiagnosticKind.MaterialFallback,
                $"Material '{requestedMaterial}' fell back to '{effectiveMaterial}'."));
        }

        SetMaterial(resolved, ThemeTokenKeys.MaterialSurface, effectiveMaterial);
        if (environment.HighContrast || environment.PowerSaver || effectiveMaterial == ThemeMaterial.Solid)
        {
            SetNumber(resolved, ThemeTokenKeys.SurfaceBlur, 0);
        }

        return new(
            materialFallbackApplied,
            accessibilityApplied,
            powerApplied,
            reducedMotionApplied,
            effectiveMaterial);
    }

    private static ThemeMaterial SelectMaterial(ThemeMaterial requested, ThemeEnvironment environment) =>
        requested switch
        {
            ThemeMaterial.Mica when environment.SupportsMica => ThemeMaterial.Mica,
            ThemeMaterial.Mica when environment.SupportsAcrylic => ThemeMaterial.Acrylic,
            ThemeMaterial.Acrylic when environment.SupportsAcrylic => ThemeMaterial.Acrylic,
            ThemeMaterial.Glass when environment.SupportsGlass => ThemeMaterial.Glass,
            ThemeMaterial.Glass when environment.SupportsAcrylic => ThemeMaterial.Acrylic,
            _ => ThemeMaterial.Solid,
        };

    private static ThemeMaterial GetMaterial(
        IDictionary<string, ThemeTokenValue> tokens,
        ThemeMaterial fallback)
    {
        return tokens.TryGetValue(ThemeTokenKeys.MaterialSurface, out var token) &&
            Enum.TryParse<ThemeMaterial>(token.Value, true, out var material)
            ? material
            : fallback;
    }

    private static void SetColor(IDictionary<string, ThemeTokenValue> tokens, string key, string value) =>
        tokens[key] = ThemeTokenValue.Color(key, value);

    private static void SetNumber(IDictionary<string, ThemeTokenValue> tokens, string key, double value) =>
        tokens[key] = ThemeTokenValue.Number(key, value);

    private static void SetBoolean(IDictionary<string, ThemeTokenValue> tokens, string key, bool value) =>
        tokens[key] = ThemeTokenValue.Boolean(key, value);

    private static void SetMaterial(IDictionary<string, ThemeTokenValue> tokens, string key, ThemeMaterial value) =>
        tokens[key] = ThemeTokenValue.Material(key, value);

    private void OnEnvironmentChanged(object? sender, EventArgs args)
    {
        if (!_disposed)
        {
            Recompute();
        }
    }

    private void Recompute()
    {
        var previous = _current;
        _current = Resolve(_environmentSource.Current, _registry);
        TokensChanged?.Invoke(this, new(previous, _current));
    }

    private static Color ParseColor(string value)
    {
        var hex = value[1..];
        var alpha = byte.Parse(hex.Length == 8 ? hex[..2] : "FF", NumberStyles.HexNumber, CultureInfo.InvariantCulture);
        var offset = hex.Length == 8 ? 2 : 0;
        var red = byte.Parse(hex.Substring(offset, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
        var green = byte.Parse(hex.Substring(offset + 2, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
        var blue = byte.Parse(hex.Substring(offset + 4, 2), NumberStyles.HexNumber, CultureInfo.InvariantCulture);
        return Color.FromArgb(alpha, red, green, blue);
    }

    private static void ApplyCompatibilityBrush(ResourceDictionary resources, string tokenKey, Color color)
    {
        var key = tokenKey switch
        {
            ThemeTokenKeys.CanvasColor => "AurorixCanvasBrush",
            ThemeTokenKeys.InkColor => "AurorixInkBrush",
            ThemeTokenKeys.MutedInkColor => "AurorixMutedInkBrush",
            ThemeTokenKeys.BorderColor => "AurorixGlassBorderBrush",
            _ => null,
        };

        if (key is not null)
        {
            resources[key] = new SolidColorBrush(color);
        }

        if (tokenKey == ThemeTokenKeys.SurfaceColor)
        {
            resources["AurorixGlassBrush"] = new SolidColorBrush(color);
        }
    }
}
