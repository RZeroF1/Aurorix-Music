using System;
using System.Linq;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Navigation;
using Windows.System;
using Windows.UI.ViewManagement;
using WinRT.Interop;

namespace Aurorix.Windows;

/// <summary>
/// WinUI shell host. This class owns presentation navigation only; Core owns
/// playback, queue, clock, and statistics state.
/// </summary>
public sealed partial class MainWindow : Window
{
    private AppWindow? _appWindow;
    private readonly AccessibilitySettings _accessibilitySettings;
    private bool _isSynchronizingSelection;
    private Shell.ShellRoute _currentRoute = Shell.ShellNavigation.Home;

    /// <summary>
    /// Exposes the system motion preference for future shell surfaces. The
    /// shell has no custom transitions, so reduced motion is respected without
    /// creating a second animation state machine.
    /// </summary>
    public bool ReducedMotionEnabled { get; }

    public MainWindow()
    {
        InitializeComponent();

        // Keep the native caption buttons while allowing the app surface to
        // use the title-bar area for its own visual language.
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);

        var uiSettings = new UISettings();
        ReducedMotionEnabled = !uiSettings.AnimationsEnabled;

        _accessibilitySettings = new AccessibilitySettings();
        _accessibilitySettings.HighContrastChanged += AccessibilitySettings_HighContrastChanged;
        ApplyHighContrastBackdrop();

        ConfigureWindowBounds();
        ContentFrame.Navigate(typeof(Shell.ShellPlaceholderPage), _currentRoute);
        UpdateHistoryButtons();
    }

    private void AccessibilitySettings_HighContrastChanged(AccessibilitySettings sender, object args)
    {
        ApplyHighContrastBackdrop();
    }

    private void ApplyHighContrastBackdrop()
    {
        // Mica is decorative only. Removing it in high contrast keeps the
        // system-provided foreground/background resources authoritative.
        SystemBackdrop = _accessibilitySettings.HighContrast ? null : new MicaBackdrop();
    }

    private void ConfigureWindowBounds()
    {
        // PerMonitorV2 is declared in app.manifest. The layout remains in
        // effective pixels and this presenter bound applies at every DPI.
        var windowHandle = WindowNative.GetWindowHandle(this);
        var windowId = Microsoft.UI.Win32Interop.GetWindowIdFromWindow(windowHandle);
        _appWindow = AppWindow.GetFromWindowId(windowId);

        if (_appWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = 960;
            presenter.PreferredMinimumHeight = 640;
        }
    }

    private void AppNavigation_SelectionChanged(
        NavigationView sender,
        NavigationViewSelectionChangedEventArgs args)
    {
        if (_isSynchronizingSelection ||
            args.SelectedItemContainer is not NavigationViewItem item ||
            item.Tag is not string routeKey ||
            !Shell.ShellNavigation.TryGetRoute(routeKey, out var route))
        {
            return;
        }

        NavigateTo(route);
    }

    private void ContentFrame_Navigated(object sender, NavigationEventArgs e)
    {
        if (e.Content is Shell.ShellPlaceholderPage page)
        {
            _currentRoute = page.Route;
            SynchronizeSelection(page.Route.Key);
        }

        UpdateHistoryButtons();
    }

    private void NavigateTo(Shell.ShellRoute route)
    {
        // SelectionFollowsFocus can raise this event while moving through the
        // rail with arrow keys. Avoid duplicate entries for the current route.
        if (Equals(_currentRoute, route) && ContentFrame.Content is not null)
        {
            return;
        }

        ContentFrame.Navigate(typeof(Shell.ShellPlaceholderPage), route);
    }

    private void SynchronizeSelection(string routeKey)
    {
        var item = AppNavigation.MenuItems
            .OfType<NavigationViewItem>()
            .Concat(AppNavigation.FooterMenuItems.OfType<NavigationViewItem>())
            .FirstOrDefault(candidate => string.Equals(candidate.Tag as string, routeKey, StringComparison.OrdinalIgnoreCase));

        if (item is null || ReferenceEquals(AppNavigation.SelectedItem, item))
        {
            return;
        }

        _isSynchronizingSelection = true;
        try
        {
            AppNavigation.SelectedItem = item;
        }
        finally
        {
            _isSynchronizingSelection = false;
        }
    }

    private void UpdateHistoryButtons()
    {
        BackButton.IsEnabled = ContentFrame.CanGoBack;
        ForwardButton.IsEnabled = ContentFrame.CanGoForward;
    }

    private void BackButton_Click(object sender, RoutedEventArgs e)
    {
        if (ContentFrame.CanGoBack)
        {
            ContentFrame.GoBack();
        }
    }

    private void ForwardButton_Click(object sender, RoutedEventArgs e)
    {
        if (ContentFrame.CanGoForward)
        {
            ContentFrame.GoForward();
        }
    }

    private void SearchBox_QuerySubmitted(AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args)
    {
        NavigateTo(Shell.ShellNavigation.Search(sender.Text));
    }

    private void SearchFocusShortcut_Invoked(
        KeyboardAccelerator sender,
        KeyboardAcceleratorInvokedEventArgs args)
    {
        SearchBox.Focus(FocusState.Keyboard);
        args.Handled = true;
    }

    private void HomeShortcut_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        SelectRoute("home");
        args.Handled = true;
    }

    private void LibraryShortcut_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        SelectRoute("library");
        args.Handled = true;
    }

    private void SearchShortcut_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        SelectRoute("search");
        args.Handled = true;
    }

    private void PlaylistsShortcut_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        SelectRoute("playlists");
        args.Handled = true;
    }

    private void SelectRoute(string routeKey)
    {
        var item = AppNavigation.MenuItems
            .OfType<NavigationViewItem>()
            .FirstOrDefault(candidate => string.Equals(candidate.Tag as string, routeKey, StringComparison.OrdinalIgnoreCase));

        if (item is null)
        {
            return;
        }

        AppNavigation.SelectedItem = item;
        item.Focus(FocusState.Keyboard);
    }
}
