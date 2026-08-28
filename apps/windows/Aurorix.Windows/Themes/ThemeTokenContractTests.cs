#if WINDOWS_THEME_CONTRACT_TESTS

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace Aurorix.Windows.Themes;

/// <summary>
/// Dependency-free probes for the Gate 3 theme contract. The temporary
/// Windows test runner defines WINDOWS_THEME_CONTRACT_TESTS and calls RunAll.
/// </summary>
public static class ThemeTokenContractTests
{
    public static void RunAll()
    {
        MergePrecedenceIsStable();
        RegistryLayersUseTheSamePrecedence();
        InvalidValuesAreIgnoredAndNumbersAreClamped();
        MaterialFallsBackWhenThePlatformCannotRenderIt();
        RegistryRoundTripsAndRejectsUnknownMajor();
        MalformedRegistryValuesFailClosed();
        SystemChangesRecomputeTheHost();
        AccessibilityAndPowerSettingsDisableUnsafeEffects();
    }

    private static void MergePrecedenceIsStable()
    {
        var result = ThemeTokenHost.Resolve(
            new ThemeEnvironment(),
            BuiltInThemeCatalog.ForSystem(ThemeSystemVariant.Light),
            [ThemeTokenValue.Color(ThemeTokenKeys.AccentColor, "#112233")],
            [ThemeTokenValue.Color(ThemeTokenKeys.AccentColor, "#445566")]);

        Equal("#445566", result[ThemeTokenKeys.AccentColor].Value);
    }

    private static void InvalidValuesAreIgnoredAndNumbersAreClamped()
    {
        var result = ThemeTokenHost.Resolve(
            new ThemeEnvironment(),
            BuiltInThemeCatalog.ForSystem(ThemeSystemVariant.Light),
            [
                new ThemeTokenValue(ThemeTokenKeys.BodyFontSize, ThemeTokenValueKind.Number, "999"),
                new ThemeTokenValue(ThemeTokenKeys.AccentColor, ThemeTokenValueKind.Color, "not-a-color"),
                new ThemeTokenValue("extension.unknown", ThemeTokenValueKind.String, "ignored"),
            ]);

        Equal("72", result[ThemeTokenKeys.BodyFontSize].Value);
        Equal("#315F58", result[ThemeTokenKeys.AccentColor].Value);
        True(result.Diagnostics.Any(item => item.Kind == ThemeTokenDiagnosticKind.ClampedToken));
        True(result.Diagnostics.Count(item => item.Kind == ThemeTokenDiagnosticKind.InvalidToken) == 2);
    }

    private static void RegistryLayersUseTheSamePrecedence()
    {
        var registry = new ThemeRegistryDocument(
            ThemeRegistryContract.CurrentSchemaMajor,
            ThemeRegistryContract.CurrentSchemaMinor,
            BuiltInThemeCatalog.LightId,
            themes:
            [
                new ThemeRegistryEntry(
                    "test.extension",
                    "Test extension",
                    "1.0.0",
                    ThemeRegistryEntryKind.Extension,
                    tokens: new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal)
                    {
                        [ThemeTokenKeys.AccentColor] = ThemeTokenValue.Color(ThemeTokenKeys.AccentColor, "#445566"),
                    }),
            ],
            localOverrides: new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal)
            {
                [ThemeTokenKeys.AccentColor] = ThemeTokenValue.Color(ThemeTokenKeys.AccentColor, "#112233"),
            });

        var result = ThemeTokenHost.Resolve(new ThemeEnvironment(), registry);
        Equal("#445566", result[ThemeTokenKeys.AccentColor].Value);
    }

    private static void MaterialFallsBackWhenThePlatformCannotRenderIt()
    {
        var result = ThemeTokenHost.Resolve(
            new ThemeEnvironment(supportsMica: false, supportsAcrylic: false, supportsGlass: false),
            BuiltInThemeCatalog.ForSystem(ThemeSystemVariant.Light));

        Equal("solid", result[ThemeTokenKeys.MaterialSurface].Value);
        True(result.FallbackState.MaterialFallbackApplied);
    }

    private static void RegistryRoundTripsAndRejectsUnknownMajor()
    {
        var registry = new ThemeRegistryDocument(
            ThemeRegistryContract.CurrentSchemaMajor,
            ThemeRegistryContract.CurrentSchemaMinor,
            BuiltInThemeCatalog.DarkId,
            localOverrides: new Dictionary<string, ThemeTokenValue>(StringComparer.Ordinal)
            {
                [ThemeTokenKeys.SpacingSm] = ThemeTokenValue.Number(ThemeTokenKeys.SpacingSm, 10),
            });
        var json = ThemeRegistryContract.Serialize(registry);
        True(ThemeRegistryContract.TryDeserialize(json, out var parsed, out var valid));
        True(valid.IsValid);
        Equal(BuiltInThemeCatalog.DarkId, parsed!.ActiveThemeId);
        Equal("10", parsed.LocalOverrides[ThemeTokenKeys.SpacingSm].Value);

        var unsupported = registry with { SchemaMajor = ThemeRegistryContract.CurrentSchemaMajor + 1 };
        True(!ThemeRegistryContract.Validate(unsupported).IsValid);
    }

    private static void MalformedRegistryValuesFailClosed()
    {
        var json = "{\"schemaMajor\":1,\"schemaMinor\":0,\"activeThemeId\":\"aurora.system\",\"themes\":[],\"localOverrides\":{\"Aurorix.Theme.Color.Accent\":null}}";
        True(!ThemeRegistryContract.TryDeserialize(json, out _, out var validation));
        True(!validation.IsValid);
    }

    private static void SystemChangesRecomputeTheHost()
    {
        var source = new MutableThemeEnvironmentSource(new ThemeEnvironment(ThemeSystemVariant.Light));
        using var host = new ThemeTokenHost(source);
        var changes = 0;
        host.TokensChanged += (_, _) => changes++;
        source.Update(new ThemeEnvironment(ThemeSystemVariant.Dark));
        Equal("#101719", host.Current[ThemeTokenKeys.CanvasColor].Value);
        Equal(1, changes);
    }

    private static void AccessibilityAndPowerSettingsDisableUnsafeEffects()
    {
        var result = ThemeTokenHost.Resolve(
            new ThemeEnvironment(highContrast: true, powerSaver: true, reducedMotion: true),
            BuiltInThemeCatalog.ForSystem(ThemeSystemVariant.Light),
            extensionOverride:
            [
                ThemeTokenValue.Material(ThemeTokenKeys.MaterialSurface, ThemeMaterial.Glass),
                ThemeTokenValue.Boolean(ThemeTokenKeys.MotionEnabled, true),
            ]);

        Equal("solid", result[ThemeTokenKeys.MaterialSurface].Value);
        Equal("false", result[ThemeTokenKeys.MotionEnabled].Value);
        Equal("0", result[ThemeTokenKeys.SurfaceBlur].Value);
        Equal("#FFFFFF", result[ThemeTokenKeys.InkColor].Value);
        True(result.FallbackState.AccessibilityFallbackApplied);
        True(result.FallbackState.PowerFallbackApplied);
        True(result.FallbackState.ReducedMotionFallbackApplied);
    }

    private static void True(bool condition)
    {
        if (!condition)
        {
            throw new InvalidOperationException("Theme contract assertion failed.");
        }
    }

    private static void Equal<T>(T expected, T actual)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
        {
            throw new InvalidOperationException(
                string.Format(CultureInfo.InvariantCulture, "Theme contract assertion failed. Expected '{0}', actual '{1}'.", expected, actual));
        }
    }
}

#endif
