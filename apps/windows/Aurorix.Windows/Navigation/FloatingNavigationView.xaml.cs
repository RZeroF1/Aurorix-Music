using System;
using System.Collections.Generic;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;

namespace Aurorix.Windows.Navigation;

public sealed partial class FloatingNavigationView : UserControl
{
    private const double CompactLength = 48;
    private const double OpenLength = 320;

    public FloatingNavigationView()
    {
        InitializeComponent();
        ApplyLayout();
    }

    public static readonly DependencyProperty PaneCornerRadiusProperty =
        DependencyProperty.Register(
            nameof(PaneCornerRadius), typeof(CornerRadius), typeof(FloatingNavigationView),
            new PropertyMetadata(new CornerRadius(8), OnLayoutChanged));

    public static readonly DependencyProperty ExpansionModeProperty =
        DependencyProperty.Register(
            nameof(ExpansionMode), typeof(FloatingNavigationExpansionMode), typeof(FloatingNavigationView),
            new PropertyMetadata(FloatingNavigationExpansionMode.LeftToRight, OnLayoutChanged));

    public CornerRadius PaneCornerRadius
    {
        get => (CornerRadius)GetValue(PaneCornerRadiusProperty);
        set => SetValue(PaneCornerRadiusProperty, value);
    }

    public FloatingNavigationExpansionMode ExpansionMode
    {
        get => (FloatingNavigationExpansionMode)GetValue(ExpansionModeProperty);
        set => SetValue(ExpansionModeProperty, value);
    }

    public NavigationView NativeView => NativeNavigationView;
    public IList<object> MenuItems => NativeNavigationView.MenuItems;
    public IList<object> FooterMenuItems => NativeNavigationView.FooterMenuItems;

    public event TypedEventHandler<NavigationView, NavigationViewSelectionChangedEventArgs>? SelectionChanged;
    public event TypedEventHandler<NavigationView, NavigationViewItemInvokedEventArgs>? ItemInvoked;
    public event TypedEventHandler<NavigationView, object>? PaneOpening;
    public event TypedEventHandler<NavigationView, object>? PaneOpened;
    public event TypedEventHandler<NavigationView, NavigationViewPaneClosingEventArgs>? PaneClosing;
    public event TypedEventHandler<NavigationView, object>? PaneClosed;

    public void ApplyThemeResources(Brush expandedPaneBackground, Brush foreground)
    {
        NativeNavigationView.Foreground = foreground;
        NativeNavigationView.Resources["NavigationViewExpandedPaneBackground"] = expandedPaneBackground;
    }

    private static void OnLayoutChanged(DependencyObject sender, DependencyPropertyChangedEventArgs args)
    {
        if (sender is FloatingNavigationView view)
        {
            view.ApplyLayout();
        }
    }

    private void ApplyLayout()
    {
        if (NativeNavigationView is null)
        {
            return;
        }

        NativeNavigationView.CompactPaneLength = CompactLength;
        NativeNavigationView.OpenPaneLength = OpenLength;
        NativeNavigationView.CornerRadius = PaneCornerRadius;
        NavigationHost.Width = OpenLength;

        var alignment = ExpansionMode switch
        {
            FloatingNavigationExpansionMode.RightToLeft => HorizontalAlignment.Right,
            FloatingNavigationExpansionMode.CenterOut => HorizontalAlignment.Center,
            _ => HorizontalAlignment.Left,
        };
        HorizontalAlignment = alignment;
        NavigationHost.HorizontalAlignment = alignment;
        NativeNavigationView.HorizontalAlignment = alignment;
    }

    private void NativeNavigationView_Loaded(object sender, RoutedEventArgs args)
    {
        ApplyLayout();
        ApplyTemplateCornerRadius();
    }

    private void NativeNavigationView_PaneOpening(NavigationView sender, object args) =>
        PaneOpening?.Invoke(sender, args);

    private void NativeNavigationView_PaneOpened(NavigationView sender, object args)
    {
        ApplyTemplateCornerRadius();
        PaneOpened?.Invoke(sender, args);
    }

    private void NativeNavigationView_PaneClosing(NavigationView sender, NavigationViewPaneClosingEventArgs args) =>
        PaneClosing?.Invoke(sender, args);

    private void NativeNavigationView_PaneClosed(NavigationView sender, object args)
    {
        ApplyTemplateCornerRadius();
        PaneClosed?.Invoke(sender, args);
    }

    private void NativeNavigationView_SelectionChanged(
        NavigationView sender, NavigationViewSelectionChangedEventArgs args) =>
        SelectionChanged?.Invoke(sender, args);

    private void NativeNavigationView_ItemInvoked(
        NavigationView sender, NavigationViewItemInvokedEventArgs args) =>
        ItemInvoked?.Invoke(sender, args);

    private void ApplyTemplateCornerRadius()
    {
        NativeNavigationView.ApplyTemplate();
        if (FindVisualChild<SplitView>(NativeNavigationView, "RootSplitView") is { } splitView)
        {
            splitView.CornerRadius = PaneCornerRadius;
        }
    }

    private static T? FindVisualChild<T>(DependencyObject root, string name)
        where T : FrameworkElement
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(root); index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is T match && match.Name == name)
            {
                return match;
            }

            if (FindVisualChild<T>(child, name) is { } descendant)
            {
                return descendant;
            }
        }

        return null;
    }
}

public enum FloatingNavigationExpansionMode
{
    LeftToRight,
    RightToLeft,
    CenterOut,
}
