using System;
using Aurorix.Windows.Themes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Aurorix.Windows.Settings;

public sealed partial class SettingsPage : Page
{
    private bool _suppressSelectionChanged;

    public SettingsPage()
    {
        InitializeComponent();
    }

    public ThemeMaterial SelectedMaterial { get; private set; } = ThemeMaterial.Mica;

    public ThemeSystemVariant SelectedVariant { get; private set; } = ThemeSystemVariant.Light;

    public double CustomAcrylicTransparency { get; private set; } = 0.4;

    public double CustomAcrylicVividness { get; private set; } = 0.6;

    public event EventHandler? SelectionChanged;

    public void SetSelection(
        ThemeMaterial material,
        ThemeSystemVariant variant,
        double customAcrylicTransparency = 0.4,
        double customAcrylicVividness = 0.6)
    {
        SelectedMaterial = material;
        SelectedVariant = variant;
        CustomAcrylicTransparency = Math.Clamp(customAcrylicTransparency, 0, 1);
        CustomAcrylicVividness = Math.Clamp(customAcrylicVividness, 0, 1);

        _suppressSelectionChanged = true;
        MaterialComboBox.SelectedIndex = MaterialIndex(material);
        VariantComboBox.SelectedIndex = variant == ThemeSystemVariant.Dark ? 1 : 0;
        CustomAcrylicTransparencySlider.Value = CustomAcrylicTransparency;
        CustomAcrylicVividnessSlider.Value = CustomAcrylicVividness;
        _suppressSelectionChanged = false;
        UpdateCustomAcrylicOptionsVisibility();
    }

    private void MaterialComboBox_SelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        if (_suppressSelectionChanged || MaterialComboBox.SelectedItem is not ComboBoxItem item ||
            item.Tag is not string tag || !TryParseMaterial(tag, out var material))
        {
            return;
        }

        SelectedMaterial = material;
        UpdateCustomAcrylicOptionsVisibility();
        SelectionChanged?.Invoke(this, EventArgs.Empty);
    }

    private void VariantComboBox_SelectionChanged(object sender, SelectionChangedEventArgs args)
    {
        if (_suppressSelectionChanged || VariantComboBox.SelectedItem is not ComboBoxItem item ||
            item.Tag is not string tag)
        {
            return;
        }

        SelectedVariant = string.Equals(tag, "dark", StringComparison.OrdinalIgnoreCase)
            ? ThemeSystemVariant.Dark
            : ThemeSystemVariant.Light;
        SelectionChanged?.Invoke(this, EventArgs.Empty);
    }

    private void CustomAcrylicTransparencySlider_ValueChanged(
        object sender,
        RangeBaseValueChangedEventArgs args)
    {
        if (_suppressSelectionChanged)
        {
            return;
        }

        CustomAcrylicTransparency = args.NewValue;
        SelectionChanged?.Invoke(this, EventArgs.Empty);
    }

    private void CustomAcrylicVividnessSlider_ValueChanged(
        object sender,
        RangeBaseValueChangedEventArgs args)
    {
        if (_suppressSelectionChanged)
        {
            return;
        }

        CustomAcrylicVividness = args.NewValue;
        SelectionChanged?.Invoke(this, EventArgs.Empty);
    }

    private void UpdateCustomAcrylicOptionsVisibility()
    {
        CustomAcrylicOptionsPanel.Visibility = SelectedMaterial == ThemeMaterial.CustomAcrylic
            ? Visibility.Visible
            : Visibility.Collapsed;
    }

    private static int MaterialIndex(ThemeMaterial material) =>
        material switch
        {
            ThemeMaterial.MicaAlt => 1,
            ThemeMaterial.Acrylic => 2,
            ThemeMaterial.TransparentAcrylic => 3,
            ThemeMaterial.CustomAcrylic => 4,
            ThemeMaterial.None => 5,
            _ => 0,
        };

    private static bool TryParseMaterial(string value, out ThemeMaterial material)
    {
        material = value switch
        {
            "acrylic" => ThemeMaterial.Acrylic,
            "transparent-acrylic" => ThemeMaterial.TransparentAcrylic,
            "custom-acrylic" => ThemeMaterial.CustomAcrylic,
            "mica" => ThemeMaterial.Mica,
            "mica-alt" => ThemeMaterial.MicaAlt,
            "none" => ThemeMaterial.None,
            _ => ThemeMaterial.Solid,
        };
        return material != ThemeMaterial.Solid;
    }
}
