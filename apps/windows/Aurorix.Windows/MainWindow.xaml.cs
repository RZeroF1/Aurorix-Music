using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.UI.ViewManagement;
using WinRT.Interop;

namespace Aurorix.Windows;

/// <summary>
/// Minimal native window host. Feature surfaces are added only after their
/// layout and ownership are explicitly agreed.
/// </summary>
public sealed partial class MainWindow : Window
{
    private AppWindow? _appWindow;
    private readonly AccessibilitySettings _accessibilitySettings;

    public MainWindow()
    {
        InitializeComponent();

        _accessibilitySettings = new AccessibilitySettings();
        TrySubscribeToAccessibilityChanges();
        ApplyHighContrastBackdrop();

        ConfigureWindowBounds();
    }

    private void AccessibilitySettings_HighContrastChanged(AccessibilitySettings sender, object args)
    {
        ApplyHighContrastBackdrop();
    }

    private void TrySubscribeToAccessibilityChanges()
    {
        try
        {
            _accessibilitySettings.HighContrastChanged += AccessibilitySettings_HighContrastChanged;
        }
        catch (COMException)
        {
            // Some packaged environments do not expose the WinRT event source.
            // Initial HighContrast state remains available; startup must not fail.
        }
    }

    private void ApplyHighContrastBackdrop()
    {
        SystemBackdrop = _accessibilitySettings.HighContrast ? null : new MicaBackdrop();
    }

    private void ConfigureWindowBounds()
    {
        var windowHandle = WindowNative.GetWindowHandle(this);
        var windowId = Microsoft.UI.Win32Interop.GetWindowIdFromWindow(windowHandle);
        _appWindow = AppWindow.GetFromWindowId(windowId);

        if (_appWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = 960;
            presenter.PreferredMinimumHeight = 640;
        }
    }

}
